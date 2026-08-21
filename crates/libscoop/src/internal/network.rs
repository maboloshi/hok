//! HTTP networking using `ureq` (pure Rust).
//!
//! Replaced `curl` (libcurl bindings, static C build) to avoid C compilation
//! overhead and align with the project's "pure Rust first" policy.
//!
//! # Architecture
//!
//! All configurable options are bundled into [`RequestOptions`]; the two core
//! functions [`head`] and [`download`] accept it by reference.  Simple callers
//! that only need a proxy and timeout can use [`RequestOptions::default()`].
//!
//! Internal helpers `build_agent` and `apply_headers` are `pub(crate)` so
//! that `package/download.rs` can reuse them without duplicating logic.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;
use tracing::warn;

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
}

// ---------------------------------------------------------------------------
// Internal helpers (pub(crate) for download.rs reuse)
// ---------------------------------------------------------------------------

/// Build a [`ureq::Agent`] from the proxy and timeout in `opts`.
pub(crate) fn build_agent(opts: &RequestOptions) -> Result<ureq::Agent, String> {
    let mut cfg =
        ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(opts.timeout_secs)));
    if let Some(proxy_url) = opts.proxy {
        let p = ureq::Proxy::new(proxy_url).map_err(|e| e.to_string())?;
        cfg = cfg.proxy(Some(p));
    }
    Ok(cfg.build().new_agent())
}

/// Inject common HTTP headers into a request based on the given options.
pub(crate) fn apply_headers<B>(
    mut req: ureq::RequestBuilder<B>,
    opts: &RequestOptions,
) -> ureq::RequestBuilder<B> {
    if let Some(ua) = opts.user_agent {
        req = req.header("User-Agent", ua);
    }
    if let Some(r) = opts.referer {
        req = req.header("Referer", r);
    }
    if let Some(cookies) = opts.cookies {
        let cookie_str: Vec<String> = cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
        if !cookie_str.is_empty() {
            req = req.header("Cookie", cookie_str.join("; "));
        }
    }
    if let Some(token) = opts.token {
        req = req.header("Authorization", &format!("Bearer {token}"));
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
pub fn head(url: &str, opts: &RequestOptions) -> HeadResult {
    let agent = match build_agent(opts) {
        Ok(a) => a,
        Err(e) => {
            return HeadResult {
                url: url.to_string(),
                status_code: 0,
                error: Some(format!("proxy error: {e}")),
                content_length: None,
            };
        }
    };
    let req = apply_headers(agent.head(url), opts);

    match req.call() {
        Ok(resp) => {
            let code = resp.status().as_u16();
            let content_length = resp
                .headers()
                .get("Content-Length")
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
pub fn download(url: &str, opts: &RequestOptions) -> Result<Vec<u8>, String> {
    let agent = build_agent(opts)?;
    let req = apply_headers(agent.get(url), opts);

    let mut body = Vec::new();
    req.call()
        .map_err(|e| format!("request failed: {e}"))?
        .into_body()
        .into_reader()
        .read_to_end(&mut body)
        .map_err(|e| format!("read failed: {e}"))?;
    Ok(body)
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
pub trait Network {
    /// Perform an HTTP HEAD request.
    fn head(&self, url: &str, opts: &RequestOptions) -> HeadResult;

    /// Download a URL's content as bytes via HTTP GET.
    fn download(&self, url: &str, opts: &RequestOptions) -> Result<Vec<u8>, String>;

    /// Perform one range request (`bytes={start}-{end}`), returning the
    /// response status, `Content-Range` start offset and body. The caller
    /// (fragmented downloader) owns retry/resume/validation logic.
    fn range_get(
        &self,
        url: &str,
        start: u64,
        end: u64,
        opts: &RequestOptions,
    ) -> Result<RangeResponse, RangeError>;

    /// Download a URL to `dest` in a streaming loop, reporting cumulative
    /// bytes written through `on_progress`.
    fn download_stream(
        &self,
        url: &str,
        opts: &RequestOptions,
        dest: &Path,
        on_progress: &mut dyn FnMut(u64),
    ) -> Result<(), String>;
}

/// Default network implementation wrapping the free functions above.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealNetwork;

impl Network for RealNetwork {
    fn head(&self, url: &str, opts: &RequestOptions) -> HeadResult {
        head(url, opts)
    }

    fn download(&self, url: &str, opts: &RequestOptions) -> Result<Vec<u8>, String> {
        download(url, opts)
    }

    fn range_get(
        &self,
        url: &str,
        start: u64,
        end: u64,
        opts: &RequestOptions,
    ) -> Result<RangeResponse, RangeError> {
        let agent = build_agent(opts).map_err(RangeError::Transient)?;
        let mut req = agent
            .get(url)
            .header("Range", &format!("bytes={start}-{end}"));
        if let Some(cookies) = opts.cookies {
            if !cookies.is_empty() {
                let cookie_val = cookies
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                req = req.header("Cookie", &cookie_val);
            }
        }

        match req.call() {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if status != 206 {
                    return Err(RangeError::Final(format!(
                        "expected 206 Partial Content, got {status} (server ignored Range request)"
                    )));
                }
                let content_range_start = resp
                    .headers()
                    .get("Content-Range")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|cr| cr.strip_prefix("bytes ").and_then(|r| r.split('-').next()))
                    .and_then(|s| s.parse::<u64>().ok());
                let mut body = Vec::new();
                resp.into_body()
                    .into_reader()
                    .read_to_end(&mut body)
                    .map_err(|e| RangeError::Transient(format!("read failed: {e}")))?;
                Ok(RangeResponse {
                    status,
                    content_range_start,
                    bytes: body,
                })
            }
            // 4xx (except retryable 408/429) — the server rejected the
            // request, retrying won't help.
            Err(ureq::Error::StatusCode(code))
                if (400..500).contains(&code) && code != 408 && code != 429 =>
            {
                Err(RangeError::Final(format!("HTTP {code} for {url}")))
            }
            // Everything else — 5xx, 408/429, Io, Timeout, ... — transient.
            Err(ureq::Error::StatusCode(code)) => {
                Err(RangeError::Transient(format!("HTTP {code}")))
            }
            Err(e) => Err(RangeError::Transient(e.to_string())),
        }
    }

    fn download_stream(
        &self,
        url: &str,
        opts: &RequestOptions,
        dest: &Path,
        on_progress: &mut dyn FnMut(u64),
    ) -> Result<(), String> {
        let agent = build_agent(opts)?;
        let mut req = agent.get(url);
        if let Some(cookies) = opts.cookies {
            if !cookies.is_empty() {
                let cookie_val = cookies
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                req = req.header("Cookie", &cookie_val);
            }
        }

        let resp = req.call().map_err(|e| format!("download failed: {e}"))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dest)
            .map_err(|e| e.to_string())?;
        let mut reader = resp.into_body().into_reader();
        let mut buf = [0u8; 32768];
        let mut dlnow = 0u64;
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| format!("read failed: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])
                .map_err(|e| format!("write failed: {e}"))?;
            dlnow += n as u64;
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