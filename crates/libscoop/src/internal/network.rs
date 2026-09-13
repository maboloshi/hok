//! HTTP networking using `reqwest` (async, rustls).
//!
//! Replaced `ureq` (blocking) so the [`Network`] trait can be async and
//! deliver real non-blocking I/O on a tokio runtime (phase 4, 方案 B).
//!
//! # Architecture
//!
//! All configurable options are bundled into [`RequestOptions`]; the core
//! functions [`head`] and [`download`] accept it by reference.  Simple callers
//! that only need a proxy and timeout can use [`RequestOptions::default()`].
//!
//! Internal helpers `build_client` and `apply_headers` are `pub(crate)` so
//! that `package/*` can reuse them without duplicating logic.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::future::Future;
use std::io::Write;
use std::path::Path;
use std::pin::Pin;
use std::time::Duration;
use tracing::warn;

use futures::StreamExt;
use reqwest::header::{
    ACCEPT, AUTHORIZATION, CONTENT_LENGTH, CONTENT_RANGE, COOKIE, RANGE, REFERER, USER_AGENT,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Result of a HEAD request, with status code and optional error message.
#[derive(Debug, Clone)]
pub struct HeadResult {
    pub url: String,
    pub status_code: u16,
    pub error: Option<String>,
    /// `Content-Length` reported by the server, when present.
    pub content_length: Option<u64>,
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// All configurable parameters for an HTTP request.
///
/// Every field is optional — use [`Default::default()`] or struct literal
/// syntax to construct a value with only the fields you need.
#[derive(Clone, Default)]
pub struct RequestOptions<'a> {
    pub proxy: Option<&'a str>,
    pub timeout_secs: u64,
    pub user_agent: Option<&'a str>,
    pub referer: Option<&'a str>,
    pub cookies: Option<&'a HashMap<String, String>>,
    pub token: Option<&'a str>,
    pub extra_headers: Option<&'a HashMap<String, String>>,
    /// Attach upstream's GitHub API *download* headers — `Accept:
    /// application/octet-stream` and `X-GitHub-Api-Version: 2022-11-28`
    /// (`lib/download.ps1:97-101`). Paired with [`Self::token`].
    pub github_api: bool,
    /// Resume a partial single-stream download from this byte offset via a
    /// `Range: bytes={range_start}-` request (0 = fresh download). Used by the
    /// single-stream path so an interrupted `.download` resumes next run.
    pub range_start: u64,
}

// ---------------------------------------------------------------------------
// Header profiles (upstream's per-call-site header contract)
// ---------------------------------------------------------------------------

/// Which upstream call site's header rules a request follows.
///
/// Upstream assembles request headers in four places, each with slightly
/// different rules (Referer suppression, PRIVATE_HOSTS, GitHub API auth). This
/// enum is the single description of those differences, so every hok request
/// path can be checked against its upstream counterpart by name instead of by
/// re-reading each call site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderProfile {
    /// `Invoke-Download` (`lib/download.ps1:88-111`) — the file downloader
    /// behind install / update / download / reinstall / checkhashes.
    Download,
    /// `checkurls`' `test_dl` (`bin/checkurls.ps1:56-70`).
    CheckUrls,
    /// `checkver`'s page fetch (`bin/checkver.ps1:118-246`).
    Checkver,
    /// `autoupdate`'s hash fetchers (`lib/autoupdate.ps1:20-206`), including
    /// `find_hash_in_headers`.
    Autoupdate,
}

/// Hosts whose downloads omit `Referer` (`lib/download.ps1:94-96`). Matched as
/// substrings against the whole URL, exactly like upstream's `-match`.
const NO_REFERER_HOSTS: [&str; 2] = ["sourceforge.net", "portableapps.com"];

/// Host fragment that marks a GitHub REST API URL.
const GITHUB_API_HOST: &str = "api.github.com";

impl HeaderProfile {
    /// `Referer` value for `url`, or `None` when upstream omits the header.
    pub fn referer(self, url: &str) -> Option<String> {
        if self == Self::Download && NO_REFERER_HOSTS.iter().any(|host| url.contains(host)) {
            return None;
        }
        Some(strip_filename(url))
    }

    /// Does upstream attach the GitHub API *download* headers to `url`?
    /// `lib/download.ps1:97` matches `api\.github\.com/repos`.
    pub fn uses_github_download_headers(self, url: &str) -> bool {
        self == Self::Download && url.contains("api.github.com/repos")
    }

    /// Does upstream attach the configured GitHub token to `url`?
    ///
    /// `lib/download.ps1:97` matches `api\.github\.com/repos`;
    /// `bin/checkver.ps1:219` matches any `api.github.com` path. Upstream never
    /// sends the token anywhere else, so hok must not either.
    pub fn uses_github_token(self, url: &str) -> bool {
        match self {
            Self::Download => url.contains("api.github.com/repos"),
            Self::Checkver => url.contains(GITHUB_API_HOST),
            Self::CheckUrls | Self::Autoupdate => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers (pub(crate) for download.rs reuse)
// ---------------------------------------------------------------------------

/// Build a [`reqwest::Client`] from the proxy and timeout in `opts`.
pub(crate) fn build_client<'a>(opts: &RequestOptions<'a>) -> Result<reqwest::Client, String> {
    build_client_with_redirects(opts, true)
}

/// [`build_client`] with the redirect policy made explicit: `follow = false`
/// returns the 3xx response itself instead of chasing the `Location` header.
fn build_client_with_redirects<'a>(
    opts: &RequestOptions<'a>,
    follow: bool,
) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(opts.timeout_secs))
        .danger_accept_invalid_certs(false);
    if let Some(proxy_url) = opts.proxy {
        let p = reqwest::Proxy::all(proxy_url).map_err(|e| e.to_string())?;
        builder = builder.proxy(p);
    }
    if !follow {
        builder = builder.redirect(reqwest::redirect::Policy::none());
    }
    builder.build().map_err(|e| e.to_string())
}

/// Inject common HTTP headers into a request based on the given options.
pub(crate) fn apply_headers<'a>(
    mut req: reqwest::RequestBuilder,
    opts: &RequestOptions<'a>,
) -> reqwest::RequestBuilder {
    if let Some(ua) = opts.user_agent {
        req = req.header(USER_AGENT, ua);
    }
    if let Some(r) = opts.referer {
        req = req.header(REFERER, r);
    }
    if let Some(cookies) = opts.cookies {
        let cookie_str: Vec<String> = cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
        if !cookie_str.is_empty() {
            req = req.header(COOKIE, cookie_str.join("; "));
        }
    }
    if let Some(token) = opts.token {
        req = req.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    if opts.github_api {
        req = req
            .header(ACCEPT, "application/octet-stream")
            .header("X-GitHub-Api-Version", "2022-11-28");
    }
    if let Some(extra) = opts.extra_headers {
        for (k, v) in extra {
            req = req.header(k.as_str(), v.as_str());
        }
    }
    req
}

/// Match PRIVATE_HOSTS entries (`(match_pattern, headers_string)` pairs)
/// against a URL and return the merged headers. Returns `None` when nothing
/// matched.
///
/// `headers_string` is the raw newline-separated `key=value` text from the
/// config; it is parsed only for entries whose pattern matches. Matching is
/// case-insensitive, mirroring PowerShell's `-match` operator used by upstream
/// Scoop. Callers fold the result into [`RequestOptions::extra_headers`]; the
/// matching itself lives here so all call sites share one implementation.
pub(crate) fn match_private_hosts<'a>(
    hosts: impl IntoIterator<Item = (&'a str, &'a str)>,
    url: &str,
) -> Option<HashMap<String, String>> {
    let matched: HashMap<String, String> = hosts
        .into_iter()
        .filter(|(pattern, _)| {
            regex::RegexBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .inspect_err(|e| warn!("invalid regex pattern '{pattern}': {e}"))
                .ok()
                .is_some_and(|re| re.is_match(url))
        })
        .flat_map(|(_, headers)| parse_headers(headers))
        .collect();
    if matched.is_empty() {
        None
    } else {
        Some(matched)
    }
}

/// Parse a newline-separated `key=value` header string (same format as the
/// `PRIVATE_HOSTS` config value, mirroring Scoop's `ConvertFrom-StringData`).
fn parse_headers(headers: &str) -> impl Iterator<Item = (String, String)> + '_ {
    headers.lines().filter_map(|line| {
        let line = line.trim();
        let (key, val) = line.split_once('=')?;
        let key = key.trim();
        if key.is_empty() {
            None
        } else {
            Some((key.to_string(), val.trim().to_string()))
        }
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Perform an HTTP HEAD request with full option control.
///
/// Returns a [`HeadResult`] that always succeeds at the `Result` level —
/// network errors are captured inside the struct's `error` field.
pub async fn head(url: &str, opts: &RequestOptions<'_>) -> HeadResult {
    let client = match build_client(opts) {
        Ok(c) => c,
        Err(e) => {
            return HeadResult {
                url: url.to_string(),
                status_code: 0,
                error: Some(format!("proxy error: {e}")),
                content_length: None,
            };
        }
    };
    let req = apply_headers(client.head(url), opts);

    match req.send().await {
        Ok(resp) => {
            let code = resp.status().as_u16();
            let content_length = resp
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            HeadResult {
                url: url.to_string(),
                status_code: code,
                error: if (200..400).contains(&code) {
                    None
                } else {
                    Some(format!("HTTP {code}"))
                },
                content_length,
            }
        }
        Err(e) => HeadResult {
            url: url.to_string(),
            status_code: 0,
            error: Some(e.to_string()),
            content_length: None,
        },
    }
}

/// Perform a HEAD request **without** following redirects, returning the
/// response so the caller can inspect the 3xx status and its headers.
///
/// Used by the metalink hash probe, which mirrors Scoop's
/// `AllowAutoRedirect = $false` + `Digest` header read in
/// `find_hash_in_headers` (lib/autoupdate.ps1:181-206).
pub(crate) async fn head_no_redirect(
    url: &str,
    opts: &RequestOptions<'_>,
) -> Result<reqwest::Response, String> {
    let client = build_client_with_redirects(opts, false)?;
    apply_headers(client.head(url), opts)
        .send()
        .await
        .map_err(|e| e.to_string())
}

/// Download a URL's content as bytes via HTTP GET with full option control.
pub async fn download(url: &str, opts: &RequestOptions<'_>) -> Result<Vec<u8>, String> {
    let client = build_client(opts)?;
    let req = apply_headers(client.get(url), opts);

    let resp = req
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let resp = resp.error_for_status().map_err(|e| e.to_string())?;
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("read failed: {e}"))
}

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------

/// Strip filename from a URL to produce a Referer value.
/// Matches Scoop's `strip_filename` in lib/core.ps1.
pub fn strip_filename(url: &str) -> String {
    // Strip query/fragment first
    let without_query = url.split('?').next().unwrap_or(url);
    let without_fragment = without_query.split('#').next().unwrap_or(without_query);
    // Remove the last path segment
    let trimmed = without_fragment.trim_end_matches('/');
    if let Some(pos) = trimmed.rfind('/') {
        trimmed[..=pos].to_string()
    } else {
        format!("{}/", without_fragment)
    }
}

// ---------------------------------------------------------------------------
// Infra trait (network capability)
// ---------------------------------------------------------------------------

/// Result of a single HTTP range request, with status and optional
/// `Content-Range` start offset. Used by the fragmented downloader.
#[derive(Debug, Clone)]
pub struct RangeResponse {
    pub status: u16,
    pub content_range_start: Option<u64>,
    pub bytes: Vec<u8>,
}

/// Error classifying whether a range request is retryable.
#[derive(Debug)]
pub enum RangeError {
    /// Retrying cannot help (HTTP 4xx except 408/429, server ignoring
    /// `Range`, bad response shape).
    Final(String),
    /// Transport-level, 5xx, 408/429 — safe to retry with backoff.
    Transient(String),
}

impl std::fmt::Display for RangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RangeError::Final(msg) | RangeError::Transient(msg) => write!(f, "{}", msg),
        }
    }
}

/// An async download-progress callback: invoked with cumulative bytes
/// written, returning a boxed future the caller awaits. Boxed (rather than
/// `AsyncFnMut`, which is not dyn-compatible) so the [`Network`] trait can
/// hold it as a trait object; returning a future lets the caller
/// `send().await` on the event bus for backpressure (no dropped events).
pub(crate) type ProgressCallback<'a> =
    dyn FnMut(u64) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> + Send;

/// The HTTP networking capability of a session.
///
/// The default implementation ([`RealNetwork`]) wraps the free functions in
/// this module verbatim; tests inject an in-memory fake via
/// [`Session::set_network`] to avoid the real network. The trait lives here
/// (the tool layer) so it has no dependency on any domain service.
#[async_trait::async_trait]
pub trait Network: Send + Sync {
    /// Perform an HTTP HEAD request.
    async fn head(&self, url: &str, opts: &RequestOptions) -> HeadResult;

    /// Download a URL's content as bytes via HTTP GET.
    async fn download(&self, url: &str, opts: &RequestOptions) -> Result<Vec<u8>, String>;

    /// Perform one range request (`bytes={start}-{end}`), returning the
    /// response status, `Content-Range` start offset and body. The caller
    /// (fragmented downloader) owns retry/resume/validation logic.
    async fn range_get(
        &self,
        url: &str,
        start: u64,
        end: u64,
        opts: &RequestOptions,
    ) -> Result<RangeResponse, RangeError>;

    /// Download a URL to `dest` in a streaming loop, reporting cumulative
    /// bytes written through `on_progress` (an async callback so callers can
    /// `send().await` on the event bus for backpressure — no dropped events).
    async fn download_stream(
        &self,
        url: &str,
        opts: &RequestOptions,
        dest: &Path,
        on_progress: &mut ProgressCallback<'_>,
    ) -> Result<(), String>;
}

/// Default network implementation wrapping the free functions above.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealNetwork;

#[async_trait::async_trait]
impl Network for RealNetwork {
    async fn head(&self, url: &str, opts: &RequestOptions) -> HeadResult {
        head(url, opts).await
    }

    async fn download(&self, url: &str, opts: &RequestOptions) -> Result<Vec<u8>, String> {
        download(url, opts).await
    }

    async fn range_get(
        &self,
        url: &str,
        start: u64,
        end: u64,
        opts: &RequestOptions,
    ) -> Result<RangeResponse, RangeError> {
        let client = build_client(opts).map_err(RangeError::Transient)?;
        let req = apply_headers(
            client
                .get(url)
                .header(RANGE, format!("bytes={start}-{end}")),
            opts,
        );

        let resp = req
            .send()
            .await
            .map_err(|e| RangeError::Transient(format!("request failed: {e}")))?;
        let status = resp.status().as_u16();
        if status == 206 {
            let content_range_start = resp
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(|cr| cr.strip_prefix("bytes ").and_then(|r| r.split('-').next()))
                .and_then(|s| s.parse::<u64>().ok());
            let bytes = resp
                .bytes()
                .await
                .map(|b| b.to_vec())
                .map_err(|e| RangeError::Transient(format!("read failed: {e}")))?;
            Ok(RangeResponse {
                status,
                content_range_start,
                bytes,
            })
        } else if (400..500).contains(&status) && status != 408 && status != 429 {
            // 4xx (except retryable 408/429) — the server rejected the
            // request, retrying won't help.
            Err(RangeError::Final(format!("HTTP {status} for {url}")))
        } else if status >= 500 || status == 408 || status == 429 {
            // 5xx, 408/429 — transient, safe to retry with backoff.
            Err(RangeError::Transient(format!("HTTP {status}")))
        } else {
            // 2xx/3xx that is not 206 (e.g. the server ignored `Range`).
            Err(RangeError::Final(format!(
                "expected 206 Partial Content, got {status} (server ignored Range request)"
            )))
        }
    }

    async fn download_stream(
        &self,
        url: &str,
        opts: &RequestOptions,
        dest: &Path,
        on_progress: &mut ProgressCallback<'_>,
    ) -> Result<(), String> {
        let client = build_client(opts)?;
        let mut req = apply_headers(client.get(url), opts);
        // Resume from `range_start` (byte 0 = fresh). The Range infra mirrors
        // the fragmented `range_get` path.
        if opts.range_start > 0 {
            req = req.header(RANGE, format!("bytes={}-", opts.range_start));
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("download failed: {e}"))?;
        let resp = resp.error_for_status().map_err(|e| e.to_string())?;

        // Resume only when the server honored the Range request (206). If it
        // ignored it and returned the whole body (200), rewrite from byte 0 —
        // appending a full body to the partial file would corrupt it.
        let resume = opts.range_start > 0 && resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        let mut file = if resume {
            OpenOptions::new().create(true).append(true).open(dest)
        } else {
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(dest)
        }
        .map_err(|e| e.to_string())?;

        let mut stream = resp.bytes_stream();
        // Absolute progress: start from the resumed offset so the caller's
        // progress bar reflects the real total, not just the transferred tail.
        let mut dlnow = if resume { opts.range_start } else { 0 };
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("read failed: {e}"))?;
            file.write_all(&chunk)
                .map_err(|e| format!("write failed: {e}"))?;
            dlnow += chunk.len() as u64;
            on_progress(dlnow).await;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn entries(pairs: &[(&str, &[(&str, &str)])]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(pattern, headers)| {
                let text = headers
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                (pattern.to_string(), text)
            })
            .collect()
    }

    fn match_url(pairs: &[(&str, &[(&str, &str)])], url: &str) -> Option<HashMap<String, String>> {
        let entries = entries(pairs);
        match_private_hosts(entries.iter().map(|(p, h)| (p.as_str(), h.as_str())), url)
    }

    /// End-to-end HTTPS smoke check for the configured TLS backend.
    ///
    /// Ignored by default: it needs outbound network access. Run it manually
    /// after touching the `reqwest` TLS features:
    /// `cargo test -p libscoop --lib tls_https_smoke -- --ignored`
    #[tokio::test]
    #[ignore = "requires network access (manual TLS backend smoke check)"]
    async fn tls_https_smoke() {
        let opts = RequestOptions {
            timeout_secs: 30,
            // api.github.com answers 403 to requests without a User-Agent.
            user_agent: Some("hok-tls-smoke/0.1"),
            ..Default::default()
        };
        let body = download("https://api.github.com/", &opts)
            .await
            .expect("HTTPS request must succeed with the configured TLS backend");
        assert!(
            body.len() > 16,
            "unexpected body length: {}",
            body.len()
        );
    }

    #[test]
    fn matching_is_case_insensitive_like_powershell_match() {
        let h = match_url(
            &[("^https://API\\.EXAMPLE\\.com", &[("X-Token", "abc")])],
            "https://api.example.com/resource",
        )
        .unwrap();
        assert_eq!(h.get("X-Token").map(|s| s.as_str()), Some("abc"));
    }

    #[test]
    fn no_match_returns_none() {
        assert_eq!(
            match_url(
                &[("example\\.com", &[("X-Token", "abc")])],
                "https://other.org/x"
            ),
            None
        );
    }

    #[test]
    fn empty_hosts_returns_none() {
        assert_eq!(
            match_private_hosts(std::iter::empty(), "https://a.example.com/"),
            None
        );
    }

    #[test]
    fn matched_headers_are_merged() {
        let h = match_url(
            &[
                ("a\\.example\\.com", &[("X-One", "1")]),
                ("b\\.example\\.com", &[("X-Two", "2")]),
            ],
            "https://B.Example.COM/x",
        )
        .unwrap();
        assert_eq!(h.get("X-Two").map(|s| s.as_str()), Some("2"));
        assert_eq!(h.get("X-One"), None);
    }

    // ── HeaderProfile (upstream's per-call-site header contract) ────────────

    #[test]
    fn download_referer_drops_the_filename() {
        assert_eq!(
            HeaderProfile::Download
                .referer("https://example.com/a/b/file.zip")
                .as_deref(),
            Some("https://example.com/a/b/")
        );
    }

    #[test]
    fn download_referer_is_suppressed_for_sourceforge_and_portableapps() {
        // `lib/download.ps1:94`: SourceForge and PortableApps reject a foreign
        // Referer. Upstream matches the whole URL with `-match`.
        assert_eq!(
            HeaderProfile::Download.referer("https://downloads.sourceforge.net/project/x/y.exe"),
            None
        );
        assert_eq!(
            HeaderProfile::Download.referer("https://portableapps.com/x.exe"),
            None
        );
        // Anywhere in the URL, mirroring upstream's substring match.
        assert_eq!(
            HeaderProfile::Download.referer("https://mirror.test/x.exe?src=sourceforge.net"),
            None
        );
    }

    #[test]
    fn other_profiles_always_send_the_referer() {
        // `bin/checkurls.ps1:60` and `lib/autoupdate.ps1:186` have no host
        // exception; `bin/checkver.ps1:246` neither.
        for profile in [
            HeaderProfile::CheckUrls,
            HeaderProfile::Checkver,
            HeaderProfile::Autoupdate,
        ] {
            assert_eq!(
                profile.referer("https://downloads.sourceforge.net/project/x/y.exe"),
                Some("https://downloads.sourceforge.net/project/x/".to_string()),
                "{profile:?} must keep the Referer"
            );
        }
    }

    #[test]
    fn github_download_headers_only_on_the_repos_path() {
        // `lib/download.ps1:97` matches `api\.github\.com/repos`.
        assert!(
            HeaderProfile::Download
                .uses_github_download_headers("https://api.github.com/repos/o/r/releases")
        );
        assert!(!HeaderProfile::Download.uses_github_download_headers("https://api.github.com/"));
        assert!(
            !HeaderProfile::Checkver
                .uses_github_download_headers("https://api.github.com/repos/o/r/releases")
        );
    }

    #[test]
    fn github_token_never_leaks_to_other_hosts() {
        // `lib/autoupdate.ps1:319` and `bin/checkurls.ps1` never authorize.
        assert!(!HeaderProfile::Autoupdate.uses_github_token("https://api.github.com/repos/o/r"));
        assert!(!HeaderProfile::CheckUrls.uses_github_token("https://api.github.com/repos/o/r"));
        // `lib/download.ps1:97` — the repos path only.
        assert!(HeaderProfile::Download.uses_github_token("https://api.github.com/repos/o/r"));
        assert!(!HeaderProfile::Download.uses_github_token("https://api.github.com/users/o"));
        // `bin/checkver.ps1:219` — any api.github.com path.
        assert!(HeaderProfile::Checkver.uses_github_token("https://api.github.com/users/o"));
        assert!(!HeaderProfile::Checkver.uses_github_token("https://example.com/x"));
    }

    #[test]
    fn github_api_download_headers_are_attached() {
        // `lib/download.ps1:97-101` — the Accept / API-version / bearer triple.
        let client = reqwest::Client::new();
        let opts = RequestOptions {
            user_agent: Some("ua"),
            token: Some("tok"),
            github_api: true,
            ..Default::default()
        };
        let req = apply_headers(client.get("https://api.github.com/repos/o/r"), &opts)
            .build()
            .expect("request builds");

        assert_eq!(req.headers()[ACCEPT], "application/octet-stream");
        assert_eq!(req.headers()["x-github-api-version"], "2022-11-28");
        assert_eq!(req.headers()[AUTHORIZATION], "Bearer tok");
    }

    #[test]
    fn github_api_headers_stay_off_by_default() {
        let client = reqwest::Client::new();
        let opts = RequestOptions {
            token: Some("tok"),
            ..Default::default()
        };
        let req = apply_headers(client.get("https://example.com/x"), &opts)
            .build()
            .expect("request builds");

        assert!(req.headers().get(ACCEPT).is_none());
        assert!(req.headers().get("x-github-api-version").is_none());
    }
}
