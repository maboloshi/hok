//! `hok update/upgrade -c/--cleanup`.
//!
//! `-c` removes the old versions of exactly the packages a run actually
//! upgraded. It is not `cleanup -a` bolted onto an upgrade: an app that merely
//! has a leftover version directory must be left alone, and a run that upgraded
//! nothing must leave the disk untouched and say so.
//!
//! These run the real binary against an isolated root with a bucket manifest
//! and a marked-installed app, so the whole chain is exercised: resolve the
//! newer version, commit it, then sweep — and nothing else.

use std::path::PathBuf;
use std::process::{Command, Output};

/// An isolated installation: temp home, user root, and `%ProgramData%`.
struct Fixture {
    base: PathBuf,
    root: PathBuf,
    home: PathBuf,
    program_data: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let base = std::env::temp_dir().join(format!("hok_update_cleanup_{name}"));
        let _ = std::fs::remove_dir_all(&base);
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

    fn app_dir(&self, name: &str) -> PathBuf {
        self.root.join("apps").join(name)
    }

    /// A bucket manifest, i.e. the newer version the upgrade resolves to.
    fn write_bucket_manifest(&self, name: &str, version: &str) {
        let dir = self.root.join("buckets").join("main").join("bucket");
        std::fs::create_dir_all(&dir).expect("create bucket dir");
        std::fs::write(
            dir.join(format!("{name}.json")),
            format!(
                r#"{{"version":"{version}","homepage":"https://example.com","license":"MIT"}}"#
            ),
        )
        .expect("write bucket manifest");
    }

    /// Mark `name` installed at `version` (metadata under `current`, the way a
    /// real install writes it).
    fn mark_installed(&self, name: &str, version: &str) {
        let current = self.app_dir(name).join("current");
        std::fs::create_dir_all(&current).expect("create current dir");
        std::fs::write(
            current.join("scoop-manifest.json"),
            format!(
                r#"{{"version":"{version}","homepage":"https://example.com","license":"MIT"}}"#
            ),
        )
        .expect("write installed manifest");
        std::fs::write(
            current.join("scoop-install.json"),
            r#"{"architecture":"64bit","bucket":"main","hold":false}"#,
        )
        .expect("write install info");
    }

    /// A leftover version directory older than the installed version.
    fn leftover(&self, name: &str, version: &str) {
        std::fs::create_dir_all(self.app_dir(name).join(version)).expect("create leftover dir");
    }

    fn run(&self, args: &[&str]) -> (i32, String) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_hok"));
        cmd.args(args)
            .arg("--lang")
            .arg("en")
            .arg("--no-color")
            .current_dir(&self.base)
            .env("SCOOP", &self.root)
            .env("SCOOP_GLOBAL", self.root.join("global"))
            .env("SCOOP_CACHE", self.root.join("cache"))
            .env("USERPROFILE", &self.home)
            .env("HOME", &self.home)
            .env("APPDATA", self.home.join("AppData"))
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("ProgramData", &self.program_data)
            .env_remove("HOK_LOG_LEVEL");
        let out: Output = cmd.output().expect("run hok");
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        (out.status.code().unwrap_or(-1), text)
    }
}

/// Acceptance: upgrading a package with a leftover version, `-c` removes the
/// old version and keeps the new one (`current` included).
#[test]
fn cleanup_removes_the_old_version_of_an_upgraded_app() {
    let fixture = Fixture::new("upgraded");
    fixture.write_bucket_manifest("myapp", "2.0.0");
    fixture.mark_installed("myapp", "1.0.0");
    fixture.leftover("myapp", "0.9.0");
    // A download-cache entry for the old version: `-c` is not `cleanup -k`.
    let cached = fixture.root.join("cache").join("myapp#0.9.0#old.zip");
    std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
    std::fs::write(&cached, b"x").unwrap();

    let (code, out) = fixture.run(&["update", "myapp", "-c", "-y"]);
    assert_eq!(code, 0, "update -c should succeed:\n{out}");

    assert!(
        fixture.app_dir("myapp").join("2.0.0").is_dir(),
        "the upgraded version must be installed:\n{out}"
    );
    assert!(
        fixture.app_dir("myapp").join("current").exists(),
        "`current` must survive the sweep:\n{out}"
    );
    assert!(
        !fixture.app_dir("myapp").join("0.9.0").exists(),
        "-c must remove the leftover old version:\n{out}"
    );
    assert!(
        cached.exists(),
        "-c must not touch the download cache (that is `cleanup -k`):\n{out}"
    );
    assert!(
        !out.contains("Nothing was upgraded"),
        "an upgraded run must not print the no-op hint:\n{out}"
    );
}

/// The same run **without** `-c` keeps the leftover: the upgrade path itself
/// never sweeps, so the removal the `-c` test asserts can only come from `-c`.
#[test]
fn without_cleanup_the_leftover_old_version_survives() {
    let fixture = Fixture::new("no_flag");
    fixture.write_bucket_manifest("myapp", "2.0.0");
    fixture.mark_installed("myapp", "1.0.0");
    fixture.leftover("myapp", "0.9.0");

    let (code, out) = fixture.run(&["update", "myapp", "-y"]);
    assert_eq!(code, 0, "update should succeed:\n{out}");
    assert!(
        fixture.app_dir("myapp").join("2.0.0").is_dir(),
        "the upgrade itself must still happen:\n{out}"
    );
    assert!(
        fixture.app_dir("myapp").join("0.9.0").exists(),
        "without -c nothing may be swept:\n{out}"
    );
}

/// Acceptance: `-c` on a run that upgrades nothing has zero side effects (the
/// leftover version survives) and prints the `hok cleanup -a` hint.
#[test]
fn cleanup_without_an_upgrade_is_a_no_op_with_a_hint() {
    let fixture = Fixture::new("noop");
    // The bucket is already at the installed version: nothing to upgrade.
    fixture.write_bucket_manifest("myapp", "1.0.0");
    fixture.mark_installed("myapp", "1.0.0");
    fixture.leftover("myapp", "0.9.0");

    let (code, out) = fixture.run(&["update", "myapp", "-c", "-y"]);
    assert_eq!(code, 0, "update -c should succeed:\n{out}");

    assert!(
        out.contains("Nothing was upgraded"),
        "the no-op hint must be printed:\n{out}"
    );
    assert!(
        fixture.app_dir("myapp").join("0.9.0").exists(),
        "a run with no upgrade must not sweep anything:\n{out}"
    );
    assert!(
        fixture
            .app_dir("myapp")
            .join("1.0.0")
            .join("manifest.json")
            .exists()
            || fixture.app_dir("myapp").join("current").exists(),
        "the installed app must be untouched:\n{out}"
    );
}

/// The same boundary after the batch has actually run: `-U/--no-upgrade` skips
/// every upgrade, so `-c` must clean nothing and say so.
#[test]
fn cleanup_with_no_upgrade_is_a_no_op_with_a_hint() {
    let fixture = Fixture::new("no_upgrade");
    fixture.write_bucket_manifest("myapp", "2.0.0");
    fixture.mark_installed("myapp", "1.0.0");
    fixture.leftover("myapp", "0.9.0");

    let (code, out) = fixture.run(&["update", "myapp", "-c", "-U", "-y"]);
    assert_eq!(code, 0, "update -c -U should succeed:\n{out}");

    assert!(
        out.contains("Nothing was upgraded"),
        "the no-op hint must be printed:\n{out}"
    );
    assert!(
        fixture.app_dir("myapp").join("0.9.0").exists(),
        "-U upgraded nothing, so nothing may be swept:\n{out}"
    );
}
