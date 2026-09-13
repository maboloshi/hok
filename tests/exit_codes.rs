//! Process exit codes of the CLI entry points.
//!
//! Scoop's scripts exit 0 on success, 1 on a usage error or a failed
//! operation (`lib/core.ps1:293` `abort` defaults to 1,
//! `libexec/scoop-install.ps1:60` reports `<app> missing` before `exit 1`),
//! and reserve a dedicated code for a handful of commands: `which` 2,
//! `bucket list` with no bucket 2, `shim` 3 (not found anywhere) / 2 (found in
//! the other scope), and the `virustotal` bit mask (8 = manifest not found).
//!
//! clap would exit 2 for every usage error, so `cmd::start` renders the parse
//! outcome itself; these tests pin the resulting process codes against an
//! empty Scoop root, where every query is deterministic and nothing is
//! installed.

use std::path::PathBuf;
use std::process::Command;

/// A throwaway Scoop root plus the environment overrides that keep the test
/// away from the real installation and the user's `config.json`.
struct Env {
    root: PathBuf,
    home: PathBuf,
}

impl Env {
    fn new(name: &str) -> Env {
        let base = std::env::temp_dir().join(format!("hok_exit_codes_{name}"));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("scoop");
        let home = base.join("home");
        std::fs::create_dir_all(&root).expect("create test root");
        std::fs::create_dir_all(&home).expect("create test home");
        Env { root, home }
    }

    /// Run `hok <args>` against this environment and return the exit code.
    ///
    /// `SCOOP_GLOBAL` is redirected inside the test root too, so a query that
    /// reaches the global scope (e.g. `shim list -g`) never reads the machine's
    /// real global installation.
    fn code(&self, args: &[&str]) -> i32 {
        let out = Command::new(env!("CARGO_BIN_EXE_hok"))
            .args(args)
            .env("SCOOP", &self.root)
            .env("SCOOP_GLOBAL", self.root.join("global"))
            .env("USERPROFILE", &self.home)
            .env("HOME", &self.home)
            .env("APPDATA", self.home.join("AppData"))
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env_remove("HOK_LOG_LEVEL")
            .output()
            .expect("run hok");
        out.status.code().unwrap_or(-1)
    }
}

/// Help screens exit 0 (Scoop renders help for a bare `scoop`), usage errors
/// exit 1.
#[test]
fn help_screens_and_usage_errors() {
    let env = Env::new("usage");

    assert_eq!(env.code(&[]), 0, "bare `hok` renders the command list");
    assert_eq!(env.code(&["--help"]), 0);
    assert_eq!(env.code(&["help"]), 0);
    assert_eq!(env.code(&["help", "not-a-command"]), 1);

    assert_eq!(env.code(&["not-a-command"]), 1, "unknown subcommand");
    assert_eq!(env.code(&["install"]), 1, "missing <app>");
    // `scoop cleanup` without an app is a usage error
    // (`scoop-cleanup.ps1:24`), while a bare `scoop cache` lists
    // (`scoop-cache.ps1:61-71`).
    assert_eq!(env.code(&["cleanup"]), 1, "missing <app>");
    assert_eq!(env.code(&["cache", "rm"]), 1, "missing <app(s)>");
    assert_eq!(env.code(&["cache"]), 0, "bare cache lists");
}

/// The config views (`config`, `-j`, `-a`, a single setting and `rm`) are all
/// successful reads/writes, including a setting that is merely unset; a usage
/// error inside the command exits 1 like every other subcommand.
#[test]
fn config_views_and_usage_errors_exit_cleanly() {
    let env = Env::new("config");

    assert_eq!(env.code(&["config"]), 0, "the file's own settings");
    assert_eq!(env.code(&["config", "-j"]), 0);
    assert_eq!(env.code(&["config", "-a"]), 0);
    assert_eq!(env.code(&["config", "proxy"]), 0, "unset is not a failure");
    assert_eq!(env.code(&["config", "proxy", "127.0.0.1:1"]), 0);
    assert_eq!(env.code(&["config", "rm", "proxy"]), 0);

    assert_eq!(env.code(&["config", "rm"]), 1, "missing <KEY>");
    assert_eq!(env.code(&["config", "-j", "-a"]), 1, "conflicting views");
    assert_eq!(
        env.code(&["config", "no_such_setting", "x"]),
        1,
        "unknown key"
    );
    assert_eq!(env.code(&["config", "aria2_split", "7"]), 1, "no aliases");
}

/// Query commands against an empty installation.
#[test]
fn query_exit_codes_on_an_empty_root() {
    let env = Env::new("query");

    // No app installed at all: Scoop reports it and exits 1
    // (`libexec/scoop-list.ps1:19-22`).
    assert_eq!(env.code(&["list"]), 1);
    assert_eq!(env.code(&["list", "ghost"]), 1);

    // A search with no match is a failed search (`scoop-search.ps1:199`).
    assert_eq!(env.code(&["search", "ghost"]), 1);

    // No bucket added (`scoop-bucket.ps1:61-70`).
    assert_eq!(env.code(&["bucket", "list"]), 2);

    assert_eq!(env.code(&["status"]), 0);
}

/// Install/uninstall lifecycle against an empty installation: a missing app is
/// reported and dropped from the batch, which still exits 0
/// (`lib/core.ps1:1084-1121`, `scoop-uninstall.ps1:44-45`).
#[test]
fn sync_exit_codes_on_an_empty_root() {
    let env = Env::new("sync");

    assert_eq!(env.code(&["uninstall", "ghost"]), 0);
    assert_eq!(env.code(&["update", "ghost"]), 0);
}

/// The per-command codes that are not 0/1.
#[test]
fn dedicated_exit_codes() {
    let env = Env::new("dedicated");

    assert_eq!(env.code(&["which", "ghost"]), 2);
    assert_eq!(env.code(&["shim", "rm", "ghost"]), 3);
    assert_eq!(env.code(&["shim", "info", "ghost"]), 3);
    // Scoop bit 8: the manifest could not be found
    // (`scoop-virustotal.ps1:16-17,272-275`).
    assert_eq!(env.code(&["virustotal", "ghost"]), 8);
}

/// `-g` parses anywhere before `--` (upstream's getopt), `list *` lists
/// everything rather than failing as an invalid regex, and a missing
/// `<command_path>` reports upstream's message with exit 1
/// (`libexec/scoop-shim.ps1:40-48,50-59,142-143`).
#[test]
fn shim_flag_positions_and_patterns() {
    let env = Env::new("shim_flags");

    assert_eq!(env.code(&["shim", "list", "-g"]), 0);
    assert_eq!(env.code(&["shim", "-g", "list"]), 0);
    assert_eq!(env.code(&["shim", "list", "*"]), 0);
    assert_eq!(env.code(&["shim", "list", "*", "-g"]), 0);

    assert_eq!(env.code(&["shim", "add", "onlyname"]), 1);
}
