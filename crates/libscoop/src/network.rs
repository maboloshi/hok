//! Session-level HTTP operations.
//!
//! Thin wrappers over [`crate::internal::network`] that inject Scoop-compatible
//! headers (Referer via `strip_filename`, PRIVATE_HOSTS extra headers,
//! User-Agent from the session) and use the session's proxy configuration.
//!
//! # Design
//!
//! - [`head_url`] — HEAD request for URL validity checks (`checkurls`).
//! - [`download_file`] — download to a local path (`create`, `checkhashes`).
//! - [`download_page`] — download as a UTF-8 string, optionally with a Bearer
//!   token for GitHub API (`checkver`).

use std::path::Path;

use crate::{error::Fallible, internal, Error, Session};

/// Perform an HTTP HEAD request with Scoop-compatible headers.
///
/// Automatically injects:
/// - Proxy from session config
/// - Referer from URL (strip_filename)
/// - PRIVATE_HOSTS extra headers (host-regex matched)
/// - Default User-Agent
///
/// `cookies` is optional — pass `Some(&map)` for manifest-level cookies.
pub fn head_url(
    session: &Session,
    url: &str,
    timeout_secs: u64,
    cookies: Option<&std::collections::HashMap<String, String>>,
) -> internal::network::HeadResult {
    let config = session.config();
    let proxy = config.proxy();
    let referer = internal::network::strip_filename(url);

    // Build PRIVATE_HOSTS extra headers
    let extra_headers = config.private_hosts().and_then(|hosts| {
        internal::network::match_private_hosts(
            hosts.iter().map(|h| (h.match_pattern(), h.headers())),
            url,
        )
    });

    let opts = internal::network::RequestOptions {
        proxy,
        timeout_secs,
        user_agent: Some("Scoop/1.0 (+http://scoop.sh/)"),
        referer: Some(&referer),
        cookies,
        extra_headers: extra_headers.as_ref(),
        token: None,
    };
    internal::network::head(url, &opts)
}

/// Download a file via HTTP GET and save to a local path, using the session's proxy.
pub fn download_file(session: &Session, url: &str, dest: &Path) -> Fallible<()> {
    let config = session.config();
    let opts = internal::network::RequestOptions {
        proxy: config.proxy(),
        timeout_secs: 120,
        ..internal::network::RequestOptions::default()
    };
    let data = internal::network::download(url, &opts).map_err(|e| Error::Custom(e.to_string()))?;
    if let Some(parent) = dest.parent() {
        internal::fs::ensure_dir(parent)?;
    }
    std::fs::write(dest, &data)?;
    Ok(())
}

/// Download a URL's content as a UTF-8 string with Scoop-compatible headers.
///
/// Automatically injects:
/// - Proxy from session config
/// - Referer from URL (strip_filename)
/// - PRIVATE_HOSTS extra headers (host-regex matched)
/// - User-Agent from session or default
///
/// `token` is optional — pass `Some("ghp_...")` for GitHub API Bearer auth.
pub fn download_page(
    session: &Session,
    url: &str,
    timeout_secs: u64,
    token: Option<&str>,
) -> Fallible<String> {
    let config = session.config();
    let proxy = config.proxy();
    let referer = internal::network::strip_filename(url);

    // Build PRIVATE_HOSTS extra headers
    let extra_headers = config.private_hosts().and_then(|hosts| {
        internal::network::match_private_hosts(
            hosts.iter().map(|h| (h.match_pattern(), h.headers())),
            url,
        )
    });

    // User-Agent: use session's custom UA or default
    let user_agent = session
        .user_agent()
        .unwrap_or("Scoop/1.0 (+http://scoop.sh/)");

    let opts = internal::network::RequestOptions {
        proxy,
        timeout_secs,
        user_agent: Some(user_agent),
        referer: Some(&referer),
        cookies: None,
        extra_headers: extra_headers.as_ref(),
        token,
    };
    let data = internal::network::download(url, &opts).map_err(Error::Custom)?;
    String::from_utf8(data).map_err(|e| Error::Custom(format!("UTF-8 decode error: {}", e)))
}
