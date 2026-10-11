//! String utilities — encoding, glob matching, and conversions.
//!
//! Pure string functions with no I/O and no business logic. Used by both
//! `internal/` and `package/` modules.
//!
//! # Design
//!
//! - **Encoding**: [`encode_wide()`] converts UTF-8 to null-terminated
//!   UTF-16LE for Windows FFI calls.
//! - **BOM**: [`strip_bom()`] removes the leading UTF-8 BOM that notepad
//!   writes ("UTF-8 with BOM"); upstream Scoop strips it while reading
//!   (`Get-Content -Encoding UTF8`, `[System.IO.File]::ReadAllLines`), so
//!   its parsers never see one.
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

/// Strip a single leading UTF-8 BOM (`U+FEFF`) from `text`.
///
/// Only the leading BOM is an encoding marker and only one is stripped;
/// a BOM in the middle of the text is data and is left alone, and a second
/// leading BOM is left to the consumer to reject (upstream strips exactly
/// one, while detecting the encoding).
pub fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{FEFF}').unwrap_or(text)
}

/// Convert a simple glob pattern (`*`, `?`) to an anchored regex string.
///
/// `*` expands to `.*`, `?` expands to `.`.  All other regex metacharacters
/// in the original pattern are escaped.
///
/// # Examples
///
/// ```
/// # use libscoop::string::glob_to_regex;
/// assert_eq!(glob_to_regex("curl*"), "^curl.*$");
/// assert_eq!(glob_to_regex("app?name"), "^app.name$");
/// ```
pub fn glob_to_regex(pattern: &str) -> String {
    let escaped = regex::escape(pattern);
    let re_str = escaped.replace(r"\*", ".*").replace(r"\?", ".");
    format!("^{re_str}$")
}

/// Whether `pattern` matches `stem` — the shared per-pattern half of
/// [`matches_any_glob`].
///
/// A trailing `.json` is ignored so callers can pass either an app name or a
/// manifest file name (`hok formatjson curl` and `hok formatjson curl.json`
/// are the same request), and matching is case-insensitive, mirroring
/// PowerShell's `Get-ChildItem -Filter "$App.json"`.
fn matches_glob(stem: &str, pattern: &str) -> bool {
    // PowerShell appends `.json` to the filter itself, so the caller may or may
    // not have spelled the extension — and `Curl.JSON` is as valid an input as
    // `curl.json`.
    let pattern = strip_json_suffix(pattern).unwrap_or(pattern);
    if pattern.contains('*') || pattern.contains('?') {
        regex::RegexBuilder::new(&glob_to_regex(pattern))
            .case_insensitive(true)
            .build()
            .is_ok_and(|re| re.is_match(stem))
    } else {
        stem.eq_ignore_ascii_case(pattern)
    }
}

/// `pattern` without a trailing `.json` (matched case-insensitively), or
/// `None` when there is none — or when nothing would be left of it.
fn strip_json_suffix(pattern: &str) -> Option<&str> {
    const EXT: &str = ".json";
    if pattern.len() <= EXT.len() {
        return None;
    }
    let (head, tail) = pattern.split_at(pattern.len() - EXT.len());
    tail.eq_ignore_ascii_case(EXT).then_some(head)
}

/// Determine whether `name` passes `patterns` (Scoop `checkurls` filter).
///
/// Returns `true` when the first pattern is `"*"` (wildcard) or when any
/// pattern matches `name` as a glob (`*` / `?` wildcards). Plain patterns
/// without wildcards match exactly.
///
/// Matching is **case-insensitive** and ignores a trailing `.json` on either
/// side of the comparison, so `curl`, `Curl` and `Curl.json` all select the
/// same manifest — mirroring `Get-ChildItem -Filter "$App.json"`, whose
/// PowerShell `-Filter` comparison likewise ignores case.
pub fn matches_any_glob(name: &str, patterns: &[String]) -> bool {
    if patterns.first().is_some_and(|p| matches_glob("*", p)) {
        return true;
    }
    patterns.iter().any(|p| matches_glob(name, p))
}

/// Normalize a JSONPath expression so `jsonpath-rust` can parse dotted field
/// names that contain characters outside `[A-Za-z0-9_]`.
///
/// `jsonpath-rust` 1.0.4's grammar only accepts plain identifiers after `.`,
/// so an expression like `$.dist-tags.latest` (a standard JSONPath form that
/// Scoop's PowerShell implementation accepts) fails to parse. This rewrites
/// such dotted segments into bracket notation: `$.dist-tags.latest` becomes
/// `$['dist-tags'].latest`, which `jsonpath-rust` does parse.
///
/// Only simple dotted paths (`$` followed by `.segment` segments) are
/// rewritten. Anything containing `[`, `*`, `?`, `..` (filters, wildcards,
/// recursive descent, existing bracket notation) is returned unchanged, and a
/// segment that cannot be safely quoted (empty, or containing `'` / `\`) also
/// short-circuits to the original expression — the caller's original query
/// then decides success or failure as before.
pub fn normalize_jsonpath(jp: &str) -> String {
    if !jp.starts_with("$.") || jp.contains(['[', '*', '?']) || jp.contains("..") {
        return jp.to_string();
    }
    let mut out = String::from("$");
    for seg in jp[2..].split('.') {
        if seg.is_empty() || seg.contains(['\'', '\\']) {
            return jp.to_string();
        }
        if seg.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            out.push('.');
            out.push_str(seg);
        } else {
            out.push_str("['");
            out.push_str(seg);
            out.push_str("']");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── strip_bom ────────────────────────────────────────────────────────────

    #[test]
    fn strip_bom_removes_one_leading_bom() {
        assert_eq!(strip_bom("\u{FEFF}{}"), "{}");
        // Only one: what remains is left to the parser to reject.
        assert_eq!(strip_bom("\u{FEFF}\u{FEFF}{}"), "\u{FEFF}{}");
    }

    #[test]
    fn strip_bom_leaves_other_text_untouched() {
        assert_eq!(strip_bom("{}"), "{}");
        assert_eq!(strip_bom(""), "");
        assert_eq!(strip_bom("\u{FEFF}"), "");
        // A BOM anywhere else is data, not an encoding marker.
        assert_eq!(strip_bom("{\"a\":\"\u{FEFF}\"}"), "{\"a\":\"\u{FEFF}\"}");
    }

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

    /// Scoop's `-App` filter is PowerShell `-Filter`-based, hence
    /// case-insensitive; `checkhashes -d <bucket> FOO` must find `foo.json`.
    #[test]
    fn filters_are_case_insensitive() {
        assert!(matches_any_glob("m1-ok", &["M1-OK".to_string()]));
        assert!(matches_any_glob("M1-OK", &["m1-ok".to_string()]));
        assert!(matches_any_glob("m1-ok", &["M1*".to_string()]));
        assert!(matches_any_glob("curl", &["CUR?".to_string()]));
        // Still a full-name match, not a substring one.
        assert!(!matches_any_glob("git-lfs", &["GIT".to_string()]));
    }

    /// Manifest discovery mirrors PowerShell's case-insensitive
    /// `Get-ChildItem -Filter "$App.json"`: `hok formatjson Curl` must still
    /// find `curl.json`, and a mixed-case glob must match too.
    #[test]
    fn filters_match_case_insensitively() {
        let exact = vec!["curl".to_string()];
        assert!(matches_any_glob("Curl", &exact));
        assert!(matches_any_glob("CURL", &exact));

        let glob = vec!["Cur*".to_string()];
        assert!(matches_any_glob("curl", &glob));
        assert!(matches_any_glob("CURL-7Z", &glob));
    }

    /// A `.json` suffix on the pattern is accepted and ignored, and a
    /// `.json`-suffixed wildcard still matches everything.
    #[test]
    fn filters_ignore_json_suffix() {
        assert!(matches_any_glob("curl", &["curl.json".to_string()]));
        assert!(matches_any_glob("curl", &["Curl.JSON".to_string()]));
        assert!(matches_any_glob("anyapp", &["*.json".to_string()]));
        assert!(!matches_any_glob("curl", &["wget.json".to_string()]));
    }

    // ── normalize_jsonpath ───────────────────────────────────────────────────

    #[test]
    fn jsonpath_rewrites_special_char_segments_to_bracket() {
        assert_eq!(
            normalize_jsonpath("$.dist-tags.latest"),
            "$['dist-tags'].latest"
        );
        assert_eq!(normalize_jsonpath("$.a-b-c.d"), "$['a-b-c'].d");
        assert_eq!(normalize_jsonpath("$.a.b-c.d_e"), "$.a['b-c'].d_e");
    }

    #[test]
    fn jsonpath_plain_identifiers_unchanged() {
        assert_eq!(normalize_jsonpath("$.tag_name"), "$.tag_name");
        assert_eq!(normalize_jsonpath("$.version"), "$.version");
    }

    #[test]
    fn jsonpath_complex_expressions_unchanged() {
        assert_eq!(
            normalize_jsonpath("$..assets[?(@.browser_download_url == 'x')].digest"),
            "$..assets[?(@.browser_download_url == 'x')].digest"
        );
        assert_eq!(
            normalize_jsonpath("$['dist-tags'].latest"),
            "$['dist-tags'].latest"
        );
        assert_eq!(normalize_jsonpath("$.*"), "$.*");
    }

    #[test]
    fn jsonpath_unquotable_or_malformed_unchanged() {
        assert_eq!(normalize_jsonpath("$"), "$");
        assert_eq!(normalize_jsonpath("$.a."), "$.a.");
        assert_eq!(normalize_jsonpath("$.a'b"), "$.a'b");
    }
}
