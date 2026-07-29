//! HTTP networking using `ureq` (pure Rust).
//!
//! Replaced `curl` (libcurl bindings, static C build) to avoid C compilation
//! overhead and align with the project's "pure Rust first" policy.

use std::collections::HashMap;
use std::io::Read;
use std::time::Duration;

/// Result of a HEAD request, with status code and optional error message.
#[derive(Debug, Clone)]
pub struct HeadResult {
    pub url: String,
    pub status_code: u16,
    pub error: Option<String>,
}

/// Check if a URL returns a successful HTTP status (2xx or 3xx).
pub fn head_url(url: &str, proxy: Option<&str>, timeout_secs: u64) -> Result<bool, String> {
    let agent = agent(proxy, timeout_secs)?;
    let resp = agent.head(url).call().map_err(|e| e.to_string())?;
    let code = resp.status().as_u16();
    Ok((200..400).contains(&code))
}

/// HEAD request with full control: custom headers, returns detailed result.
///
/// Supports:
/// - Custom User-Agent
/// - Referer header (strip_filename equivalent)
/// - Cookie header
/// - Additional custom headers (for PRIVATE_HOSTS support)
pub fn head_url_ext(
    url: &str,
    proxy: Option<&str>,
    timeout_secs: u64,
    user_agent: Option<&str>,
    referer: Option<&str>,
    cookies: Option<&HashMap<String, String>>,
    extra_headers: Option<&HashMap<String, String>>,
) -> HeadResult {
    let mut cfg = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_secs)));
    if let Some(proxy_url) = proxy {
        let p = match ureq::Proxy::new(proxy_url) {
            Ok(p) => p,
            Err(e) => return HeadResult {
                url: url.to_string(),
                status_code: 0,
                error: Some(format!("proxy error: {e}")),
            },
        };
        cfg = cfg.proxy(Some(p));
    }
    let agent = cfg.build().new_agent();

    let mut req = agent.head(url);

    // Custom User-Agent (Scoop compatibility)
    if let Some(ua) = user_agent {
        req = req.header("User-Agent", ua);
    }

    // Referer: strip_filename semantics (dirname of URL)
    if let Some(r) = referer {
        req = req.header("Referer", r);
    }

    // Cookie header from manifest
    if let Some(cookies_map) = cookies {
        let cookie_str: Vec<String> = cookies_map.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        if !cookie_str.is_empty() {
            req = req.header("Cookie", cookie_str.join("; "));
        }
    }

    // Extra headers (PRIVATE_HOSTS, etc.)
    if let Some(extra) = extra_headers {
        for (k, v) in extra {
            req = req.header(k.as_str(), v.as_str());
        }
    }

    match req.call() {
        Ok(resp) => {
            let code = resp.status().as_u16();
            HeadResult {
                url: url.to_string(),
                status_code: code,
                error: if (200..400).contains(&code) { None } else { Some(format!("HTTP {code}")) },
            }
        }
        Err(e) => {
            HeadResult {
                url: url.to_string(),
                status_code: 0,
                error: Some(e.to_string()),
            }
        }
    }
}

/// Download a URL's content as bytes via HTTP GET.
pub fn download_file(url: &str, proxy: Option<&str>, timeout_secs: u64) -> Result<Vec<u8>, String> {
    let agent = agent(proxy, timeout_secs)?;
    let resp = agent.get(url).call().map_err(|e| e.to_string())?;
    let mut body = Vec::new();
    resp.into_body()
        .into_reader()
        .read_to_end(&mut body)
        .map_err(|e| e.to_string())?;
    Ok(body)
}

fn agent(proxy: Option<&str>, timeout_secs: u64) -> Result<ureq::Agent, String> {
    let mut cfg = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_secs)));
    if let Some(proxy_url) = proxy {
        let p = ureq::Proxy::new(proxy_url).map_err(|e| e.to_string())?;
        cfg = cfg.proxy(Some(p));
    }
    Ok(cfg.build().new_agent())
}

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
