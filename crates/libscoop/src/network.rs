//! Session-level HTTP operations.
//!
//! Thin wrappers over [`crate::internal::network`], plus [`ScoopRequest`] — the
//! single place where hok resolves the headers upstream Scoop sends (proxy,
//! User-Agent, Referer, PRIVATE_HOSTS extra headers and GitHub API auth).
//!
//! # Design
//!
//! - [`ScoopRequest`] — resolves the header contract for one URL
//!   (see [`HeaderProfile`]).
//! - [`head_url`] — HEAD request for URL validity checks (`checkurls`,
//!   `info -v`).
//! - [`download_file`] — download to a local path (`create`, `checkhashes`,
//!   checkver's download-and-hash).
//! - [`download_page`] — download as a UTF-8 string, optionally with a Bearer
//!   token for GitHub API (`checkver`).

use std::collections::HashMap;
use std::path::Path;

use crate::constant::DEFAULT_USER_AGENT;
use crate::{Error, Session, error::Fallible, internal};

pub(crate) use internal::network::HeaderProfile;

/// Upstream's request header contract for one URL, resolved to owned values.
///
/// Every config-derived value is copied out here so the session's config read
/// guard is released before the request is awaited (a guard held across an
/// `.await` would block a concurrent `config_mut()`), and so concurrent callers
/// can share one resolution without touching [`Session`] again.
///
/// Upstream spreads these decisions over `lib/download.ps1:88-111`,
/// `bin/checkurls.ps1:56-70`, `bin/checkver.ps1:118-246` and
/// `lib/autoupdate.ps1:20-206`; [`HeaderProfile`] selects which of those rule
/// sets applies. PRIVATE_HOSTS extra headers are applied on every profile
/// (upstream omits them in the `autoupdate` fetchers only).
pub(crate) struct ScoopRequest {
    proxy: Option<String>,
    user_agent: String,
    referer: Option<String>,
    extra_headers: Option<HashMap<String, String>>,
    token: Option<String>,
    github_api: bool,
}

impl ScoopRequest {
    /// Resolve proxy, User-Agent, Referer, PRIVATE_HOSTS and GitHub API headers
    /// for `url`.
    pub(crate) fn for_url(session: &Session, url: &str, profile: HeaderProfile) -> Self {
        let (proxy, extra_headers, gh_token) = {
            let config = session.config();
            (
                config.proxy().map(|s| s.to_owned()),
                config.private_hosts().and_then(|hosts| {
                    internal::network::match_private_hosts(
                        hosts.iter().map(|h| (h.match_pattern(), h.headers())),
                        url,
                    )
                }),
                config.gh_token.clone(),
            )
        };
        Self {
            proxy,
            user_agent: session
                .user_agent()
                .unwrap_or(DEFAULT_USER_AGENT)
                .to_owned(),
            referer: profile.referer(url),
            extra_headers,
            token: if profile.uses_github_token(url) {
                gh_token
            } else {
                None
            },
            github_api: profile.uses_github_download_headers(url),
        }
    }

    /// Override the User-Agent with a manifest's `checkver.useragent`
    /// (`bin/checkver.ps1:118` substitutes it into the request headers).
    pub(crate) fn with_user_agent(mut self, user_agent: Option<&str>) -> Self {
        if let Some(ua) = user_agent {
            self.user_agent = ua.to_owned();
        }
        self
    }

    /// Materialize [`RequestOptions`][internal::network::RequestOptions]
    /// borrowing this resolution.
    pub(crate) fn options(&self, timeout_secs: u64) -> internal::network::RequestOptions<'_> {
        internal::network::RequestOptions {
            proxy: self.proxy.as_deref(),
            timeout_secs,
            user_agent: Some(&self.user_agent),
            referer: self.referer.as_deref(),
            cookies: None,
            extra_headers: self.extra_headers.as_ref(),
            token: self.token.as_deref(),
            github_api: self.github_api,
            range_start: 0,
        }
    }
}

/// Perform an HTTP HEAD request with Scoop's `checkurls` headers
/// (`bin/checkurls.ps1:56-70`).
///
/// `cookies` is optional — pass `Some(&map)` for manifest-level cookies.
pub async fn head_url(
    session: &Session,
    url: &str,
    timeout_secs: u64,
    cookies: Option<&HashMap<String, String>>,
) -> internal::network::HeadResult {
    let request = ScoopRequest::for_url(session, url, HeaderProfile::CheckUrls);
    let opts = internal::network::RequestOptions {
        cookies,
        ..request.options(timeout_secs)
    };
    session.network().head(url, &opts).await
}

/// Download a file via HTTP GET and save it to a local path, using the session's
/// proxy.
///
/// Upstream reaches these downloads through `Invoke-CachedDownload` /
/// `Invoke-Download` as well (`lib/download.ps1:23`, `:53-96`), so the full
/// download header contract applies — User-Agent, Referer (suppressed for
/// SourceForge/PortableApps), PRIVATE_HOSTS and GitHub API headers. Upstream
/// passes no cookies on either of the two callers of this function
/// (`bin/checkhashes.ps1:122`, `lib/autoupdate.ps1:319`), so none are sent.
pub async fn download_file(session: &Session, url: &str, dest: &Path) -> Fallible<()> {
    let request = ScoopRequest::for_url(session, url, HeaderProfile::Download);
    let opts = request.options(120);
    let data = session
        .network()
        .download(url, &opts)
        .await
        .map_err(|e| Error::Custom(e.to_string()))?;
    if let Some(parent) = dest.parent() {
        internal::fs::ensure_dir(parent)?;
    }
    std::fs::write(dest, &data)?;
    Ok(())
}

/// Download a URL's content as a UTF-8 string with Scoop's `autoupdate` headers
/// (`lib/autoupdate.ps1:20-206`).
///
/// `token` is optional — pass `Some("ghp_...")` for GitHub API Bearer auth.
pub async fn download_page(
    session: &Session,
    url: &str,
    timeout_secs: u64,
    token: Option<&str>,
) -> Fallible<String> {
    let request = ScoopRequest::for_url(session, url, HeaderProfile::Autoupdate);
    let mut opts = request.options(timeout_secs);
    if token.is_some() {
        opts.token = token;
    }
    let data = session
        .network()
        .download(url, &opts)
        .await
        .map_err(Error::Custom)?;
    String::from_utf8(data).map_err(|e| Error::Custom(format!("UTF-8 decode error: {}", e)))
}

#[cfg(test)]
mod tests {
    //! Wire-level assertions: every header hok promises has to survive the trip
    //! through the real `reqwest` client, so these tests read the bytes a
    //! loopback server actually received instead of inspecting `RequestOptions`.

    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serve exactly one request, returning the raw request head it received.
    fn capture_one_request() -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut chunk = [0u8; 512];
            loop {
                let n = stream.read(&mut chunk).unwrap();
                if n == 0 {
                    break;
                }
                head.extend_from_slice(&chunk[..n]);
                if head.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let body = b"fixture-payload";
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\
                     Content-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            );
            let _ = stream.write_all(body);
            let _ = stream.flush();
            String::from_utf8_lossy(&head).to_lowercase()
        });
        (format!("http://{addr}/pkg.bin"), handle)
    }

    /// A session with a PRIVATE_HOSTS entry covering loopback, so the extra
    /// headers are observable on the wire.
    fn session_with_private_hosts(root: &Path) -> Session {
        let config_path = root.join("hok.json");
        let escaped = |p: &Path| p.to_string_lossy().replace('\\', "\\\\");
        std::fs::write(
            &config_path,
            format!(
                r#"{{"root_path": "{}", "cache_path": "{}",
                     "private_hosts": [{{"match": "127\\.0\\.0\\.1", "headers": "X-Private=yes"}}]}}"#,
                escaped(root),
                escaped(&root.join("cache")),
            ),
        )
        .unwrap();
        Session::new_with(&config_path).unwrap()
    }

    /// `Invoke-Download` (`lib/download.ps1:88-111`): User-Agent, Referer and
    /// PRIVATE_HOSTS must all be on the wire for file downloads.
    #[tokio::test]
    async fn download_file_sends_the_download_header_contract() {
        let root = crate::test_utils::tmpdir("d4_download_headers");
        let session = session_with_private_hosts(&root);
        let (url, server) = capture_one_request();
        let dest = root.join("out.bin");

        download_file(&session, &url, &dest).await.unwrap();
        let head = server.join().unwrap();

        assert!(
            head.contains(&format!(
                "user-agent: {}",
                DEFAULT_USER_AGENT.to_lowercase()
            )),
            "User-Agent missing or wrong:\n{head}"
        );
        let port = url
            .trim_start_matches("http://")
            .trim_end_matches("/pkg.bin");
        assert!(
            head.contains(&format!("referer: http://{port}/")),
            "Referer missing or wrong:\n{head}"
        );
        assert!(
            head.contains("x-private: yes"),
            "PRIVATE_HOSTS header missing:\n{head}"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"fixture-payload");
    }

    /// `lib/download.ps1:94`: SourceForge/PortableApps downloads carry no
    /// Referer.
    #[tokio::test]
    async fn download_file_suppresses_the_referer_for_sourceforge_urls() {
        let root = crate::test_utils::tmpdir("d4_download_no_referer");
        let session = session_with_private_hosts(&root);
        let (url, server) = capture_one_request();
        let url = format!("{url}?mirror=downloads.sourceforge.net");

        download_file(&session, &url, &root.join("out.bin"))
            .await
            .unwrap();
        let head = server.join().unwrap();

        assert!(
            head.contains(&format!(
                "user-agent: {}",
                DEFAULT_USER_AGENT.to_lowercase()
            )),
            "User-Agent missing:\n{head}"
        );
        assert!(
            !head.contains("referer:"),
            "Referer must be suppressed for SourceForge:\n{head}"
        );
    }

    /// `checkurls` (`bin/checkurls.ps1:56-70`): HEAD carries the same
    /// User-Agent and an unconditional Referer — the pre-D4 code hard-coded a
    /// *different* User-Agent here and ignored the session's.
    #[tokio::test]
    async fn head_url_sends_the_checkurls_header_contract() {
        let root = crate::test_utils::tmpdir("d4_head_url_headers");
        let session = session_with_private_hosts(&root);
        let (url, server) = capture_one_request();

        let result = head_url(&session, &url, 5, None).await;
        let head = server.join().unwrap();

        assert_eq!(result.status_code, 200);
        assert!(
            head.contains(&format!(
                "user-agent: {}",
                DEFAULT_USER_AGENT.to_lowercase()
            )),
            "User-Agent missing or wrong:\n{head}"
        );
        assert!(head.contains("referer:"), "Referer missing:\n{head}");
        assert!(
            head.contains("x-private: yes"),
            "PRIVATE_HOSTS header missing:\n{head}"
        );
    }

    /// `bin/checkver.ps1:118-246`: the checkver page fetch carries the
    /// manifest's User-Agent, a Referer and PRIVATE_HOSTS.
    #[tokio::test]
    async fn checkver_profile_headers_reach_the_wire() {
        let root = crate::test_utils::tmpdir("d4_checkver_headers");
        let session = session_with_private_hosts(&root);
        let (url, server) = capture_one_request();

        let request = ScoopRequest::for_url(&session, &url, HeaderProfile::Checkver)
            .with_user_agent(Some("ManifestAgent/9.9"));
        session
            .network()
            .download(&url, &request.options(5))
            .await
            .unwrap();
        let head = server.join().unwrap();

        assert!(
            head.contains("user-agent: manifestagent/9.9"),
            "manifest User-Agent missing:\n{head}"
        );
        assert!(head.contains("referer:"), "Referer missing:\n{head}");
        assert!(
            head.contains("x-private: yes"),
            "PRIVATE_HOSTS header missing:\n{head}"
        );
    }

    /// `bin/checkver.ps1:118`: a manifest's `checkver.useragent` replaces the
    /// default User-Agent.
    #[test]
    fn checkver_profile_carries_the_manifest_user_agent() {
        let root = crate::test_utils::tmpdir("d4_manifest_user_agent");
        let session = session_with_private_hosts(&root);
        let request =
            ScoopRequest::for_url(&session, "https://example.com/x", HeaderProfile::Checkver)
                .with_user_agent(Some("CustomAgent/1.0"));
        assert_eq!(
            request.options(5).user_agent,
            Some("CustomAgent/1.0"),
            "manifest user agent must win"
        );
    }
}
