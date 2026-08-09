//! String utilities — encoding, glob matching, and conversions.
//!
//! Pure string functions with no I/O and no business logic. Used by both
//! `internal/` and `package/` modules.
//!
//! # Design
//!
//! - **Encoding**: [`encode_wide()`] converts UTF-8 to null-terminated
//!   UTF-16LE for Windows FFI calls.
//! - **Glob matching**: [`glob_to_regex()`] turns a simple `*`/`?` glob into
//!   an anchored regex; [`matches_any_glob()`] checks a name against a list of
//!   glob patterns (Scoop's `checkurls -Filter` semantics).

/// Encode a UTF-8 string to a null-terminated UTF-16LE vector.
///
/// Used for Windows FFI calls (`ShellExecuteW`, etc.).
pub fn encode_wide(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Convert a simple glob pattern (`*`, `?`) to an anchored regex string.
///
/// `*` expands to `.*`, `?` expands to `.`.  All other regex metacharacters
/// in the original pattern are escaped.
///
/// # Examples
///
/// ```
/// # use libscoop::internal::string::glob_to_regex;
/// assert_eq!(glob_to_regex("curl*"), "^curl.*$");
/// assert_eq!(glob_to_regex("app?name"), "^app.name$");
/// ```
pub fn glob_to_regex(pattern: &str) -> String {
    let escaped = regex::escape(pattern);
    let re_str = escaped.replace(r"\*", ".*").replace(r"\?", ".");
    format!("^{re_str}$")
}

/// Determine whether `name` passes `patterns` (Scoop `checkurls` filter).
///
/// Returns `true` when the first pattern is `"*"` (wildcard) or when any
/// pattern matches `name` as a glob (`*` / `?` wildcards, case-sensitive).
/// Plain patterns without wildcards match exactly.
pub fn matches_any_glob(name: &str, patterns: &[String]) -> bool {
    if patterns.first().map(|s| s.as_str()) == Some("*") {
        return true;
    }
    patterns.iter().any(|p| {
        if p.contains('*') || p.contains('?') {
            regex::Regex::new(&glob_to_regex(p)).is_ok_and(|re| re.is_match(name))
        } else {
            name == p
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── glob_to_regex ────────────────────────────────────────────────────────

    #[test]
    fn glob_star_becomes_dotstar() {
        let re = glob_to_regex("curl*");
        assert_eq!(re, "^curl.*$");
    }

    #[test]
    fn glob_question_becomes_dot() {
        let re = glob_to_regex("app?name");
        assert_eq!(re, "^app.name$");
    }

    #[test]
    fn glob_no_wildcards_anchored_literal() {
        let re = glob_to_regex("exact");
        assert_eq!(re, "^exact$");
    }

    // ── matches_any_glob ─────────────────────────────────────────────────────

    #[test]
    fn wildcard_filter_matches_any_name() {
        let filters = vec!["*".to_string()];
        assert!(matches_any_glob("anyapp", &filters));
        assert!(matches_any_glob("", &filters));
    }

    #[test]
    fn specific_filter_matches_exact_name() {
        let filters = vec!["curl".to_string()];
        assert!(matches_any_glob("curl", &filters));
        assert!(!matches_any_glob("libcurl", &filters));
    }

    #[test]
    fn specific_filter_no_match() {
        let filters = vec!["wget".to_string()];
        assert!(!matches_any_glob("curl", &filters));
    }

    #[test]
    fn multiple_filters_any_match() {
        let filters = vec!["wget".to_string(), "curl".to_string()];
        assert!(matches_any_glob("curl", &filters));
        assert!(matches_any_glob("wget", &filters));
        assert!(!matches_any_glob("git", &filters));
    }

    #[test]
    fn empty_filters_no_match_unless_wildcard() {
        let filters: Vec<String> = vec![];
        // Empty filter list ≠ wildcard; first() is None, and no iter match.
        assert!(!matches_any_glob("app", &filters));
    }

    #[test]
    fn glob_pattern_matches_with_wildcards() {
        let filters = vec!["curl*".to_string()];
        assert!(matches_any_glob("curl-7z", &filters));
        assert!(!matches_any_glob("wget", &filters));
    }

    #[test]
    fn glob_question_matches_single_char() {
        let filters = vec!["cur?".to_string()];
        assert!(matches_any_glob("curl", &filters));
        assert!(!matches_any_glob("curl2", &filters));
    }
}
