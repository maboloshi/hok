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
use std::io::Read;
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
            };
        }
    };
    let req = apply_headers(agent.head(url), opts);

    match req.call() {
        Ok(resp) => {
            let code = resp.status().as_u16();
            HeadResult {
                url: url.to_string(),
                status_code: code,
                error: if (200..400).contains(&code) {
                    None
                } else {
                    Some(format!("HTTP {code}"))
                },
            }
        }
        Err(e) => HeadResult {
            url: url.to_string(),
            status_code: 0,
            error: Some(e.to_string()),
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

    fn match_url(
        pairs: &[(&str, &[(&str, &str)])],
        url: &str,
    ) -> Option<HashMap<String, String>> {
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
            match_url(&[("example\\.com", &[("X-Token", "abc")])], "https://other.org/x"),
            None
        );
    }

    #[test]
    fn empty_hosts_returns_none() {
        assert_eq!(match_private_hosts(std::iter::empty(), "https://a.example.com/"), None);
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
