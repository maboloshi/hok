//! Manifest JSON formatting utilities.
//!
//! Standardize formatting for Scoop manifest JSON files:
//! After parsing into a JSON value, re-serialize with 4-space indentation + CRLF line endings,
//! consistent with Scoop's official conventions.
//!
//! # Usage
//!
//! Typically used in conjunction with [`crate::package::manifest_walker::discover`],
//! to batch-scan manifests in a bucket directory and format them.
//!
//! ```no_run
//! use std::path::Path;
//! use libscoop::package::formatjson;
//!
//! let changed = formatjson::format_manifest_file(Path::new("app.json")).unwrap();
//! if changed {
//!     println!("Reformatted app.json");
//! }
//! ```
//!
//! # Notes
//!
//! - Only write when the file content has actually changed (avoid unnecessary disk writes).
//! - Supports glob wildcard filtering (`*`, `?`).
//! - If the input file contains a BOM (`\u{FEFF}`), it will be automatically stripped.

use crate::{error::Fallible, Error};
use regex::Regex;
use serde::Serialize;
use std::path::Path;

/// Convert a simple glob pattern (`*`, `?`) to an anchored regex string.
///
/// `*` expands to `.*`, `?` expands to `.`.  All other regex metacharacters
/// in the original pattern are escaped.
///
/// # Examples
///
/// ```
/// # use libscoop::package::formatjson::glob_to_regex;
/// assert_eq!(glob_to_regex("curl*"), "^curl.*$");
/// assert_eq!(glob_to_regex("app?name"), "^app.name$");
/// ```
pub fn glob_to_regex(pattern: &str) -> String {
    let escaped = regex::escape(pattern);
    let re_str = escaped.replace(r"\*", ".*").replace(r"\?", ".");
    format!("^{re_str}$")
}

/// Check whether `name` matches the given app `pattern`.
///
/// When the pattern contains `*` or `?`, it is treated as a glob.
/// Otherwise, an exact stem match is performed (case-sensitive).
pub fn app_matches(name: &str, pattern: &str) -> bool {
    if pattern.contains('*') || pattern.contains('?') {
        let re_str = glob_to_regex(pattern);
        Regex::new(&re_str).is_ok_and(|re| re.is_match(name))
    } else {
        name == pattern
    }
}

/// Format a single manifest JSON file in place using Scoop conventions.
///
/// Reads the file, strips any BOM, parses as JSON5, then serialises with
/// 4-space indentation and CRLF line endings.  Writes back only if the
/// content changed.
///
/// # Returns
///
/// `Ok(true)` if the file was modified, `Ok(false)` if it was already
/// correctly formatted, or an error on I/O / parse failure.
pub fn format_manifest_file(path: &Path) -> Fallible<bool> {
    let content = std::fs::read_to_string(path)?;
    let cleaned = content.trim_start_matches('\u{FEFF}');

    let value: serde_json::Value = json5::from_str(cleaned)
        .map_err(|e| Error::Custom(format!("{}: parse error: {}", path.display(), e)))?;

    let formatted = to_scoop_json(&value)?;

    if formatted != content {
        std::fs::write(path, formatted.as_bytes())?;
        return Ok(true);
    }
    Ok(false)
}

/// Serialise a JSON value to a string with 4-space indentation and CRLF endings.
///
/// Matches the Scoop convention for manifest formatting.
pub fn to_scoop_json(value: &serde_json::Value) -> Fallible<String> {
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    value
        .serialize(&mut ser)
        .map_err(|e| Error::Custom(format!("serialize error: {}", e)))?;
    let mut formatted =
        String::from_utf8(buf).map_err(|e| Error::Custom(format!("utf8 error: {}", e)))?;
    formatted = formatted.replace('\n', "\r\n");
    if !formatted.ends_with("\r\n") {
        formatted.push_str("\r\n");
    }
    Ok(formatted)
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

    // ── app_matches ──────────────────────────────────────────────────────────

    #[test]
    fn app_matches_exact_same_name() {
        assert!(app_matches("curl", "curl"));
    }

    #[test]
    fn app_matches_exact_different_name() {
        assert!(!app_matches("wget", "curl"));
    }

    #[test]
    fn app_matches_glob_star() {
        assert!(app_matches("curl-7z", "curl*"));
        assert!(!app_matches("wget", "curl*"));
    }

    #[test]
    fn app_matches_glob_question() {
        assert!(app_matches("curl", "cur?"));
        assert!(!app_matches("curl2", "cur?"));
    }

    // ── to_scoop_json ────────────────────────────────────────────────────────

    #[test]
    fn to_scoop_json_uses_crlf() {
        let v = serde_json::json!({"version": "1.0"});
        let output = to_scoop_json(&v).unwrap();
        assert!(output.contains("\r\n"), "output should use CRLF");
    }

    #[test]
    fn to_scoop_json_ends_with_crlf() {
        let v = serde_json::json!({"a": 1});
        let output = to_scoop_json(&v).unwrap();
        assert!(output.ends_with("\r\n"));
    }
}
