//! HTTP networking using `ureq` (pure Rust).
//!
//! Replaced `curl` (libcurl bindings, static C build) to avoid C compilation
//! overhead and align with the project's "pure Rust first" policy.

use std::io::Read;
use std::time::Duration;

/// Get the content length of a URL via HTTP HEAD.
pub fn get_content_length(url: &str, proxy: Option<&str>) -> Option<f64> {
    let agent = agent(proxy, 30).ok()?;
    let resp = agent.head(url).call().ok()?;
    resp.headers()
        .get("Content-Length")?
        .to_str()
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
}

/// Check if a URL returns a successful HTTP status (2xx or 3xx).
pub fn head_url(url: &str, proxy: Option<&str>, timeout_secs: u64) -> Result<bool, String> {
    let agent = agent(proxy, timeout_secs)?;
    let resp = agent.head(url).call().map_err(|e| e.to_string())?;
    let code = resp.status().as_u16();
    Ok((200..400).contains(&code))
}

/// Download a URL's content as bytes via HTTP GET.
pub fn download_file(url: &str, proxy: Option<&str>) -> Result<Vec<u8>, String> {
    let agent = agent(proxy, 120)?;
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
