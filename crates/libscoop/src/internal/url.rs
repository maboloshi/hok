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

/// Determine whether `name` passes `app_filters`.
///
/// Returns `true` when the first filter is `"*"` (wildcard) or when any
/// filter is a substring of `name`.
pub fn app_filter_matches(name: &str, app_filters: &[String]) -> bool {
    if app_filters.first().map(|s| s.as_str()) == Some("*") {
        return true;
    }
    app_filters.iter().any(|p| name.contains(p.as_str()))
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

    // ── app_filter_matches ──────────────────────────────────────────────────

    #[test]
    fn wildcard_filter_matches_any_name() {
        let filters = vec!["*".to_string()];
        assert!(app_filter_matches("anyapp", &filters));
        assert!(app_filter_matches("", &filters));
    }

    #[test]
    fn specific_filter_matches_substring() {
        let filters = vec!["curl".to_string()];
        assert!(app_filter_matches("curl", &filters));
        assert!(app_filter_matches("libcurl", &filters));
    }

    #[test]
    fn specific_filter_no_match() {
        let filters = vec!["wget".to_string()];
        assert!(!app_filter_matches("curl", &filters));
    }

    #[test]
    fn multiple_filters_any_match() {
        let filters = vec!["wget".to_string(), "curl".to_string()];
        assert!(app_filter_matches("curl", &filters));
        assert!(app_filter_matches("wget", &filters));
        assert!(!app_filter_matches("git", &filters));
    }

    #[test]
    fn empty_filters_no_match_unless_wildcard() {
        let filters: Vec<String> = vec![];
        // Empty filter list ≠ wildcard; first() is None, and no iter match.
        assert!(!app_filter_matches("app", &filters));
    }
}
