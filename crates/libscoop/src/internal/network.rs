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
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use tracing::warn;

use futures::StreamExt;
use reqwest::header::{
    AUTHORIZATION, CONTENT_LENGTH, CONTENT_RANGE, COOKIE, RANGE, REFERER, USER_AGENT,
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
    /// Resume a partial single-stream download from this byte offset via a
    /// `Range: bytes={range_start}-` request (0 = fresh download). Used by the
    /// single-stream path so an interrupted `.download` resumes next run.
    pub range_start: u64,
}

// ---------------------------------------------------------------------------
// Internal helpers (pub(crate) for download.rs reuse)
// ---------------------------------------------------------------------------

/// Build a [`reqwest::Client`] from the proxy and timeout in `opts`.
pub(crate) fn build_client<'a>(opts: &RequestOptions<'a>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(opts.timeout_secs))
        .danger_accept_invalid_certs(false);
    if let Some(proxy_url) = opts.proxy {
        let p = reqwest::Proxy::all(proxy_url).map_err(|e| e.to_string())?;
        builder = builder.proxy(p);
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
    /// bytes written through `on_progress`.
    async fn download_stream(
        &self,
        url: &str,
        opts: &RequestOptions,
        dest: &Path,
        on_progress: &mut (dyn FnMut(u64) + Send),
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
        on_progress: &mut (dyn FnMut(u64) + Send),
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
            on_progress(dlnow);
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
}
