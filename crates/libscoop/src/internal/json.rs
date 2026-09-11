//! Tolerant JSON parsing — the single entry point for JSON text.
//!
//! Every read of a user-facing or bucket file in `libscoop` goes through
//! [`parse_str`], mirroring upstream Scoop, where every such read funnels
//! through PowerShell's `ConvertFrom-Json` and only the *error policy*
//! differs per call site:
//!
//! | upstream call site | upstream error policy | hok |
//! |---|---|---|
//! | `lib/manifest.ps1:5-12` `parse_json` | `warn` + `$null` | [`parse_str`] → `Err` |
//! | `lib/manifest.ps1:14-32` `url_manifest` | `warn` + `$null` | [`parse_str`] → `Err` |
//! | `lib/core.ps1:82-95` `load_cfg` | `Write-Host`, no value | [`parse_str`] → `Err` |
//! | `lib/json.ps1:130`, `lib/buckets.ps1:40` | throws | [`parse_str`] → `Err` |
//! | `libexec/scoop-search.ps1:96` | `-ErrorAction Continue` | — (no counterpart) |
//! | `lib/database.ps1:175` | `-ErrorAction SilentlyContinue` | — (no counterpart) |
//! | — (no counterpart) | — | [`parse_str_or_none`] (hok diagnostics only) |
//!
//! Upstream never surfaces a *manifest* parse error: it warns and passes
//! `$null` on, which callers report as "Couldn't find manifest". hok returns
//! the parse error instead, which names the real problem. That is a
//! deliberate deviation under the project's "better than Scoop" rule, not an
//! accidental one — it is registered, not silently pending.
//!
//! # The chain
//!
//! 1. [`strip_bom`] — notepad's "UTF-8 with BOM" (upstream strips it while
//!    reading, so its parser never sees one).
//! 2. strict `serde_json` — the fast path, and the error reported when both
//!    parsers fail: it points at the real problem (a type mismatch, a
//!    syntax error), unlike a JSON5 error.
//! 3. `json5` as the fallback — additionally accepts comments, trailing
//!    commas, single-quoted strings and unquoted keys.
//!
//! This module is about *parsing* only. Reading and byte-decoding stay with
//! the callers because upstream's read APIs differ per call site and carry
//! different encoding policies: `[System.IO.File]::ReadAllLines`
//! (`lib/core.ps1:90`, auto-detects the encoding, substitutes invalid
//! bytes) versus `Get-Content -Encoding UTF8` (`lib/manifest.ps1:8`,
//! UTF-8 only) versus `[System.IO.File]::ReadAllText`
//! (`libexec/scoop-search.ps1:96`). Folding I/O in here would silently pick
//! one policy for all three.
//!
//! Two caller-level nuances, neither of which changes the chain:
//! `Config::load` ignores a read error and falls back to Scoop's file (so an
//! unparseable hok config surfaces as "config not found"), and
//! `formatjson::schema_warnings` is the best-effort site.
//!
//! # Who uses it
//!
//! - the config (`config::read_json_file`);
//! - manifests, both as `ManifestSpec` (`Manifest::parse`/`from_json`/
//!   `parse_str`) and as a raw `Value` for order-preserving rewrites
//!   (`formatjson`, `checkhashes`, `checkver::apply_autoupdate`, `auto_pr`);
//! - the `import` export file (`import::parse_import_json`);
//! - child-process output lines, for the BOM only (`psengine`, `script`).
//!
//! Reads that intentionally stay on plain `serde_json`: remote payloads
//! (`internal::github`, checkver/checkver-hash page content) and
//! machine-generated state (`install.json`, SQLite rows). Those producers are
//! not hand-edited, so a JSON5 fallback would turn a broken producer into a
//! quietly different value instead of an error.
//!
//! # Tolerance contract
//!
//! Upstream's tolerance is whatever the host's `ConvertFrom-Json` accepts,
//! and that is not a stable contract. Measured 2026-09-12 with
//! `powershell.exe` 5.1.26100 (Scoop's original host) and `pwsh` 7.6.5:
//!
//! | input | PS 5.1 | PS 7.6.5 | hok |
//! |---|---|---|---|
//! | `{"a":1}` | OK | OK | OK |
//! | `{"a":1,}` | FAIL | OK | OK |
//! | `{"a":1 // c}` (no line break) | FAIL | FAIL | FAIL |
//! | `{"a":1 // c\n}` | FAIL | OK | OK |
//! | `{"a":1 /* c */}` | FAIL | OK | OK |
//! | `{'a':1}` | OK | OK | OK |
//! | `{a:1}` | OK | OK | OK |
//! | `{"a":01}` | OK | OK | **FAIL** |
//! | `[1,2,]` | FAIL | OK | OK |
//! | `{"a":1,"a":2}` | OK | OK | OK |
//! | `{ not json` | FAIL | FAIL | FAIL |
//!
//! So hok is **not** a superset of either host: it is looser on comments,
//! trailing commas and unquoted keys (which 5.1 rejects), and *stricter* on
//! leading zeros (JSON5 — like JSON — forbids them, while both hosts accept
//! them). The hok column is pinned by the `tolerance_contract` test below;
//! the host columns are recorded ground truth only, since they depend on
//! the machine.

use crate::error::Fallible;
use crate::internal::string::strip_bom;
use serde::de::DeserializeOwned;

/// Parse `text` as JSON with the shared tolerance chain (see module docs).
///
/// A leading UTF-8 BOM is stripped, strict `serde_json` is tried first, and
/// `json5` is the fallback. When both fail, the strict error is returned.
pub fn parse_str<T: DeserializeOwned>(text: &str) -> Fallible<T> {
    let text = strip_bom(text);
    match serde_json::from_str::<T>(text) {
        Ok(value) => Ok(value),
        Err(strict_err) => {
            json5::from_str::<T>(text).map_err(|_| crate::Error::Custom(strict_err.to_string()))
        }
    }
}

/// Best-effort [`parse_str`]: `None` when the text does not parse.
///
/// The only caller is `formatjson::schema_warnings`, which reports
/// unparseable manifests as "no warnings" instead of failing a whole batch
/// (upstream has no counterpart: its `formatjson` reads through
/// `parse_json`, and a `$null` result then fails binding to
/// `ConvertToPrettyJson`). Every error-swallowing JSON read is this
/// function, so `parse_str_or_none` finds them all.
pub fn parse_str_or_none<T: DeserializeOwned>(text: &str) -> Option<T> {
    parse_str(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── the chain ────────────────────────────────────────────────────────────

    #[test]
    fn strict_json_parses() {
        let value: serde_json::Value = parse_str(r#"{"a":1}"#).unwrap();
        assert_eq!(value["a"], 1);
    }

    #[test]
    fn bom_prefixed_text_parses() {
        let value: serde_json::Value = parse_str("\u{FEFF}{\"a\":1}").unwrap();
        assert_eq!(value["a"], 1);
    }

    #[test]
    fn json5_syntax_parses_through_the_fallback() {
        for text in [
            r#"{"a":1,}"#,
            "{\"a\":1 // c\n}",
            r#"{"a":1 /* c */}"#,
            "{'a':1}",
            "{a:1}",
            "[1,2,]",
        ] {
            assert!(parse_str::<serde_json::Value>(text).is_ok(), "{text}");
        }
    }

    #[test]
    fn unparseable_text_reports_the_strict_error() {
        // The reported error comes from `serde_json`, not from JSON5: it
        // names the real problem (here: a non-string key), unlike a JSON5
        // pest error, which only says where the grammar gave up.
        let err = parse_str::<serde_json::Value>("{ not json").unwrap_err();
        let strict = serde_json::from_str::<serde_json::Value>("{ not json").unwrap_err();
        assert_eq!(err.to_string(), strict.to_string());
    }

    #[test]
    fn best_effort_variant_swallows_the_error() {
        assert!(parse_str_or_none::<serde_json::Value>("{ not json").is_none());
        assert!(parse_str_or_none::<serde_json::Value>(r#"{"a":1,}"#).is_some());
    }

    // ── the contract ─────────────────────────────────────────────────────────

    /// Pin hok's tolerance so it cannot drift unnoticed (module docs hold the
    /// full table and the measured PowerShell columns).
    ///
    /// The columns are `(input, hok, PS 5.1, PS 7.6.5)`; the host columns are
    /// today's recorded ground truth for the same inputs and are deliberately
    /// **not** asserted — they describe the machine, not hok.
    #[test]
    fn tolerance_contract() {
        let cases: [(&str, bool, bool, bool); 11] = [
            (r#"{"a":1}"#, true, true, true),
            // Looser than PS 5.1: JSON5 comments, trailing commas, bare keys.
            (r#"{"a":1,}"#, true, false, true),
            ("{\"a\":1 // c}", false, false, false),
            ("{\"a\":1 // c\n}", true, false, true),
            (r#"{"a":1 /* c */}"#, true, false, true),
            ("{'a':1}", true, true, true),
            ("{a:1}", true, true, true),
            (r#"{"a":1,"a":2}"#, true, true, true),
            ("[1,2,]", true, false, true),
            // Stricter than *both* hosts: JSON5, like JSON, forbids leading
            // zeros. The one case where hok rejects what upstream accepts.
            (r#"{"a":01}"#, false, true, true),
            ("{ not json", false, false, false),
        ];

        for (input, hok_accepts, ps51_accepts, ps7_accepts) in cases {
            assert_eq!(
                parse_str::<serde_json::Value>(input).is_ok(),
                hok_accepts,
                "hok verdict for {input:?} (upstream: PS 5.1 {ps51_accepts}, PS 7.6.5 {ps7_accepts})"
            );
        }
    }
}
