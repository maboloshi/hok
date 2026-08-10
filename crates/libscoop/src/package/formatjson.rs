//! Manifest JSON formatting utilities.
//!
//! Standardize formatting for Scoop manifest JSON files:
//! After parsing into a JSON value, re-serialize with 4-space indentation + CRLF line endings,
//! consistent with Scoop's official conventions.
//!
//! # Usage
//!
//! Format a single manifest:
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
//! Or batch-format a whole bucket directory (with the same app-pattern
//! filtering as `checkhashes`/`checkver`/`checkurls`) via [`format_manifests`].
//! Both entry points are used by the `hok formatjson` command.
//!
//! # Notes
//!
//! - Only write when the file content has actually changed (avoid unnecessary disk writes).
//! - Supports glob wildcard filtering (`*`, `?`).
//! - If the input file contains a BOM (`\u{FEFF}`), it will be automatically stripped.
//! - Input is parsed tolerantly (JSON5: comments, trailing commas, single-quoted
//!   strings, unescaped newlines, etc. are accepted) and re-serialised as
//!   strict standard JSON — non-standard input in, standard JSON out.
//! - Semantics match Scoop's official `bin/formatjson.ps1` (`lib/json.ps1` `ConvertToPrettyJson`):
//!   single-element arrays collapse to scalars, multi-line strings split into arrays,
//!   single-element arrays nested inside arrays are flattened, and empty objects/arrays
//!   are expanded across lines.  Escape sequences inside strings are *not* unescaped
//!   (official behavior would alter string semantics, so this is intentionally skipped).

use crate::package::manifest_walker;
use crate::{error::Fallible, Error};
use serde::Serialize;
use std::path::Path;

/// Format a single manifest JSON file in place using Scoop conventions.
///
/// Reads the file, strips any BOM, parses it tolerantly (JSON5: comments,
/// trailing commas, etc. are accepted), then serialises with 4-space
/// indentation and CRLF line endings into strict JSON.  Writes back only if
/// the content changed.
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

/// Batch result of [`format_manifests`].
#[derive(Debug, Clone, Default)]
pub struct FormatReport {
    /// Manifests that were rewritten.
    pub formatted: u32,
    /// Manifests already correctly formatted (not rewritten).
    pub unchanged: u32,
    /// Error messages from manifests that could not be formatted.
    pub errors: Vec<String>,
    /// Warnings about manifests missing required fields
    /// (`version` / `homepage` / `license`). The parser tolerates their
    /// absence (upstream never validates them), but such manifests cannot
    /// be installed meaningfully, so `formatjson` calls them out.
    pub warnings: Vec<String>,
}

/// Format every manifest under `dir`, filtered by `app` patterns.
///
/// Uses the same app-filtering semantics as `checkhashes` / `checkver` /
/// `checkurls` (`manifest_walker::discover_matching` / `matches_any_glob`):
/// a first pattern of `"*"` matches everything, otherwise each stem must
/// match at least one pattern (glob `*`/`?` supported, plain patterns match
/// exactly). `package.json` is skipped, and results are processed in sorted
/// path order.
///
/// Per-file failures are collected into [`FormatReport::errors`] instead of
/// aborting the whole batch.
pub fn format_manifests(dir: &Path, app: &[String]) -> Fallible<FormatReport> {
    let mut report = FormatReport::default();
    for (path, _stem) in manifest_walker::discover_matching(dir, app)? {
        match format_manifest_file(&path) {
            Ok(true) => report.formatted += 1,
            Ok(false) => report.unchanged += 1,
            Err(e) => report.errors.push(e.to_string()),
        }
        report.warnings.extend(missing_fields(&path));
    }
    Ok(report)
}

/// Check a manifest for missing required fields (`version` / `homepage` /
/// `license`). The manifest parser tolerates their absence, but a manifest
/// without them cannot be installed meaningfully, so they are surfaced as
/// warnings during `formatjson`.
fn missing_fields(path: &Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return vec![];
    };
    let Ok(value) = json5::from_str::<serde_json::Value>(content.trim_start_matches('\u{FEFF}'))
    else {
        return vec![];
    };
    ["version", "homepage", "license"]
        .iter()
        .filter(|k| value.get(**k).is_none())
        .map(|k| format!("{}: missing '{}' field", path.display(), k))
        .collect()
}

/// Serialise a JSON value to a string with 4-space indentation and CRLF endings.
///
/// Matches the Scoop convention for manifest formatting.
pub fn to_scoop_json(value: &serde_json::Value) -> Fallible<String> {
    // 1. Normalize values the way official `ConvertToPrettyJson` does
    //    (single-element array -> scalar, multi-line string -> array, ...).
    let mut value = value.clone();
    normalize_values(&mut value);

    // 2. Serialize with 4-space indentation.
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    value
        .serialize(&mut ser)
        .map_err(|e| Error::Custom(format!("serialize error: {}", e)))?;
    let mut formatted =
        String::from_utf8(buf).map_err(|e| Error::Custom(format!("utf8 error: {}", e)))?;

    // 3. Convert line endings to CRLF, then expand empty objects/arrays.
    formatted = formatted.replace('\n', "\r\n");
    formatted = expand_empty_containers(&formatted);

    // 4. Ensure a trailing newline.
    if !formatted.ends_with("\r\n") {
        formatted.push_str("\r\n");
    }
    Ok(formatted)
}

/// Aligns with official `lib/json.ps1` `normalize_values`.
///
/// Recursively visits object property values:
/// - multi-line strings are split on `\r?\n` into trimmed string arrays,
/// - single-element arrays collapse to their scalar element,
/// - single-element arrays nested inside multi-element arrays are flattened.
///
/// Like the official implementation, arrays are not recursed into: strings
/// inside arrays are left untouched and nested objects inside arrays are not
/// normalized.
fn normalize_values(value: &mut serde_json::Value) {
    if let serde_json::Value::Object(map) = value {
        for v in map.values_mut() {
            normalize_value(v);
        }
    }
}

fn normalize_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(_) => normalize_values(value),
        serde_json::Value::String(s) => {
            let parts: Vec<String> = s
                .split('\n')
                .map(|line| line.trim_end_matches('\r').trim().to_string())
                .collect();
            if parts.len() > 1 {
                *value = serde_json::Value::Array(
                    parts.into_iter().map(serde_json::Value::String).collect(),
                );
            }
        }
        serde_json::Value::Array(arr) => {
            if arr.len() == 1 {
                // Single-element array whose element is not itself an array:
                // collapse to the scalar element.
                if !arr[0].is_array() {
                    *value = arr.remove(0);
                }
            } else if !arr.is_empty() {
                // Multi-element array: flatten single-element inner arrays.
                let mut result: Vec<serde_json::Value> = Vec::with_capacity(arr.len());
                for elem in arr.drain(..) {
                    if let serde_json::Value::Array(mut inner) = elem {
                        if inner.len() == 1 {
                            result.append(&mut inner);
                        } else {
                            result.push(serde_json::Value::Array(inner));
                        }
                    } else {
                        result.push(elem);
                    }
                }
                *arr = result;
            }
        }
        _ => {}
    }
}

/// Expands empty objects/arrays to the official multi-line layout:
///
/// ```text
/// "a": {
///
/// },
/// ```
///
/// `serde_json`'s pretty printer emits `{}` / `[]` compactly; the official
/// `ConvertToPrettyJson` character formatter outputs them across lines.
/// Because pretty output only ever places `{` and `}` adjacent for empty
/// containers, a simple scan (skipping string literals) is safe.
fn expand_empty_containers(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len() + 64);
    let mut depth = 0usize;
    let mut in_string = false;
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '{' | '[' => {
                out.push(c);
                let close = if c == '{' { '}' } else { ']' };
                if i + 1 < chars.len() && chars[i + 1] == close {
                    // Empty container: expand across lines with the official layout.
                    out.push_str("\r\n");
                    out.push_str(&"    ".repeat(depth + 1));
                    out.push_str("\r\n");
                    out.push_str(&"    ".repeat(depth));
                    out.push(close);
                    i += 2;
                    continue;
                }
                depth += 1;
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                out.push(c);
            }
            _ => out.push(c),
        }
        i += 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── normalize_values ─────────────────────────────────────────────────────

    #[test]
    fn collapses_single_element_array_to_scalar() {
        let v = serde_json::json!({"url": ["https://example.com/app.zip"]});
        let output = to_scoop_json(&v).unwrap();
        assert!(
            output.contains("\"url\": \"https://example.com/app.zip\""),
            "output: {output}"
        );
    }

    #[test]
    fn keeps_multi_element_array() {
        let v = serde_json::json!({"url": ["https://a", "https://b"]});
        let output = to_scoop_json(&v).unwrap();
        assert!(
            output
                .contains("\"url\": [\r\n        \"https://a\",\r\n        \"https://b\"\r\n    ]"),
            "output: {output}"
        );
    }

    #[test]
    fn splits_multiline_string_into_array() {
        let v = serde_json::json!({"notes": "first line\r\n  second line  "});
        let output = to_scoop_json(&v).unwrap();
        assert!(
            output.contains(
                "\"notes\": [\r\n        \"first line\",\r\n        \"second line\"\r\n    ]"
            ),
            "output: {output}"
        );
    }

    #[test]
    fn flattens_single_element_nested_arrays() {
        // Multi-element inner arrays are kept, single-element inner arrays flatten.
        let v = serde_json::json!({"bin": [["a.exe", "a"], ["b"]]});
        let output = to_scoop_json(&v).unwrap();
        assert!(
            output.contains("\"bin\": [\r\n        [\r\n            \"a.exe\",\r\n            \"a\"\r\n        ],\r\n        \"b\"\r\n    ]"),
            "output: {output}"
        );
    }

    #[test]
    fn does_not_recurse_into_array_elements() {
        // Official behavior: strings inside arrays are not split on newlines.
        let v = serde_json::json!({"arr": [{"a": "x\ny"}, {"a": "z"}]});
        let output = to_scoop_json(&v).unwrap();
        assert!(output.contains("\"a\": \"x\\ny\""), "output: {output}");
    }

    // ── expand_empty_containers ──────────────────────────────────────────────

    #[test]
    fn expands_empty_object() {
        let v = serde_json::json!({"a": {}});
        let output = to_scoop_json(&v).unwrap();
        assert!(
            output.contains("\"a\": {\r\n        \r\n    }"),
            "output: {output}"
        );
    }

    #[test]
    fn expands_empty_array() {
        let v = serde_json::json!({"b": []});
        let output = to_scoop_json(&v).unwrap();
        assert!(
            output.contains("\"b\": [\r\n        \r\n    ]"),
            "output: {output}"
        );
    }

    #[test]
    fn expands_root_empty_container() {
        let output = to_scoop_json(&serde_json::Value::Object(Default::default())).unwrap();
        assert_eq!(output, "{\r\n    \r\n}\r\n");
    }

    #[test]
    fn keeps_empty_braces_inside_strings() {
        let v = serde_json::json!({"a": "{}", "b": "[]"});
        let output = to_scoop_json(&v).unwrap();
        assert!(output.contains("\"a\": \"{}\""), "output: {output}");
        assert!(output.contains("\"b\": \"[]\""), "output: {output}");
    }

    // ── format_manifest_file ─────────────────────────────────────────────────

    #[test]
    fn accepts_non_strict_syntax_and_outputs_strict_json() {
        let dir = crate::test_utils::tmpdir("formatjson_nonstrict");
        let path = dir.join("app.json");
        // Trailing comma + comment: valid JSON5, invalid strict JSON.
        std::fs::write(&path, "{\"a\": 1,} // comment").unwrap();
        assert!(format_manifest_file(&path).unwrap());
        let out = std::fs::read_to_string(&path).unwrap();
        // Non-standard input in, standard strict JSON out.
        assert_eq!(out, "{\r\n    \"a\": 1\r\n}\r\n");
    }

    #[test]
    fn preserves_key_order_with_non_strict_input() {
        let dir = crate::test_utils::tmpdir("formatjson_order");
        let path = dir.join("app.json");
        // Comment + trailing comma input; keys deliberately out of alphabetical order.
        std::fs::write(&path, "{\"z\": 1, \"a\": 2, // keep me\n \"m\": 3,}").unwrap();
        assert!(format_manifest_file(&path).unwrap());
        let out = std::fs::read_to_string(&path).unwrap();
        let z = out.find("\"z\": 1").expect("key z present");
        let a = out.find("\"a\": 2").expect("key a present");
        let m = out.find("\"m\": 3").expect("key m present");
        assert!(z < a && a < m, "key order must be preserved:\n{out}");
        // Output is strict JSON: comment stripped, trailing comma removed.
        assert!(!out.contains("keep me"), "comment must be stripped:\n{out}");
        assert!(
            !out.contains(",}"),
            "trailing comma must be removed:\n{out}"
        );
    }

    #[test]
    fn formats_strict_json_file() {
        let dir = crate::test_utils::tmpdir("formatjson_strict_ok");
        let path = dir.join("app.json");
        std::fs::write(&path, "{\"a\":[\"x\"]}").unwrap();
        assert!(format_manifest_file(&path).unwrap());
        let out = std::fs::read_to_string(&path).unwrap();
        assert_eq!(out, "{\r\n    \"a\": \"x\"\r\n}\r\n");
    }

    // ── format_manifests ──────────────────────────────────────────────────────────

    #[test]
    fn format_manifests_batch_formats_all_with_wildcard() {
        let dir = crate::test_utils::tmpdir("formatjson_dir_wildcard");
        std::fs::write(dir.join("app1.json"), "{\"a\":[\"x\"]}").unwrap();
        std::fs::write(dir.join("app2.json"), "{\"b\":[\"y\"]}").unwrap();

        let report = format_manifests(&dir, &["*".to_string()]).unwrap();
        assert_eq!(report.formatted, 2);
        assert_eq!(report.unchanged, 0);
        assert!(report.errors.is_empty());

        // Idempotent second pass: nothing to rewrite.
        let report = format_manifests(&dir, &["*".to_string()]).unwrap();
        assert_eq!(report.formatted, 0);
        assert_eq!(report.unchanged, 2);
    }

    #[test]
    fn format_manifests_exact_pattern_does_not_substring_match() {
        let dir = crate::test_utils::tmpdir("formatjson_dir_filter");
        std::fs::write(dir.join("app1.json"), "{\"a\":[\"x\"]}").unwrap();
        std::fs::write(dir.join("app2.json"), "{\"b\":[\"y\"]}").unwrap();

        // Plain pattern matches exactly: only app1 is formatted.
        let report = format_manifests(&dir, &["app1".to_string()]).unwrap();
        assert_eq!(report.formatted, 1);
        assert_eq!(report.unchanged, 0);
    }

    #[test]
    fn missing_required_fields_reported_as_warnings() {
        let dir = crate::test_utils::tmpdir("formatjson_missing_fields");
        // No version/homepage/license at all.
        std::fs::write(
            dir.join("app1.json"),
            "{\"url\":\"https://example.com/x.zip\",\"hash\":\"a\"}",
        )
        .unwrap();
        // Complete manifest → no warnings.
        std::fs::write(
            dir.join("app2.json"),
            "{\"version\":\"1.0\",\"homepage\":\"https://example.com\",\"license\":\"MIT\",\"url\":\"https://example.com/x.zip\",\"hash\":\"a\"}",
        )
        .unwrap();

        let report = format_manifests(&dir, &["*".to_string()]).unwrap();
        // app1 is missing all three required fields → one warning each.
        assert_eq!(report.warnings.len(), 3, "one warning per missing field");
        let joined = report.warnings.join("\n");
        assert!(joined.contains("version"), "{joined}");
        assert!(joined.contains("homepage"), "{joined}");
        assert!(joined.contains("license"), "{joined}");
        assert!(!joined.contains("app2"), "{joined}");
    }

    #[test]
    fn format_manifests_skips_package_json_and_collects_errors() {
        let dir = crate::test_utils::tmpdir("formatjson_dir_errors");
        std::fs::write(dir.join("app.json"), "{\"a\":[\"x\"]}").unwrap();
        std::fs::write(dir.join("package.json"), "{\"name\":\"x\"}").unwrap();
        // Invalid JSON5: parse failure is collected, not fatal.
        std::fs::write(dir.join("bad.json"), "{ not json").unwrap();

        let report = format_manifests(&dir, &["*".to_string()]).unwrap();
        assert_eq!(report.formatted, 1, "app.json formatted");
        assert_eq!(report.errors.len(), 1, "bad.json collected as error");
        assert!(
            !report.errors[0].contains("package.json"),
            "package.json must be skipped"
        );
    }
}
