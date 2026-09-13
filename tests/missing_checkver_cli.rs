//! `hok missing-checkver` — the `[C]heckver | [A]utoupdate` truth table.
//!
//! Mirrors official `bin/missing-checkver.ps1`: every matching manifest gets a
//! row (a parse failure too, rendered `[ ][ ]` plus the official warning), and
//! `-SkipSupported` narrows the listing to the manifests that still miss one of
//! the two properties. The old `-s/--supported` had the inverse meaning.

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
        let base = std::env::temp_dir().join(format!("hok_missing_checkver_{name}"));
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

    fn dir(&self) -> &Path {
        &self.bucket
    }

    /// Run `hok missing-checkver <args...>` in this environment; returns
    /// `(code, stdout+stderr)` with colour disabled.
    fn run(&self, args: &[&str]) -> (i32, String) {
        let dir = self.bucket.to_str().expect("utf-8 bucket path");
        let out = Command::new(env!("CARGO_BIN_EXE_hok"))
            .arg("missing-checkver")
            .args(args)
            .arg("-d")
            .arg(dir)
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
}

const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// A minimal valid manifest plus extra top-level fields.
fn manifest(extra: &str) -> String {
    format!(
        r#"{{
    "version": "1.0",
    "homepage": "https://example.com",
    "license": "MIT",
    "url": "https://example.com/app.zip",
    "hash": "{HASH}"{extra}
}}
"#
    )
}

/// The three states, as the truth-table fixtures.
fn write_fixtures(env: &Env) {
    env.write(
        "both.json",
        &manifest(r#","checkver": "github", "autoupdate": {"url": "https://e.com/$version"}"#),
    );
    env.write("neither.json", &manifest(""));
    env.write("onlyver.json", &manifest(r#","checkver": "github""#));
}

/// Every matching manifest gets a row, present properties are letters and
/// absent ones are blanks, and the summary counts both gaps plus the parse
/// failures.
#[test]
fn default_lists_every_manifest_as_a_truth_table_row() {
    let env = Env::new("table");
    write_fixtures(&env);
    env.write("bad.json", "{ not json");

    let (code, text) = env.run(&[]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("[C]heckver"), "legend missing: {text}");
    assert!(text.contains("[A]utoupdate"), "legend missing: {text}");

    assert!(text.contains("[C][A] both"), "both.json row: {text}");
    assert!(text.contains("[ ][ ] neither"), "neither.json row: {text}");
    assert!(text.contains("[C][ ] onlyver"), "onlyver.json row: {text}");
    assert!(
        text.contains("[ ][ ] bad"),
        "unparsable manifest must still get a row: {text}"
    );
    assert!(
        text.contains("Error parsing JSON at '"),
        "the official parse warning is expected: {text}"
    );

    // Four rows: three parsed + the failure. The failure is counted on its own,
    // never as "missing checkver".
    assert!(
        text.contains(
            "Scanned 3 manifests: 1 missing checkver, 2 missing autoupdate, 1 unparsable."
        ),
        "summary line: {text}"
    );
}

/// `-SkipSupported` drops exactly the manifests that have both properties.
#[test]
fn skip_supported_drops_only_fully_supported_manifests() {
    let env = Env::new("skip");
    write_fixtures(&env);

    let (code, text) = env.run(&["-s"]);
    assert_eq!(code, 0, "{text}");
    assert!(!text.contains("both"), "-s must skip both.json: {text}");
    assert!(text.contains("[ ][ ] neither"), "{text}");
    assert!(text.contains("[C][ ] onlyver"), "{text}");
}

/// When every row is either supported or absent, the summary says so.
#[test]
fn a_fully_supported_bucket_reports_no_gaps() {
    let env = Env::new("all_ok");
    env.write(
        "both.json",
        &manifest(r#","checkver": "github", "autoupdate": {"url": "https://e.com/$version"}"#),
    );

    let (code, text) = env.run(&[]);
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains(
            "Scanned 1 manifests: 0 missing checkver, 0 missing autoupdate, 0 unparsable."
        ),
        "{text}"
    );
    assert!(
        text.contains("All manifests have checkver and autoupdate."),
        "{text}"
    );
}

/// The app filter mirrors official `-App` / `-Filter "$App.json"`: a plain
/// name, an explicit `.json` suffix and a different case all select the same
/// manifest.
#[test]
fn app_filter_accepts_names_suffixes_and_case() {
    let env = Env::new("filter");
    write_fixtures(&env);

    for app in [
        "onlyver",
        "onlyver.json",
        "OnlyVer",
        "OnlyVer.JSON",
        "only*",
    ] {
        let (code, text) = env.run(&[app]);
        assert_eq!(code, 0, "`missing-checkver {app}` failed: {text}");
        assert!(
            text.contains("onlyver"),
            "`{app}` did not select it: {text}"
        );
        assert!(
            !text.contains("neither"),
            "`{app}` matched too much: {text}"
        );
    }
}

/// The app filter is upstream's positional `-App` parameter, exactly like the
/// sibling bucket commands — there is no `-a/--app` option, and `-a` must stay
/// a usage error rather than being silently ignored.
#[test]
fn the_app_filter_is_positional_not_a_flag() {
    let env = Env::new("positional");
    write_fixtures(&env);

    let (code, text) = env.run(&["-a", "onlyver"]);
    assert_eq!(code, 1, "`-a` must not be accepted: {text}");
    assert!(
        text.contains("[APP]"),
        "usage should show the positional: {text}"
    );
}

/// A missing directory is reported through the shared guard instead of
/// producing an empty table.
#[test]
fn a_missing_directory_is_an_error() {
    let env = Env::new("nodir");
    let missing = env.dir().join("nope");
    let out = Command::new(env!("CARGO_BIN_EXE_hok"))
        .args(["missing-checkver", "-d"])
        .arg(&missing)
        .args(["--lang", "en", "--no-color"])
        .env("SCOOP", env.bucket.parent().expect("scoop root"))
        .env("USERPROFILE", &env.home)
        .env("HOME", &env.home)
        .env("APPDATA", env.home.join("AppData"))
        .env("XDG_CONFIG_HOME", env.home.join(".config"))
        .env_remove("HOK_LOG_LEVEL")
        .output()
        .expect("run hok");
    assert_ne!(out.status.code().unwrap_or(-1), 0);
}
