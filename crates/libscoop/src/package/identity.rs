//! Package identity — bucket/name splitting and extraction.
//!
//! Helpers that interpret a user-supplied query or dependency string of the
//! form `bucket/name` (or `bucket\name`) and extract the package name part.
//! Shared by `query`, `resolve`, `sync_install`, and `sync_remove`.

/// Split a bucket-qualified query ("bucket/name" or "bucket\name") into the
/// bucket prefix and the package name.
///
/// A backslash is only treated as a separator when it is not the leading
/// character, so regex escapes like `\d` keep their meaning in non-explicit
/// mode.
pub(crate) fn split_bucket_query(query: &str) -> (Option<String>, &str) {
    if let Some((bucket, name)) = query.split_once('/') {
        return (Some(bucket.to_owned()), name);
    }
    if let Some(pos) = query.find('\\') {
        if pos > 0 {
            return (Some(query[..pos].to_owned()), &query[pos + 1..]);
        }
    }
    (None, query)
}

/// Extact `name` from `bucket/name`.
pub(crate) fn extract_name<S: AsRef<str>>(input: S) -> String {
    input
        .as_ref()
        .split_once('/')
        .map(|(_, n)| n)
        .unwrap_or(input.as_ref())
        .to_owned()
}

/// A parsed install query of the form `bucket/name`, `name@version`,
/// `bucket/name@version`, a manifest URL (`https://…/app.json[@version]`),
/// or a local manifest path (`\path\to\app.json[@version]`).
///
/// Mirrors upstream `parse_app` (lib/core.ps1): the `app` part is either a
/// bare name (`[a-zA-Z0-9_.-]+`) or anything ending in `.json` (URLs, UNC
/// paths, local files); a `@version` suffix is only split off when it
/// directly follows the `.json` part, so `@` inside a URL (userinfo etc.)
/// stays part of the app string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppQuery {
    /// Bucket prefix (`bucket/name`), if given.
    pub bucket: Option<String>,
    /// The app part: a bare name, or the full URL / path / file name.
    pub app: String,
    /// The requested version (`name@version`), if given.
    pub version: Option<String>,
}

static APP_QUERY_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"^(?:(?<bucket>[a-zA-Z0-9_.-]+)/)?(?<app>.*\.json|[a-zA-Z0-9_.-]+)(?:@(?<version>.*))?$",
    )
    .expect("valid app query regex")
});

/// Parse a user-supplied install query into its components, following the
/// same grammar as upstream Scoop's `parse_app` (lib/core.ps1).
///
/// Returns `None` when the query does not match the grammar at all (e.g. a
/// regex escape like `\d` used as a search query).
pub(crate) fn parse_app(query: &str) -> Option<AppQuery> {
    let caps = APP_QUERY_RE.captures(query)?;
    Some(AppQuery {
        bucket: caps.name("bucket").map(|m| m.as_str().to_owned()),
        app: caps
            .name("app")
            .map(|m| m.as_str().to_owned())
            .unwrap_or_else(|| query.to_owned()),
        version: caps.name("version").map(|m| m.as_str().to_owned()),
    })
}

/// Whether the given string is a manifest URL or UNC path, matching upstream
/// `Get-Manifest`'s `'^(ht|f)tps?://|\\\\'` check.
pub(crate) fn is_manifest_url(s: &str) -> bool {
    s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("ftp://")
        || s.starts_with("ftps://")
        || s.starts_with("\\\\")
}

/// Extract the app name from a manifest URL or path, matching upstream
/// `appname_from_url` (lib/core.ps1): the last path segment without the
/// `.json` extension.
pub(crate) fn appname_from_url(url: &str) -> String {
    let leaf = url.rsplit(['/', '\\']).next().unwrap_or(url);
    leaf.strip_suffix(".json").unwrap_or(leaf).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_bucket_query_handles_both_separators() {
        assert_eq!(
            split_bucket_query("extras/curl"),
            (Some("extras".to_owned()), "curl")
        );
        assert_eq!(
            split_bucket_query("extras\\curl"),
            (Some("extras".to_owned()), "curl")
        );
        assert_eq!(split_bucket_query("curl"), (None, "curl"));
        assert_eq!(split_bucket_query("\\d"), (None, "\\d"));
    }

    #[test]
    fn extract_name_strips_bucket_prefix() {
        assert_eq!(extract_name("extras/curl"), "curl");
        assert_eq!(extract_name("curl"), "curl");
    }

    // ── parse_app ──────────────────────────────────────────────────────────

    fn q(bucket: Option<&str>, app: &str, version: Option<&str>) -> AppQuery {
        AppQuery {
            bucket: bucket.map(str::to_owned),
            app: app.to_owned(),
            version: version.map(str::to_owned),
        }
    }

    #[test]
    fn parse_app_plain_name() {
        assert_eq!(parse_app("git"), Some(q(None, "git", None)));
    }

    #[test]
    fn parse_app_bucket_qualified() {
        assert_eq!(
            parse_app("extras/curl"),
            Some(q(Some("extras"), "curl", None))
        );
        assert_eq!(parse_app("main/curl"), Some(q(Some("main"), "curl", None)));
    }

    #[test]
    fn parse_app_specific_version() {
        assert_eq!(parse_app("gh@2.7.0"), Some(q(None, "gh", Some("2.7.0"))));
        assert_eq!(
            parse_app("extras/gh@2.7.0"),
            Some(q(Some("extras"), "gh", Some("2.7.0")))
        );
    }

    #[test]
    fn parse_app_manifest_url() {
        let url = "https://raw.githubusercontent.com/ScoopInstaller/Main/master/bucket/runat.json";
        assert_eq!(parse_app(url), Some(q(None, url, None)));
    }

    #[test]
    fn parse_app_manifest_url_with_version() {
        let url = "https://raw.githubusercontent.com/ScoopInstaller/Main/master/bucket/neovim.json";
        assert_eq!(
            parse_app(&format!("{url}@0.9.0")),
            Some(q(None, url, Some("0.9.0")))
        );
    }

    #[test]
    fn parse_app_local_path() {
        assert_eq!(
            parse_app(r"\path\to\app.json"),
            Some(q(None, r"\path\to\app.json", None))
        );
        assert_eq!(
            parse_app(r"\path\to\app.json@version"),
            Some(q(None, r"\path\to\app.json", Some("version")))
        );
    }

    #[test]
    fn parse_app_at_inside_url_stays_in_app() {
        // `@` before the final `.json` is part of the URL (userinfo), not a version.
        let url = "https://user@host.com/app.json";
        assert_eq!(parse_app(url), Some(q(None, url, None)));
    }

    #[test]
    fn parse_app_rejects_regex_escape() {
        // `\d` matches neither the bucket prefix nor the bare-name class.
        assert_eq!(parse_app("\\d"), None);
        assert_eq!(parse_app(""), None);
    }

    // ── appname_from_url ───────────────────────────────────────────────────

    #[test]
    fn appname_from_url_strips_json_suffix() {
        assert_eq!(
            appname_from_url("https://example.com/bucket/runat.json"),
            "runat"
        );
        assert_eq!(appname_from_url("C:\\path\\to\\app.json"), "app");
        assert_eq!(appname_from_url("plain"), "plain");
    }

    // ── is_manifest_url ────────────────────────────────────────────────────

    #[test]
    fn is_manifest_url_detects_urls_and_unc() {
        assert!(is_manifest_url("https://example.com/app.json"));
        assert!(is_manifest_url("http://example.com/app.json"));
        assert!(is_manifest_url("\\\\server\\share\\app.json"));
        assert!(!is_manifest_url("git"));
        assert!(!is_manifest_url("C:\\path\\to\\app.json"));
    }
}
