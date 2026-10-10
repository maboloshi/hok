//! Root resolution in stripped environments.
//!
//! Upstream Scoop builds its roots through a candidate chain filtered by
//! `Where-Object { $_ }` (`lib/core.ps1:1291-1301`), so an environment variable
//! set to an **empty** value counts as *unset* and the next candidate wins.
//! hok follows that: an empty `SCOOP`/`SCOOP_GLOBAL` must fall back to the
//! documented defaults rather than resolving to an empty (i.e.
//! current-directory-relative) root.
//!
//! A root that genuinely cannot be determined (no `SCOOP` and no home
//! directory) is reported as an error — it used to panic inside
//! `Session::new` (`internal::fs::write_json`).

use std::path::PathBuf;
use std::process::Command;

mod common;

use common::{hok, isolate, text};

/// An isolated installation: temp home, user root, and `%ProgramData%`.
struct Fixture {
    base: PathBuf,
    root: PathBuf,
    home: PathBuf,
    program_data: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let base = common::base("root_env", name);
        let fixture = Fixture {
            root: base.join("scoop"),
            home: base.join("home"),
            program_data: base.join("programdata"),
            base,
        };
        for dir in [&fixture.root, &fixture.home, &fixture.program_data] {
            std::fs::create_dir_all(dir).expect("create fixture dir");
        }
        fixture
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = hok();
        cmd.args(args)
            .current_dir(&self.base)
            .env("SCOOP", &self.root)
            .env("SCOOP_GLOBAL", self.root.join("global"))
            .env("ProgramData", &self.program_data);
        isolate(&mut cmd, &self.home);
        cmd
    }

    fn run(&self, args: &[&str]) -> (i32, String) {
        text(self.command(args).output().expect("run hok"))
    }
}

/// An empty `$SCOOP` counts as unset: the root falls back to `$HOME/scoop`,
/// exactly like an unset `$SCOOP`.
#[test]
fn empty_scoop_falls_back_to_the_home_directory() {
    let fixture = Fixture::new("empty_scoop");

    let (code, out) = text(
        fixture
            .command(&["config", "-a"])
            .env("SCOOP", "")
            .output()
            .expect("run hok"),
    );
    assert_eq!(code, 0, "config -a should succeed: {out}");

    let expected = fixture.home.join("scoop");
    assert!(
        out.contains(&expected.display().to_string()),
        "expected root {expected:?} in output:\n{out}"
    );
    assert!(!out.contains("panicked"), "must not panic:\n{out}");
}

/// An empty `$SCOOP_GLOBAL` counts as unset: the global root falls back to
/// `%ProgramData%\scoop`.
#[test]
fn empty_scoop_global_falls_back_to_program_data() {
    let fixture = Fixture::new("empty_scoop_global");

    let (code, out) = text(
        fixture
            .command(&["config", "-a"])
            .env("SCOOP_GLOBAL", "")
            .output()
            .expect("run hok"),
    );
    assert_eq!(code, 0, "config -a should succeed: {out}");

    let expected = fixture.program_data.join("scoop");
    assert!(
        out.contains(&expected.display().to_string()),
        "expected global root {expected:?} in output:\n{out}"
    );
    assert!(!out.contains("panicked"), "must not panic:\n{out}");
}

/// No `$SCOOP` and no home directory: the root is unknown, and that is
/// reported as an error instead of panicking (it used to abort inside
/// `Session::new`).
#[test]
fn unknown_root_is_reported_instead_of_panicking() {
    let fixture = Fixture::new("unknown_root");

    let (code, out) = text(
        fixture
            .command(&["list"])
            .env("SCOOP", "")
            .env_remove("USERPROFILE")
            .env_remove("HOME")
            .output()
            .expect("run hok"),
    );

    assert!(!out.contains("panicked"), "must not panic:\n{out}");
    assert_eq!(code, 1, "unknown root is a clean failure:\n{out}");
    assert!(
        out.contains("the Scoop root directory is unknown"),
        "expected the root error in output:\n{out}"
    );
}

/// The ordinary environment still resolves `$SCOOP` as before.
#[test]
fn explicit_scoop_still_wins() {
    let fixture = Fixture::new("explicit_scoop");

    let (code, out) = fixture.run(&["config", "-a"]);
    assert_eq!(code, 0, "config -a should succeed: {out}");
    assert!(
        out.contains(&fixture.root.display().to_string()),
        "expected root {:?} in output:\n{out}",
        fixture.root
    );
}
