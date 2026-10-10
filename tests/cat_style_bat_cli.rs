//! `cat_style` is not hok's own vocabulary — it is passed to `bat --style`
//! verbatim, as upstream does (`libexec/scoop-cat.ps1:21`) — so both ends of the
//! setting have to survive a `bat` that rejects the value:
//!
//! * `hok cat` must still show the manifest. It used to end with an empty
//!   screen: bat's non-zero exit was dropped, the "fall back to plain JSON"
//!   branch was dead, and only bat's bare parser error was left on stderr.
//! * `hok config cat_style <value>` must not store a value bat rejects, so the
//!   next `cat` does not have to fall back at all.
//!
//! The `bat.exe` on the isolated PATH is a copy of the hok binary itself, which
//! rejects the `bat` argument list and exits non-zero — the one failing
//! executable the test is guaranteed to have, and deterministic everywhere.

use std::path::{Path, PathBuf};
use std::process::Command;

mod common;

use common::{hok, isolate};

/// A minimal manifest; `cat` prints it as pretty JSON.
const MANIFEST: &str = r#"{
    "version": "1.0",
    "homepage": "https://example.com",
    "license": "MIT",
    "url": "https://example.com/app.zip",
    "hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
}
"#;

/// A throwaway Scoop root plus an isolated home, so the binary never reads the
/// real installation or the user's `config.json`.
struct Env {
    root: PathBuf,
    home: PathBuf,
    /// Holds the failing `bat.exe`; prepended to `PATH`.
    stub: PathBuf,
    /// An empty directory: `PATH` with no `bat.exe` anywhere.
    empty: PathBuf,
    manifest: PathBuf,
}

impl Env {
    fn new(name: &str) -> Env {
        let base = common::base("cat_style", name);
        let root = base.join("scoop");
        let home = base.join("home");
        let stub = base.join("stub");
        let empty = base.join("empty");
        for dir in [&root, &home, &stub, &empty] {
            std::fs::create_dir_all(dir).expect("create isolated dir");
        }
        std::fs::copy(env!("CARGO_BIN_EXE_hok"), stub.join("bat.exe")).expect("stage stub bat.exe");
        let manifest = base.join("app.json");
        std::fs::write(&manifest, MANIFEST).expect("write manifest fixture");
        Env {
            root,
            home,
            stub,
            empty,
            manifest,
        }
    }

    /// The config file the CLI reads: `$USERPROFILE/.config/hok/config.json`
    /// (`config::default::hok_config_path`).
    fn config_file(&self) -> PathBuf {
        self.home.join(".config").join("hok").join("config.json")
    }

    fn set_cat_style(&self, style: &str) {
        let file = self.config_file();
        std::fs::create_dir_all(file.parent().expect("config dir")).expect("create config dir");
        std::fs::write(&file, format!(r#"{{"cat_style": "{style}"}}"#)).expect("write config");
    }

    /// Run `hok <args>` with `path` replacing the inherited `PATH`.
    fn run_with_path(&self, args: &[&str], path: &str) -> (i32, String, String) {
        let mut cmd = hok();
        cmd.args(args)
            .env("PATH", path)
            .env("SCOOP", &self.root)
            .env("SCOOP_GLOBAL", self.root.join("global"));
        isolate(&mut cmd, &self.home);
        let out = cmd.output().expect("run hok");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// The stub `bat.exe` first on `PATH`.
    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let path = format!(
            "{};{}",
            self.stub.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        self.run_with_path(args, &path)
    }

    /// No `bat.exe` on `PATH` at all (`PATH` is an empty directory).
    fn run_without_bat(&self, args: &[&str]) -> (i32, String, String) {
        let path = self.empty.to_string_lossy().into_owned();
        self.run_with_path(args, &path)
    }
}

/// `cat_style` holds a value bat rejects: the manifest is still what the
/// command prints, and the warning names the setting instead of leaving only
/// bat's own parser error.
#[test]
fn cat_falls_back_to_plain_json_when_bat_rejects_the_style() {
    let env = Env::new("fallback");
    env.set_cat_style("bat");
    let manifest = env.manifest.to_str().expect("utf-8 manifest path");

    let (code, stdout, stderr) = env.run(&["cat", manifest, "--lang", "en", "--no-color"]);

    assert_eq!(code, 0, "a bad cat_style is not a fatal cat: {stderr}");
    assert!(
        stdout.contains("\"version\""),
        "the manifest must still be printed: {stdout:?}"
    );
    assert!(
        stderr.contains("cat_style"),
        "the warning must name the setting at fault: {stderr:?}"
    );
}

/// The setting is refused at write time, with bat's own diagnosis, and nothing
/// is stored.
#[test]
fn config_refuses_a_style_bat_rejects() {
    let env = Env::new("reject");

    let (code, _, stderr) = env.run(&["config", "cat_style", "bat", "--lang", "en", "--no-color"]);

    assert_eq!(code, 1, "the value is not stored: {stderr}");
    assert!(
        stderr.contains("cat_style") && stderr.contains("--style"),
        "the error must explain that cat_style is a bat --style value: {stderr:?}"
    );
    // The first run may create the config file (the one-time migration from
    // Scoop's); what must not be there is the refused setting.
    let stored = std::fs::read_to_string(env.config_file()).unwrap_or_default();
    assert!(
        !stored.contains("cat_style"),
        "a refused value must not be written to the config: {stored:?}"
    );
}

/// Without bat on `PATH` there is nothing to ask, so an unchecked value is
/// stored as-is; `cat` is where an unusable bat gets reported.
#[test]
fn config_stores_an_unchecked_style_when_bat_is_absent() {
    let env = Env::new("no-bat");

    let (code, _, stderr) = env.run_without_bat(&[
        "config",
        "cat_style",
        "numbers",
        "--lang",
        "en",
        "--no-color",
    ]);

    assert_eq!(code, 0, "no bat to ask is not a rejection: {stderr:?}");
    let stored = std::fs::read_to_string(env.config_file()).expect("config written");
    assert!(
        stored.contains("numbers"),
        "the value is stored verbatim: {stored}"
    );
}

/// Guard against the stub silently losing its point: the copied binary must
/// really reject the argument list hok hands bat (otherwise the two tests above
/// would pass for the wrong reason).
#[test]
fn the_stub_bat_really_fails() {
    let env = Env::new("stub");
    let stub: &Path = &env.stub.join("bat.exe");
    let out = Command::new(stub)
        .args(["--no-paging", "--style", "bat", "--language", "json"])
        .output()
        .expect("run stub bat");
    assert!(
        !out.status.success(),
        "the stub must exit non-zero for the tests above to mean anything"
    );
}
