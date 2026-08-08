//! URL string helpers.
//!
//! Provides utility functions for common parsing operations on URL strings, such as extracting the filename,
//! extracting the basename, and decoding percent-encoding.
//!
//! # Usage
//!
//! These functions are pure functions (no I/O, no network) and can be safely called from any thread.
//!
//! # Notes
//!
//! - [`remote_filename`] first percent-decodes the URL, then extracts the last path segment;
//!   unlike [`basename`], which does not decode and strips the extension.
//! - These functions do not validate the URL; invalid URLs typically return the entire input string.

use regex::Regex;

/// Extracts the filename portion of a URL (the last path segment, percent-decoded).
///
/// # Example
///
/// ```
/// # use libscoop::internal::url::remote_filename;
/// assert_eq!(remote_filename("https://example.com/foo%20bar.zip"), "foo bar.zip");
/// assert_eq!(remote_filename("https://example.com/pkg/"), "");
/// ```
pub fn remote_filename(url: &str) -> String {
    let decoded = decoded(url);
    decoded.rsplit('/').next().unwrap_or(&decoded).to_string()
}

/// Extract the basename of the URL (the part after removing the file extension, without percent-decoding).
///
/// # Example
///
/// ```
/// # use libscoop::internal::url::basename;
/// assert_eq!(basename("https://example.com/archive.tar.gz"), "archive.tar");
/// assert_eq!(basename("https://example.com/noext"), "noext");
/// ```
pub fn basename(url: &str) -> String {
    let filename = url.rsplit('/').next().unwrap_or(url);
    match filename.rfind('.') {
        Some(pos) => filename[..pos].to_string(),
        None => filename.to_string(),
    }
}

/// Decode URL percent-encoding, replacing `%XX` with the corresponding byte character.
///
/// For invalid `%XX` sequences (non-hexadecimal characters), the input character is preserved as-is.
///
/// # Example
///
/// ```
/// # use libscoop::internal::url::decoded;
/// assert_eq!(decoded("hello%20world"), "hello world");
/// assert_eq!(decoded("no_encoding"), "no_encoding");
/// ```
pub fn decoded(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
                continue;
            }
            result.push('%');
            result.push_str(&hex);
        } else {
            result.push(c);
        }
    }
    result
}

/// Strip the fragment part (`#...`) from a raw URL string.
///
/// Scoop manifest URLs may contain `#/dl.7z` or similar fragment hints that
/// instruct the downloader to rename the file. This helper strips the fragment
/// so that the resulting string is a plain HTTP URL suitable for a HEAD check.
pub fn strip_url_fragment(raw: &str) -> &str {
    raw.split('#').next().unwrap_or(raw)
}

/// Strip the query part (`?...`) from a raw URL string.
///
/// Download URLs often carry query parameters (e.g. `?download=1`); the query
/// must not leak into derived filenames or extension-based archive detection.
/// The Scoop rename fragment (`#/...`) is deliberately kept — it may follow
/// the query (URL grammar puts `#` after `?`), and the target
/// filename/extension lives there.
pub fn strip_url_query(raw: &str) -> String {
    let (body, fragment) = match raw.split_once('#') {
        Some((body, fragment)) => (body, Some(fragment)),
        None => (raw, None),
    };
    let body = body.split('?').next().unwrap_or(body);
    match fragment {
        Some(fragment) => format!("{}#{}", body, fragment),
        None => body.to_string(),
    }
}

// ---------------------------------------------------------------------------
// GitHub / SourceForge URL detection & parsing
// ---------------------------------------------------------------------------

/// Whether `url` is a github.com web URL (homepage, releases page, or release
/// download asset), excluding the `api.github.com` host.
pub fn is_github_web_url(url: &str) -> bool {
    url.contains("github.com/") && !url.contains("api.github.com")
}

/// Whether `url` points at the `api.github.com` host.
pub fn is_github_api_url(url: &str) -> bool {
    url.contains("api.github.com")
}

/// Whether `url` is a github.com releases page or asset URL (contains
/// `/releases/` on the github.com host, e.g. `/releases/latest` or
/// `/releases/download/...`).
pub fn is_github_releases_url(url: &str) -> bool {
    is_github_web_url(url) && url.contains("/releases/")
}

/// Whether `url` is a github.com `/releases/download/...` asset URL.
pub fn is_github_releases_download_url(url: &str) -> bool {
    is_github_web_url(url) && url.contains("/releases/download/")
}

/// Extract `(owner, repo)` from a github.com URL.
///
/// Accepts all common forms:
/// - `https://github.com/owner/repo`
/// - `https://github.com/owner/repo/releases/...`
/// - `https://github.com/owner/repo/releases/download/...`
/// - `git@github.com:owner/repo.git` (SSH-style `:` separator)
/// - `ssh://git@github.com/owner/repo.git`
///
/// A trailing `.git` suffix is stripped. Returns `None` for non-github URLs
/// and for `api.github.com` URLs.
pub fn github_owner_repo(url: &str) -> Option<(String, String)> {
    if url.contains("api.github.com") {
        return None;
    }
    let re = Regex::new(r"github\.com[:/]([^/]+)/([^/]+?)(?:/|$)").ok()?;
    let caps = re.captures(url)?;
    let owner = caps.get(1)?.as_str().to_string();
    let mut repo = caps.get(2)?.as_str().trim_end_matches('/');
    if repo.len() > 4 && repo.ends_with(".git") {
        repo = &repo[..repo.len() - 4];
    }
    Some((owner, repo.to_string()))
}

/// Build the GitHub API "latest release" URL (`.../releases/latest`) from a
/// github.com homepage or releases URL.
pub fn github_releases_api_url(url_or_homepage: &str) -> Option<String> {
    github_owner_repo(url_or_homepage).map(|(owner, repo)| {
        format!("https://api.github.com/repos/{owner}/{repo}/releases/latest")
    })
}

/// Build the GitHub API releases-list URL (`.../releases`) from a github.com URL.
pub fn github_releases_list_api_url(url: &str) -> Option<String> {
    github_owner_repo(url)
        .map(|(owner, repo)| format!("https://api.github.com/repos/{owner}/{repo}/releases"))
}

/// Extract SourceForge project name from a homepage URL.
pub fn extract_sourceforge_project(homepage: &str) -> Option<String> {
    let re = Regex::new(r"sourceforge\.net/projects/([^/]+)").ok()?;
    let caps = re.captures(homepage)?;
    caps.get(1).map(|m| m.as_str().to_string())
}

/// Parse a SourceForge download URL to extract `(project, file_path)`.
pub fn parse_sourceforge_url(url: &str) -> Option<(String, String)> {
    // Scoop regex: '(?:downloads\.)?sourceforge.net\/projects?\/(?<project>[^\/]+)\/(?:files\/)?(?<file>.*)'
    let re = Regex::new(
        r"(?:downloads\.)?(?:sourceforge\.net|sf\.net)/projects?/([^/]+)(?:/files)?/(.*)",
    )
    .ok()?;
    let caps = re.captures(url)?;
    let project = caps.get(1)?.as_str().to_string();
    let file_path = caps.get(2)?.as_str().trim_end_matches('/').to_string();
    Some((project, file_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_filename_decodes_percent() {
        assert_eq!(
            remote_filename("https://example.com/foo%20bar.zip"),
            "foo bar.zip"
        );
    }

    #[test]
    fn remote_filename_plain_url() {
        assert_eq!(
            remote_filename("https://example.com/releases/app-1.0.zip"),
            "app-1.0.zip"
        );
    }

    #[test]
    fn basename_strips_extension() {
        assert_eq!(
            basename("https://example.com/archive.tar.gz"),
            "archive.tar"
        );
    }

    #[test]
    fn basename_no_extension() {
        assert_eq!(basename("https://example.com/noext"), "noext");
    }

    #[test]
    fn decoded_percent_space() {
        assert_eq!(decoded("hello%20world"), "hello world");
    }

    #[test]
    fn decoded_no_encoding() {
        assert_eq!(decoded("plaintext"), "plaintext");
    }

    #[test]
    fn decoded_invalid_percent_kept() {
        // %ZZ is not valid hex, should be kept as-is
        assert!(decoded("%ZZ").contains('%'));
    }

    // ── strip_url_fragment ──────────────────────────────────────────────────

    #[test]
    fn strip_fragment_removes_hash_suffix() {
        assert_eq!(
            strip_url_fragment("https://example.com/foo.exe#/dl.7z"),
            "https://example.com/foo.exe"
        );
    }

    #[test]
    fn strip_fragment_no_hash_returns_original() {
        assert_eq!(
            strip_url_fragment("https://example.com/foo.zip"),
            "https://example.com/foo.zip"
        );
    }

    #[test]
    fn strip_fragment_empty_string() {
        assert_eq!(strip_url_fragment(""), "");
    }

    #[test]
    fn strip_fragment_only_hash() {
        assert_eq!(strip_url_fragment("#anchor"), "");
    }

    // ── strip_url_query ────────────────────────────────────────────────────

    #[test]
    fn strip_query_removes_parameters() {
        assert_eq!(
            strip_url_query("https://example.com/pkg.zip?download=1"),
            "https://example.com/pkg.zip"
        );
    }

    #[test]
    fn strip_query_no_query_returns_original() {
        assert_eq!(
            strip_url_query("https://example.com/foo.zip"),
            "https://example.com/foo.zip"
        );
    }

    #[test]
    fn strip_query_keeps_rename_fragment() {
        // The Scoop `#/dl.7z` rename hint must survive query stripping.
        assert_eq!(
            strip_url_query("https://example.com/dl?token=abc#/dl.7z"),
            "https://example.com/dl#/dl.7z"
        );
    }

    #[test]
    fn strip_query_empty_string() {
        assert_eq!(strip_url_query(""), "");
    }

    // ── is_github_web_url / is_github_api_url ───────────────────────────────

    #[test]
    fn github_web_url_detected() {
        assert!(is_github_web_url("https://github.com/BurntSushi/ripgrep"));
        assert!(is_github_web_url("https://github.com/sharkdp/bat/releases/latest"));
        assert!(is_github_web_url(
            "https://github.com/owner/repo/releases/download/1.0/app.zip"
        ));
    }

    #[test]
    fn github_web_url_excludes_api_host() {
        assert!(!is_github_web_url("https://api.github.com/repos/owner/repo"));
    }

    #[test]
    fn github_web_url_non_github_false() {
        assert!(!is_github_web_url("https://example.com/owner/repo"));
    }

    #[test]
    fn github_api_url_detected() {
        assert!(is_github_api_url("https://api.github.com/repos/owner/repo"));
        assert!(!is_github_api_url("https://github.com/owner/repo"));
    }

    // ── is_github_releases_url / is_github_releases_download_url ────────────

    #[test]
    fn github_releases_url_detected() {
        assert!(is_github_releases_url("https://github.com/owner/repo/releases/latest"));
        assert!(is_github_releases_url(
            "https://github.com/owner/repo/releases/download/1.0/app.zip"
        ));
        assert!(!is_github_releases_url("https://github.com/owner/repo"));
        assert!(!is_github_releases_url("https://api.github.com/repos/owner/repo/releases"));
    }

    #[test]
    fn github_releases_download_url_detected() {
        assert!(is_github_releases_download_url(
            "https://github.com/owner/repo/releases/download/1.0/app.zip"
        ));
        assert!(!is_github_releases_download_url(
            "https://github.com/owner/repo/releases/latest"
        ));
    }

    // ── github_owner_repo ───────────────────────────────────────────────────

    #[test]
    fn github_owner_repo_from_homepage() {
        assert_eq!(
            github_owner_repo("https://github.com/BurntSushi/ripgrep"),
            Some(("BurntSushi".to_string(), "ripgrep".to_string()))
        );
    }

    #[test]
    fn github_owner_repo_from_releases_url() {
        assert_eq!(
            github_owner_repo("https://github.com/sharkdp/bat/releases/latest"),
            Some(("sharkdp".to_string(), "bat".to_string()))
        );
    }

    #[test]
    fn github_owner_repo_from_download_url() {
        assert_eq!(
            github_owner_repo(
                "https://github.com/BurntSushi/ripgrep/releases/download/14.1.1/ripgrep-14.1.1.zip"
            ),
            Some(("BurntSushi".to_string(), "ripgrep".to_string()))
        );
    }

    #[test]
    fn github_owner_repo_ssh_separator() {
        assert_eq!(
            github_owner_repo("git@github.com:owner/repo.git"),
            Some(("owner".to_string(), "repo".to_string()))
        );
    }

    #[test]
    fn github_owner_repo_ssh_url_scheme() {
        assert_eq!(
            github_owner_repo("ssh://git@github.com/owner/repo.git"),
            Some(("owner".to_string(), "repo".to_string()))
        );
    }

    #[test]
    fn github_owner_repo_non_github_returns_none() {
        assert_eq!(github_owner_repo("https://example.com/owner/repo"), None);
    }

    #[test]
    fn github_owner_repo_api_url_returns_none() {
        assert_eq!(github_owner_repo("https://api.github.com/repos/owner/repo"), None);
    }

    // ── github_releases_api_url / github_releases_list_api_url ──────────────

    #[test]
    fn github_releases_api_url_from_homepage() {
        let result = github_releases_api_url("https://github.com/BurntSushi/ripgrep");
        assert!(result.is_some());
        let url = result.unwrap();
        assert!(url.contains("api.github.com"));
        assert!(url.contains("BurntSushi/ripgrep"));
        assert!(url.ends_with("/releases/latest"));
    }

    #[test]
    fn github_releases_api_url_from_releases_url() {
        let result = github_releases_api_url("https://github.com/sharkdp/bat/releases/latest");
        assert!(result.is_some());
        assert!(result.unwrap().contains("api.github.com"));
    }

    #[test]
    fn github_releases_api_url_non_github_returns_none() {
        let result = github_releases_api_url("https://example.com/owner/repo");
        assert!(result.is_none());
    }

    #[test]
    fn github_releases_list_api_url_builds_list_endpoint() {
        let result = github_releases_list_api_url(
            "https://github.com/BurntSushi/ripgrep/releases/download/14.1.1/ripgrep.zip",
        )
        .unwrap();
        assert_eq!(
            result,
            "https://api.github.com/repos/BurntSushi/ripgrep/releases"
        );
    }

    #[test]
    fn github_releases_list_api_url_non_github_returns_none() {
        assert_eq!(
            github_releases_list_api_url("https://example.com/owner/repo"),
            None
        );
    }

    // ── extract_sourceforge_project / parse_sourceforge_url ─────────────────

    #[test]
    fn sourceforge_extracts_project_from_homepage() {
        let result = extract_sourceforge_project("https://sourceforge.net/projects/sevenzip/");
        assert_eq!(result.as_deref(), Some("sevenzip"));
    }

    #[test]
    fn sourceforge_returns_none_for_non_sf_url() {
        let result = extract_sourceforge_project("https://example.com/project");
        assert!(result.is_none());
    }

    #[test]
    fn parse_sourceforge_download_url() {
        let (project, file_path) = parse_sourceforge_url(
            "https://downloads.sourceforge.net/project/sevenzip/7-Zip/24.09/7z2409-x64.exe",
        )
        .unwrap();
        assert_eq!(project, "sevenzip");
        assert_eq!(file_path, "7-Zip/24.09/7z2409-x64.exe");
    }

    #[test]
    fn parse_sourceforge_url_sf_net_shortcut() {
        let (project, file_path) =
            parse_sourceforge_url("https://sf.net/projects/foo/files/bar/baz.zip").unwrap();
        assert_eq!(project, "foo");
        assert_eq!(file_path, "bar/baz.zip");
    }

    #[test]
    fn parse_sourceforge_url_non_sf_returns_none() {
        assert_eq!(parse_sourceforge_url("https://example.com/files/x.zip"), None);
    }
}
