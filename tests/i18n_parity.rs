//! Rendering parity with the pre-migration `rust-i18n` backend (optimisation
//! audit item 4).
//!
//! `tests/data/i18n_reference.tsv` is a golden dump produced by the **old**
//! implementation, before `rust-i18n` was removed. For every key of `en.yml`
//! and `zh.yml` it records two renderings per locale:
//!
//! 1. `plain`  — the key without interpolation args;
//! 2. `probed` — the key with **every** `%{name}` placeholder used anywhere in
//!    the locale files bound to `«name»`, so the interpolation path is
//!    exercised for each placeholder name.
//!
//! Six locales were captured, chosen to pin the lookup rules rather than just
//! the happy path:
//!
//! | locale | expected behaviour |
//! |---|---|
//! | `en`, `zh` | the two shipped tables |
//! | `en-US`, `zh-CN` | subtag truncation falls back to `en` / `zh` |
//! | `fr` | unknown locale — every key renders as the key itself |
//! | `""` | empty locale — same as unknown |
//!
//! The fixture also locks in `serde_yaml`'s reading of the tricky corners of
//! the locale files: `|-` block scalars (kept with real newlines), `\n`/`\\`
//! escapes, and flow sequences (`cli_alias.search: [s]`), which the previous
//! implementation flattened to the empty string.
//!
//! **How the fixture was generated.** It came from a one-shot generator that
//! lived inside the `hok` crate (it had to: `rust_i18n::t!` expands to
//! `crate::_rust_i18n_t!`, which only exists in the crate that called
//! `rust_i18n::i18n!`, so `tests/` could not reach it). That generator scanned
//! the leaf keys out of the YAML with the same scanner `tests/i18n_keys.rs`
//! uses, called `rust_i18n::set_locale(locale)` followed by `rust_i18n::t!(key)`
//! for each key, and formatted the result with `{:?}`. It is deliberately not
//! kept: once `rust-i18n` is gone it cannot compile, and the fixture plus this
//! test are what remain of the comparison.
//!
//! Run explicitly: `cargo test -p hok --test i18n_parity -- --ignored`.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

/// `hok_i18n`'s selected locale is process-wide and both tests below move it;
/// they take turns instead of racing when run together (`--include-ignored`).
static LOCALE_LOCK: Mutex<()> = Mutex::new(());

fn lock_locale() -> MutexGuard<'static, ()> {
    LOCALE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Placeholder names bound by the `probed` column, in fixture order.
const PROBE_NAMES: &[&str] = &[
    "actual",
    "apps",
    "broken",
    "command",
    "count",
    "ctx",
    "deps",
    "detail",
    "e",
    "expected",
    "failed",
    "file",
    "filename",
    "files",
    "from",
    "held",
    "invalid",
    "isolated",
    "key",
    "missing",
    "msg",
    "n",
    "name",
    "names",
    "noauto",
    "object",
    "orphaned",
    "passed",
    "path",
    "pattern",
    "pkg",
    "query",
    "reason",
    "ref",
    "source",
    "to",
    "total",
    "updatable",
    "updated",
    "upgradable",
    "url",
    "valid",
    "value",
    "ver",
    "verb",
    "version",
    "versions",
];

fn probe(name: &str) -> String {
    format!("«{name}»")
}

/// A rendering mismatch, reported with enough context to be actionable.
struct Mismatch {
    locale: String,
    key: String,
    column: &'static str,
    expected: String,
    actual: String,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{}/{}: expected {}, got {}",
            self.locale, self.key, self.column, self.expected, self.actual
        )
    }
}

#[test]
#[ignore = "parity harness; run with --ignored to re-verify against the reference dump"]
fn rendered_output_matches_pre_migration_reference() {
    let _guard = lock_locale();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/i18n_reference.tsv");
    let reference = fs::read_to_string(&fixture)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", fixture.display()));

    assert_eq!(
        hok_i18n::available_locales().collect::<Vec<_>>(),
        vec!["en", "zh"],
        "the reference was captured against exactly these tables"
    );

    let names: Vec<&str> = PROBE_NAMES.to_vec();
    let values: Vec<String> = PROBE_NAMES.iter().map(|name| probe(name)).collect();

    let mut mismatches: Vec<Mismatch> = Vec::new();
    let mut records = 0usize;
    for line in reference.lines() {
        let mut fields = line.split('\t');
        let (Some(locale), Some(key), Some(plain), Some(probed)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            panic!("malformed fixture line: {line}");
        };
        records += 1;

        hok_i18n::set_locale(locale);
        let actual_plain = format!("{:?}", hok_i18n::t!(key));
        let actual_probed = format!("{:?}", hok_i18n::translate_with(key, &names, &values));

        for (column, expected, actual) in [
            ("plain", plain, actual_plain),
            ("probed", probed, actual_probed),
        ] {
            if expected != actual {
                mismatches.push(Mismatch {
                    locale: locale.to_string(),
                    key: key.to_string(),
                    column,
                    expected: expected.to_string(),
                    actual,
                });
            }
        }
    }

    assert!(records > 0, "the reference fixture is empty");

    if !mismatches.is_empty() {
        let mut report = String::new();
        writeln!(
            report,
            "{} of {} renderings differ from the pre-migration reference:",
            mismatches.len(),
            records * 2
        )
        .expect("write to String");
        for mismatch in mismatches.iter().take(20) {
            writeln!(report, "  {mismatch}").expect("write to String");
        }
        if mismatches.len() > 20 {
            writeln!(report, "  ... and {} more", mismatches.len() - 20).expect("write to String");
        }
        panic!("{report}");
    }

    println!("{records} recorded renderings reproduced exactly");
}

/// The forms `t!` has to support at call sites, checked against the real tables.
#[test]
fn t_macro_supports_every_call_shape_used_in_the_tree() {
    let _guard = lock_locale();
    hok_i18n::set_locale("en");

    // Literal key, no interpolation.
    let literal = hok_i18n::t!("output.done");
    assert!(
        !literal.is_empty() && literal != "output.done",
        "got {literal:?}"
    );

    // Dynamic key: `crates/hok-i18n-derive` renders clap help from runtime keys.
    let dynamic = String::from("output.done");
    assert_eq!(hok_i18n::t!(dynamic), literal);

    // Interpolated key: the placeholder is substituted, never rendered raw.
    let interpolated = hok_i18n::t!("error.package_not_found", name = "git");
    assert!(interpolated.contains("git"), "got {interpolated:?}");
    assert!(!interpolated.contains("%{name}"), "got {interpolated:?}");

    // A non-string value goes through `Display`, as `%{count}` sites require.
    let counted = hok_i18n::t!("cmd.info_more", n = 7usize);
    assert!(counted.contains('7'), "got {counted:?}");

    // String-literal placeholder name: needed for Rust keywords such as `ref`.
    let keyword = hok_i18n::t!("error.bucket_update_merge_conflict", "ref" = "main");
    assert!(keyword.contains("main"), "got {keyword:?}");

    // An unknown key renders as the key itself rather than panicking.
    assert_eq!(hok_i18n::t!("no.such.key"), "no.such.key");

    // Switching locale switches the rendered text.
    hok_i18n::set_locale("zh");
    assert_ne!(hok_i18n::t!("output.done"), literal);
    hok_i18n::set_locale("en");
}
