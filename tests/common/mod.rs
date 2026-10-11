//! Shared plumbing for the CLI integration harnesses in `tests/`.
//!
//! Every harness needs the same few things: a throwaway directory under the
//! system temp, the `hok` binary under test, an environment that keeps the
//! child away from the real installation / the user's `config.json` / an
//! inherited log level, and `(exit code, stdout + stderr)`.
//!
//! The root *layout* inside that directory is deliberately **not** shared: the
//! harnesses exercise different installations (a bucket-only root, a full root
//! with `apps`/`buckets`, an empty `SCOOP`, a `PATH` with a stub `bat.exe`), so
//! each keeps its own `Env`/`Fixture` and only borrows the plumbing below.
//!
//! Cargo does not build a `tests/` subdirectory as its own test target, so this
//! is included with `mod common;` and its unused-per-harness helpers are fine.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh, emptied `hok_<suite>_<name>` directory under the system temp.
pub fn base(suite: &str, name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("hok_{suite}_{name}"));
    let _ = std::fs::remove_dir_all(&base);
    base
}

/// The binary under test.
pub fn hok() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hok"))
}

/// Point the child at `home` instead of the real user profile, and drop an
/// inherited `HOK_LOG_LEVEL` so logging set by the invoking environment cannot
/// pollute an assertion.
pub fn isolate<'a>(cmd: &'a mut Command, home: &Path) -> &'a mut Command {
    cmd.env("USERPROFILE", home)
        .env("HOME", home)
        .env("APPDATA", home.join("AppData"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env_remove("HOK_LOG_LEVEL")
}

/// `(exit code, stdout + stderr)` — what the harnesses assert on together.
pub fn text(out: Output) -> (i32, String) {
    let mut all = String::from_utf8_lossy(&out.stdout).into_owned();
    all.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), all)
}
