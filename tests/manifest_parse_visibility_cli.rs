//! A manifest that exists but does not parse must stop being invisible.
//!
//! Plan H3 (`O2a + O3`): a query that named the package reports one `warn`
//! naming the file and the real reason, while the existing "not found" error
//! and its exit code stay exactly as they were; a full scan reports one counted
//! summary instead of a line per broken manifest; and a healthy bucket gains no
//! output at all.
//!
//! These run the real binary against an isolated root, so the whole chain is
//! exercised end to end. The evidence is local fixtures — no network.

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
        let base = std::env::temp_dir().join(format!("hok_manifest_visibility_{name}"));
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

    /// The bucket directory the scanning commands take via `-d`.
    fn bucket_dir(&self) -> PathBuf {
        let dir = self.root.join("buckets").join("main").join("bucket");
        std::fs::create_dir_all(&dir).expect("create bucket dir");
        dir
    }

    fn write_manifest(&self, name: &str, body: &str) {
        std::fs::write(self.bucket_dir().join(format!("{name}.json")), body)
            .expect("write manifest");
    }

    fn write_good(&self, name: &str) {
        self.write_manifest(
            name,
            r#"{"version":"1.0.0","homepage":"https://example.com","license":"MIT"}"#,
        );
    }

    fn command(&self, args: &[&str]) -> Command {
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
        cmd
    }

    /// `(exit code, stdout, stderr)`.
    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let out: Output = self.command(args).output().expect("run hok");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// `(exit code, stdout+stderr)`.
    fn run_merged(&self, args: &[&str]) -> (i32, String) {
        let (code, stdout, stderr) = self.run(args);
        (code, format!("{stdout}{stderr}"))
    }
}

/// The official sentence (`lib/manifest.ps1:10`) plus hok's reason.
const PARSE_MARKER: &str = "Error parsing JSON at '";

/// Count occurrences of a substring, so "exactly one warning" is a hard
/// assertion instead of "at least one".
fn occurrences(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// The named-query commands that must not answer "not found" in silence.
#[test]
fn a_named_query_warns_about_a_broken_manifest_and_keeps_exit_code_one() {
    let fixture = Fixture::new("named");
    fixture.write_good("good");
    fixture.write_manifest("broken", "{ not json");

    for args in [
        vec!["search", "-e", "broken"],
        vec!["info", "broken"],
        vec!["install", "broken"],
        vec!["depends", "broken"],
    ] {
        let (code, text) = fixture.run_merged(&args);
        assert_eq!(
            code,
            1,
            "`{}` must keep exit code 1: {text}",
            args.join(" ")
        );
        assert_eq!(
            occurrences(&text, PARSE_MARKER),
            1,
            "`{}` must warn exactly once: {text}",
            args.join(" ")
        );
        assert!(
            text.contains("broken.json"),
            "`{}` must name the file: {text}",
            args.join(" ")
        );
    }
}

/// A full scan reports one counted summary (paths are `-vv` details), and the
/// healthy package is still listed.
#[test]
fn a_wildcard_search_reports_one_counted_summary() {
    let fixture = Fixture::new("wildcard");
    fixture.write_good("good");
    fixture.write_manifest("broken_a", "{ not json");
    fixture.write_manifest("broken_b", "[1, 2");

    let (code, text) = fixture.run_merged(&["search", "*"]);
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains("good"),
        "the healthy manifest is listed: {text}"
    );
    assert!(
        !text.contains("broken_a"),
        "broken files are not packages: {text}"
    );
    assert_eq!(
        occurrences(&text, "manifest(s) could not be parsed"),
        1,
        "one summary line: {text}"
    );
    assert_eq!(
        occurrences(&text, PARSE_MARKER),
        0,
        "paths stay behind -vv: {text}"
    );
}

/// The paths only appear at `-vv`, where each broken manifest gets a `detail`
/// line under the summary.
#[test]
fn a_wildcard_search_lists_the_paths_only_at_high_verbosity() {
    let fixture = Fixture::new("wildcard_vv");
    fixture.write_good("good");
    fixture.write_manifest("broken_a", "{ not json");

    let (code, text) = fixture.run_merged(&["-vv", "search", "*"]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(occurrences(&text, PARSE_MARKER), 1, "{text}");
    assert!(text.contains("broken_a.json"), "{text}");
}

/// A healthy bucket must gain no output: the same query over a bucket with and
/// without an unrelated broken manifest is byte-identical.
#[test]
fn an_unrelated_broken_manifest_adds_no_output_to_a_named_query() {
    let clean = Fixture::new("byte_clean");
    clean.write_good("good");

    let dirty = Fixture::new("byte_dirty");
    dirty.write_good("good");
    dirty.write_manifest("broken", "{ not json");

    let (clean_code, clean_out, clean_err) = clean.run(&["search", "-e", "good"]);
    let (dirty_code, dirty_out, dirty_err) = dirty.run(&["search", "-e", "good"]);

    assert_eq!(clean_code, 0, "{clean_out}{clean_err}");
    assert_eq!(dirty_code, 0, "{dirty_out}{dirty_err}");
    assert_eq!(
        dirty_out, clean_out,
        "stdout must be byte-identical with an unrelated broken manifest present"
    );
    assert_eq!(
        dirty_err, clean_err,
        "stderr must be byte-identical with an unrelated broken manifest present"
    );
}

/// `checkver` and `checkurls` used to skip an unparsable file in silence;
/// official Scoop warns through `parse_json` (`lib/manifest.ps1:10`).
#[test]
fn the_bucket_scanning_commands_warn_about_a_broken_manifest() {
    let fixture = Fixture::new("scanners");
    fixture.write_good("good");
    fixture.write_manifest("broken", "{ not json");
    let dir = fixture.bucket_dir();
    let dir = dir.to_str().expect("utf-8 bucket path");

    for command in ["checkver", "checkurls"] {
        let (code, text) = fixture.run_merged(&[command, "-d", dir, "broken"]);
        assert_eq!(code, 0, "`{command}` must still succeed: {text}");
        assert_eq!(
            occurrences(&text, PARSE_MARKER),
            1,
            "`{command}` must warn exactly once: {text}"
        );
        assert!(text.contains("broken.json"), "`{command}`: {text}");
    }
}
