//! Report the `checkver` / `autoupdate` status of every manifest in a bucket.
//!
//! Mirrors Scoop's `bin/missing-checkver.ps1`: each matching manifest gets a
//! `[C]heckver | [A]utoupdate` truth-table row, and the caller can narrow the
//! listing to the manifests that still miss one of the two properties
//! (official `-SkipSupported`).

use std::path::Path;

use crate::{
    error::Fallible,
    package::{manifest, manifest_walker},
};

/// One row of the `[C]heckver | [A]utoupdate` truth table.
#[derive(Debug, Clone)]
pub struct MissingCheckverItem {
    /// Manifest stem (its file name without the `.json` extension).
    pub name: String,
    /// Whether the manifest declares a usable `checkver`.
    ///
    /// `None` when the manifest could not be parsed at all; such a row is
    /// rendered `[ ][ ]` exactly like a manifest that has neither property
    /// (official `bin/missing-checkver.ps1:40-51`), and is counted separately
    /// in [`MissingCheckverReport::unparsable`] so the totals stay honest.
    pub checkver: Option<bool>,
    /// Whether the manifest declares an `autoupdate`. See [`Self::checkver`]
    /// for the `None` case.
    pub autoupdate: Option<bool>,
}

impl MissingCheckverItem {
    /// Whether the manifest has both properties, i.e. is excluded by official
    /// `-SkipSupported` (`bin/missing-checkver.ps1:42`).
    ///
    /// An unparsable manifest is never "supported".
    pub fn is_supported(&self) -> bool {
        self.checkver == Some(true) && self.autoupdate == Some(true)
    }

    /// Whether the manifest could not be parsed.
    pub fn is_unparsable(&self) -> bool {
        self.checkver.is_none()
    }
}

#[derive(Debug, Clone, Default)]
pub struct MissingCheckverReport {
    /// Manifests that were successfully parsed.
    pub total: u32,
    /// Successfully parsed manifests without a `checkver`.
    pub missing_checkver: u32,
    /// Successfully parsed manifests without an `autoupdate`.
    pub missing_autoupdate: u32,
    /// Manifests the walker found but that could not be parsed.
    pub unparsable: u32,
    /// Every matching manifest, in the walker's sorted path order.
    pub items: Vec<MissingCheckverItem>,
    /// Parse failures, each already worded as the official warning line
    /// (`Error parsing JSON at '<path>'.`). The caller renders them.
    pub warnings: Vec<String>,
}

/// Scan `dir` for manifests matching `app` and report their `checkver` /
/// `autoupdate` status.
///
/// `app` uses the shared bucket-pattern semantics
/// ([`manifest_walker::discover_matching`] /
/// [`crate::internal::string::matches_any_glob`]): a first pattern of `"*"`
/// matches every manifest, otherwise a stem must match at least one pattern
/// (glob `*` / `?`, case-insensitive). A caller that accepts an *app name*
/// rather than a stem pattern (official `-App`) strips a trailing `.json`
/// before calling — see `hok missing-checkver -a`.
///
/// Parse failures are not fatal: the file still gets a row (with `checkver` /
/// `autoupdate` both unknown) plus a warning, so the truth-table line count
/// stays equal to the number of matching manifests. `package.json` is not a
/// manifest and is skipped by the walker, exactly as for every other bucket
/// scanning command.
pub fn scan(dir: &Path, app: &[String]) -> Fallible<MissingCheckverReport> {
    let mut report = MissingCheckverReport::default();

    for (path, name) in manifest_walker::discover_matching(dir, app)? {
        match manifest::parse_file(&path) {
            Ok(m) => {
                let has_checkver = m.checkver().is_some();
                let has_autoupdate = m.autoupdate().is_some();
                report.total += 1;
                if !has_checkver {
                    report.missing_checkver += 1;
                }
                if !has_autoupdate {
                    report.missing_autoupdate += 1;
                }
                report.items.push(MissingCheckverItem {
                    name,
                    checkver: Some(has_checkver),
                    autoupdate: Some(has_autoupdate),
                });
            }
            Err(_) => {
                report.unparsable += 1;
                report.items.push(MissingCheckverItem {
                    name,
                    checkver: None,
                    autoupdate: None,
                });
                // Worded like the official warning
                // (`lib/manifest.ps1:10`), so a bucket maintainer sees the
                // same line `bin/missing-checkver.ps1` prints.
                report
                    .warnings
                    .push(format!("Error parsing JSON at '{}'.", path.display()));
            }
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn tmpdir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("hok_missing_checkver_{label}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

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

    fn all() -> Vec<String> {
        vec!["*".to_string()]
    }

    #[test]
    fn truth_table_covers_every_matching_manifest() {
        let dir = tmpdir("truth_table");
        fs::write(
            dir.join("both.json"),
            manifest(r#","checkver": "github", "autoupdate": {"url": "https://e.com/$version"}"#),
        )
        .unwrap();
        fs::write(dir.join("neither.json"), manifest("")).unwrap();
        fs::write(
            dir.join("onlyver.json"),
            manifest(r#","checkver": "github""#),
        )
        .unwrap();

        let report = scan(&dir, &all()).unwrap();
        assert_eq!(report.total, 3);
        assert_eq!(report.items.len(), 3, "one row per manifest");
        assert_eq!(report.missing_checkver, 1, "neither.json");
        assert_eq!(report.missing_autoupdate, 2, "neither + onlyver");
        assert_eq!(report.unparsable, 0);
        assert!(report.warnings.is_empty());

        // Walker order is the sorted path order.
        let names: Vec<&str> = report.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["both", "neither", "onlyver"]);

        assert!(report.items[0].is_supported());
        assert!(!report.items[1].is_supported());
        assert!(!report.items[2].is_supported());
    }

    #[test]
    fn unparsable_manifest_gets_a_row_and_is_counted_once() {
        let dir = tmpdir("unparsable");
        fs::write(dir.join("bad.json"), "{ not json").unwrap();
        fs::write(dir.join("ok.json"), manifest("")).unwrap();

        let report = scan(&dir, &all()).unwrap();
        assert_eq!(report.total, 1, "only the parseable manifest is counted");
        assert_eq!(report.unparsable, 1, "parse failure counted separately");
        assert_eq!(
            report.missing_checkver, 1,
            "the failure must not inflate the checkver count"
        );
        assert_eq!(report.items.len(), 2, "the failure still gets a row");
        assert!(report.items[0].is_unparsable());
        assert!(
            !report.items[0].is_supported(),
            "unparsable is not supported"
        );
        assert_eq!(report.warnings.len(), 1);
        assert!(
            report.warnings[0].starts_with("Error parsing JSON at '"),
            "{}",
            report.warnings[0]
        );
    }

    #[test]
    fn app_pattern_filters_rows() {
        let dir = tmpdir("filter");
        fs::write(dir.join("curl.json"), manifest("")).unwrap();
        fs::write(dir.join("git-lfs.json"), manifest("")).unwrap();

        let report = scan(&dir, &["git*".to_string()]).unwrap();
        assert_eq!(report.items.len(), 1);
        assert_eq!(report.items[0].name, "git-lfs");

        // Case-insensitive, like PowerShell's `-Filter`.
        let report = scan(&dir, &["CURL".to_string()]).unwrap();
        assert_eq!(report.items.len(), 1);
        assert_eq!(report.items[0].name, "curl");
    }

    #[test]
    fn package_json_is_not_a_manifest() {
        let dir = tmpdir("package_json");
        fs::write(dir.join("package.json"), manifest("")).unwrap();
        fs::write(dir.join("app.json"), manifest("")).unwrap();

        let report = scan(&dir, &all()).unwrap();
        let names: Vec<&str> = report.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["app"], "package.json is skipped by the walker");
    }

    #[test]
    fn empty_dir_reports_nothing() {
        let dir = tmpdir("empty");
        let report = scan(&dir, &all()).unwrap();
        assert_eq!(report.total, 0);
        assert_eq!(report.unparsable, 0);
        assert!(report.items.is_empty());
        assert!(report.warnings.is_empty());
    }
}
