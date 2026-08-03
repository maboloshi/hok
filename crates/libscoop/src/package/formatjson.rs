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
use serde::Serialize;
use std::path::Path;

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
