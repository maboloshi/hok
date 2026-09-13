//! `hok formatjson` — the CLI surface and the in-place rewrite contract.
//!
//! Guards the subcommand name first: upstream's script is `bin/formatjson.ps1`,
//! and the enum variant `FormatJson` used to be clap-kebab-cased into
//! `format-json`, so `hok formatjson` was rejected as an unknown subcommand.
//! Both spellings must keep working (`format-json` stays as an alias).
//!
//! The rewrite assertions mirror official `bin/formatjson.ps1`: 4-space
//! indentation, a re-parsed manifest, and — unlike the official script, which
//! truncates an unparsable file to an empty line — an unparsable manifest left
//! exactly as it was, reported instead of destroyed.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A throwaway Scoop root plus an isolated home, so the binary never reads the
/// real installation or the user's `config.json`.
struct Env {
    bucket: PathBuf,
    home: PathBuf,
}

impl Env {
    fn new(name: &str) -> Env {
        let base = std::env::temp_dir().join(format!("hok_formatjson_{name}"));
        let _ = std::fs::remove_dir_all(&base);
        let bucket = base.join("bucket");
        let home = base.join("home");
        std::fs::create_dir_all(&bucket).expect("create bucket dir");
        std::fs::create_dir_all(&home).expect("create home dir");
        Env { bucket, home }
    }

    fn write(&self, name: &str, content: &str) {
        std::fs::write(self.bucket.join(name), content).expect("write fixture");
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.bucket.join(name)).expect("read fixture")
    }

    /// Run `hok <args...>` in this environment; returns `(code, stdout+stderr)`.
    fn run(&self, args: &[&str]) -> (i32, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_hok"))
            .args(args)
            .arg("--lang")
            .arg("en")
            .arg("--no-color")
            .env("SCOOP", self.bucket.parent().expect("scoop root"))
            .env("USERPROFILE", &self.home)
            .env("HOME", &self.home)
            .env("APPDATA", self.home.join("AppData"))
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env_remove("HOK_LOG_LEVEL")
            .output()
            .expect("run hok");
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        (out.status.code().unwrap_or(-1), text)
    }

    fn dir(&self) -> &Path {
        &self.bucket
    }
}

/// A minimal valid manifest with a deliberately non-Scoop layout.
const COMPACT: &str = r#"{"version":"1.0","homepage":"https://example.com","license":"MIT","url":["https://example.com/app.zip"],"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#;

/// `hok formatjson` and its `format-json` alias must both resolve, and an
/// unknown neighbour must still be rejected (so the fix cannot be a blanket
/// "accept anything").
#[test]
fn both_spellings_resolve_and_unknown_ones_do_not() {
    let env = Env::new("names");
    env.write("app.json", COMPACT);
    let dir = env.dir().to_str().unwrap().to_string();

    for name in ["formatjson", "format-json"] {
        let (code, text) = env.run(&[name, "-d", &dir]);
        assert_eq!(code, 0, "`hok {name}` should succeed, got: {text}");
        assert!(
            !text.contains("unrecognized subcommand"),
            "`hok {name}` was rejected: {text}"
        );
    }

    let (code, text) = env.run(&["format-jsonn", "-d", &dir]);
    assert_eq!(code, 1, "a typo must stay a usage error: {text}");
}

/// The new name leads the command list; the alias stays discoverable.
#[test]
fn the_canonical_name_is_shown_in_help() {
    let env = Env::new("help");
    let (code, text) = env.run(&["--help"]);
    assert_eq!(code, 0);
    assert!(
        text.contains("  formatjson  "),
        "canonical name missing from the command list: {text}"
    );
}

/// A compact manifest is rewritten with the Scoop layout, and a second run is
/// a no-op (idempotent).
#[test]
fn formats_in_place_and_is_idempotent() {
    let env = Env::new("format");
    env.write("app.json", COMPACT);
    let dir = env.dir().to_str().unwrap().to_string();

    let (code, _) = env.run(&["formatjson", "-d", &dir]);
    assert_eq!(code, 0);
    let formatted = env.read("app.json");
    assert!(formatted.contains("\r\n"), "CRLF expected: {formatted:?}");
    assert!(
        formatted.contains("\n    \"version\": \"1.0\""),
        "{formatted:?}"
    );
    // A single-element array collapses to its scalar (official
    // `ConvertToPrettyJson` / `normalize_values`).
    assert!(
        formatted.contains("\"url\": \"https://example.com/app.zip\""),
        "{formatted:?}"
    );

    let (code, text) = env.run(&["formatjson", "-d", &dir]);
    assert_eq!(code, 0);
    assert_eq!(
        env.read("app.json"),
        formatted,
        "second run rewrote the file"
    );
    assert!(text.contains("No manifests needed formatting."), "{text}");
}

/// An unparsable manifest is reported and left byte-identical: the official
/// script would truncate it (`$null` piped into `WriteAllLines`), which is the
/// destructive behavior hok deliberately does not copy.
#[test]
fn unparsable_manifest_is_reported_not_truncated() {
    let env = Env::new("unparsable");
    let broken = "{ not json";
    env.write("bad.json", broken);
    let dir = env.dir().to_str().unwrap().to_string();

    let (code, text) = env.run(&["formatjson", "-d", &dir]);
    assert_eq!(
        code, 0,
        "a per-file failure must not abort the batch: {text}"
    );
    assert!(text.contains("bad.json"), "failure not reported: {text}");
    assert_eq!(env.read("bad.json"), broken, "unparsable file was modified");
}

/// App filtering mirrors the official `-Filter "$App.json"`: a plain name, a
/// wildcard, an explicit `.json` suffix and a different case all select the
/// same single manifest.
#[test]
fn app_filter_is_case_insensitive_and_ignores_the_json_suffix() {
    let env = Env::new("filter");
    env.write("curl.json", COMPACT);
    env.write("wget.json", COMPACT);
    let dir = env.dir().to_str().unwrap().to_string();

    for app in ["curl", "Curl", "curl.json", "Curl.JSON", "cur*"] {
        env.write("curl.json", COMPACT);
        env.write("wget.json", COMPACT);
        let (code, text) = env.run(&["formatjson", app, "-d", &dir]);
        assert_eq!(code, 0, "`formatjson {app}` failed: {text}");
        assert!(
            env.read("curl.json") != COMPACT,
            "`{app}` did not select curl.json"
        );
        assert_eq!(
            env.read("wget.json"),
            COMPACT,
            "`{app}` wrongly selected wget.json"
        );
    }
}
