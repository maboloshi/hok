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

use std::path::{Path, PathBuf};
use std::process::Command;

/// A throwaway Scoop root plus the environment overrides that keep the test
/// away from the real installation and the user's `config.json`.
struct Env {
    root: PathBuf,
    home: PathBuf,
    /// An empty working directory: the bucket-scanning commands default `-d`
    /// to `.`, so running them from a fixture directory keeps the scan
    /// deterministic and offline instead of walking the source checkout.
    cwd: PathBuf,
}

impl Env {
    fn new(name: &str) -> Env {
        let base = std::env::temp_dir().join(format!("hok_exit_codes_{name}"));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("scoop");
        let home = base.join("home");
        let cwd = base.join("cwd");
        std::fs::create_dir_all(&root).expect("create test root");
        std::fs::create_dir_all(&home).expect("create test home");
        std::fs::create_dir_all(&cwd).expect("create test cwd");
        Env { root, home, cwd }
    }

    /// Run `hok <args>` against this environment and return the exit code.
    ///
    /// `SCOOP_GLOBAL` is redirected inside the test root too, so a query that
    /// reaches the global scope (e.g. `shim list -g`) never reads the machine's
    /// real global installation.
    fn code(&self, args: &[&str]) -> i32 {
        self.command(args)
            .output()
            .expect("run hok")
            .status
            .code()
            .unwrap_or(-1)
    }

    /// Run `hok <args>` with this environment's empty fixture directory as the
    /// working directory; returns `(code, stdout + stderr)`.
    fn run_in(&self, cwd: &Path, args: &[&str]) -> (i32, String) {
        let out = self
            .command(args)
            .current_dir(cwd)
            .output()
            .expect("run hok");
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        (out.status.code().unwrap_or(-1), text)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_hok"));
        cmd.args(args)
            .env("SCOOP", &self.root)
            .env("SCOOP_GLOBAL", self.root.join("global"))
            .env("USERPROFILE", &self.home)
            .env("HOME", &self.home)
            .env("APPDATA", self.home.join("AppData"))
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env_remove("HOK_LOG_LEVEL");
        cmd
    }
}

/// `scoop download` still downloads the remaining apps after one fails, but the
/// process must report the failure: upstream ends with
/// `exit [int]$download_failed` (`libexec/scoop-download.ps1`), so one bad
/// manifest makes the whole invocation exit 1.
///
/// The fixture uses a manifest whose version carries an unsupported character,
/// which hok rejects before any network access (`cmd.manifest_version_invalid`)
/// — so this stays offline and deterministic.
#[test]
fn download_failure_exits_nonzero() {
    let env = Env::new("download_failure");

    let good = env.cwd.join("good.json");
    std::fs::write(
        &good,
        // A valid version but a URL that is never reached: the version check
        // happens before resolution, so this file only needs to be plausible.
        r#"{"version": "1.0.0", "url": "http://127.0.0.1:1/x.bin", "hash": "0000000000000000000000000000000000000000000000000000000000000000"}"#,
    )
    .unwrap();
    let bad = env.cwd.join("bad.json");
    std::fs::write(
        &bad,
        r#"{"version": "1.0!", "url": "http://127.0.0.1:1/x.bin", "hash": "0000000000000000000000000000000000000000000000000000000000000000"}"#,
    )
    .unwrap();

    let bad = bad.to_string_lossy().into_owned();
    let (code, text) = env.run_in(&env.cwd, &["download", &bad]);
    assert_eq!(
        code, 1,
        "a rejected manifest must fail the download invocation: {text}"
    );

    // An unresolvable path is a failure too, and must not abort the run.
    let (code, text) = env.run_in(&env.cwd, &["download", r"D:\nonexistent\app.json"]);
    assert_eq!(code, 1, "an unresolvable query must exit non-zero: {text}");
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
    // `-l/--list` is the read-only counterpart: a bare `cleanup -l` is every
    // app in scope, a missing app is reported without failing, and the
    // destructive `-k` cannot be combined with it.
    assert_eq!(env.code(&["cleanup", "-l"]), 0, "bare `cleanup -l` lists");
    assert_eq!(env.code(&["cleanup", "-l", "ghost"]), 0, "ghost reported");
    assert_eq!(env.code(&["cleanup", "-l", "-k"]), 1, "-l rejects -k");
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

/// A run that rewrites manifests must name its target: `-d/--dir` still
/// defaults to `.`, but never for a write.
///
/// The scan behind these commands (`manifest_walker::discover`) collects every
/// `*.json` under the directory — `package.json` is the only exclusion — and
/// `formatjson` never checks whether a file looks like a Scoop manifest, so a
/// bare `hok formatjson` used to rewrite `tsconfig.json`, `package-lock.json`,
/// `target/**` and the user's own `config.json` in place. Upstream declares
/// `-Dir` `Mandatory = $true` for the same reason (`bin/formatjson.ps1`,
/// `bin/checkhashes.ps1`, `bin/checkurls.ps1`, `bin/missing-checkver.ps1`) or
/// throws (`bin/checkver.ps1:85`); hok only insists when the run writes.
///
/// `--force`/`--force-update` imply the update and therefore also write
/// (upstream `if ($ForceUpdate) { $Update = $true }`, `bin/checkhashes.ps1:52`
/// and `bin/checkver.ps1:404`), so both switches are covered.
#[test]
fn write_runs_require_an_explicit_dir() {
    let env = Env::new("dir_guard");
    let cwd = env.cwd.clone();

    // A decoy tree: exactly the collateral a bare `formatjson` used to destroy.
    std::fs::write(cwd.join("tsconfig.json"), r#"{ "compilerOptions": {} }"#).unwrap();
    std::fs::write(cwd.join("package-lock.json"), r#"{"a":1}"#).unwrap();
    std::fs::create_dir_all(cwd.join("target")).unwrap();
    std::fs::write(cwd.join("target").join("out.json"), r#"{"b":2}"#).unwrap();

    for args in [
        vec!["formatjson"],
        vec!["checkhashes", "--update"],
        vec!["checkhashes", "--force"],
        vec!["checkver", "--update"],
        vec!["checkver", "--force-update"],
    ] {
        let (code, text) = env.run_in(&cwd, &args);
        let invocation = args.join(" ");
        assert_eq!(
            code, 1,
            "`hok {invocation}` must be a usage error without -d: {text}"
        );
        assert!(
            text.contains("-d"),
            "the error must say which target to pass: {text}"
        );
        assert!(
            text.contains(args[0]),
            "the error must name the command (`{invocation}`): {text}"
        );
    }

    // The guard fires before the walk: not one byte of the decoy tree moved.
    assert_eq!(
        std::fs::read_to_string(cwd.join("tsconfig.json")).unwrap(),
        r#"{ "compilerOptions": {} }"#
    );
    assert_eq!(
        std::fs::read_to_string(cwd.join("package-lock.json")).unwrap(),
        r#"{"a":1}"#
    );
    assert_eq!(
        std::fs::read_to_string(cwd.join("target").join("out.json")).unwrap(),
        r#"{"b":2}"#
    );
}

/// ... and the same commands still work when the target is explicit.
#[test]
fn write_runs_accept_an_explicit_dir() {
    let env = Env::new("dir_guard_ok");
    let bucket = env.cwd.join("bucket");
    std::fs::create_dir_all(&bucket).unwrap();
    // `formatjson` rewrites this in place.
    std::fs::write(
        bucket.join("fmt.json"),
        r#"{"version":"1.0","homepage":"https://e.c","license":"MIT"}"#,
    )
    .unwrap();
    // `checkhashes` just reports a manifest without URLs (nothing to download).
    std::fs::write(
        bucket.join("hashes.json"),
        r#"{"version":"1.0","homepage":"https://e.c","license":"MIT"}"#,
    )
    .unwrap();
    // `checkver` needs an entry that declares `checkver`, otherwise the run has
    // nothing to wait for (see the read-only test below); a refused loopback
    // URL keeps it offline and fast.
    std::fs::write(
        bucket.join("ver.json"),
        r#"{"version":"1.0","homepage":"https://e.c","license":"MIT","checkver":{"url":"http://127.0.0.1:1/"}}"#,
    )
    .unwrap();
    let dir = bucket.to_str().unwrap().to_string();

    for args in [
        vec!["formatjson", "-d", &dir],
        vec!["checkhashes", "--update", "-d", &dir],
        vec!["checkver", "--update", "-d", &dir, "-t", "1"],
    ] {
        let (code, text) = env.run_in(&env.cwd, &args);
        let invocation = args.join(" ");
        // These runs may legitimately fail (no URLs, refused connection); the
        // assertion is that the write guard stays out of the way.
        assert!(
            !text.contains("writes to disk"),
            "`hok {invocation}` must get past the -d guard: {text}"
        );
        assert_ne!(code, 1, "`hok {invocation}` failed: {text}");
    }

    let formatted = std::fs::read_to_string(bucket.join("fmt.json")).unwrap();
    assert!(
        formatted.contains("\r\n"),
        "manifest not rewritten: {formatted:?}"
    );
}

/// Read-only runs keep the `.` default: the guard must not leak into them.
///
/// This is a deliberate deviation from upstream, which requires `-Dir` for
/// `checkurls` / `missing-checkver` and for a plain `checkhashes` too.
///
/// The fixture directory is deliberately not empty: `hok checkver` hangs when
/// *no* manifest declares `checkver` (pre-existing at base `2a18d5b`, unrelated
/// to this guard), so it gets an entry pointing at a refused loopback URL —
/// which also proves the `.` default really was scanned, because the stem shows
/// up in the report. `urls.json` is the one `checkurls` can actually report.
#[test]
fn read_only_runs_still_default_to_the_current_directory() {
    let env = Env::new("dir_default");
    let cwd = env.cwd.clone();
    std::fs::write(
        cwd.join("plain.json"),
        r#"{"version":"1.0","homepage":"https://e.c","license":"MIT"}"#,
    )
    .unwrap();
    std::fs::write(
        cwd.join("ver.json"),
        r#"{"version":"1.0","homepage":"https://e.c","license":"MIT","checkver":{"url":"http://127.0.0.1:1/"}}"#,
    )
    .unwrap();
    std::fs::write(
        cwd.join("urls.json"),
        r#"{"version":"1.0","homepage":"https://e.c","license":"MIT","url":"http://127.0.0.1:1/x.zip","hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
    )
    .unwrap();

    // command → a stem that only shows up if the current directory was scanned
    for (args, expected) in [
        (vec!["checkhashes"], "ver"),
        (vec!["checkurls"], "urls"),
        (vec!["missing-checkver"], "plain"),
    ] {
        let (code, text) = env.run_in(&cwd, &args);
        let invocation = args.join(" ");
        assert_eq!(
            code, 0,
            "`hok {invocation}` must still run with -d defaulting to `.`: {text}"
        );
        assert!(
            text.contains(expected),
            "`hok {invocation}` did not scan the current directory: {text}"
        );
    }

    let (code, text) = env.run_in(&cwd, &["checkver", "-t", "1"]);
    assert_eq!(code, 0, "`hok checkver` must still run: {text}");
    assert!(
        text.contains("ver"),
        "`hok checkver` did not scan the current directory: {text}"
    );
}
