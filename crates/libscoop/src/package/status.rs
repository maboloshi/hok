//! Installed-app status aggregation for the `status` command.
//!
//! Mirrors upstream `scoop status` (libexec/scoop-status.ps1) + `app_status`
//! (lib/core.ps1): for every installed app it reports which dimensions need
//! attention — outdated, install-failed, deprecated, manifest-removed,
//! missing dependencies, held — plus the installed and latest versions.
//!
//! # Design
//!
//! The heavy lifting (installed enumeration, `current`-junction resolution,
//! version selection, upgradable detection) lives in [`query::query_installed`];
//! this module builds on it and adds the pieces Scoop computes on top of the
//! installed package:
//!
//! - **deprecated** — a manifest for the app under the origin bucket's
//!   `deprecated` subfolder (upstream keeps deprecated manifests there);
//! - **removed** — the origin bucket no longer carries a live manifest for the
//!   app (either the bucket directory is gone, or the manifest was deleted);
//! - **missing_deps** — `depends` entries of the source manifest that are not
//!   installed.
//!
//! Output here is data, never text: the caller (CLI) renders the table and the
//! "Everything is ok!" gating.

use std::path::Path;

use crate::Session;
use crate::error::Fallible;
use crate::package::QueryOption;
use crate::package::query::query_installed;

/// The per-app status record produced by [`status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppStatus {
    /// Installed app name.
    pub name: String,
    /// Origin bucket name (or `__isolated__` for URL/path installs).
    pub bucket: String,
    /// Installed version (`None` for a broken install).
    pub installed_version: Option<String>,
    /// Latest version available from the origin bucket (`Some` only when
    /// outdated; upstream leaves this column blank otherwise).
    pub latest_version: Option<String>,
    /// A newer version exists in the origin bucket.
    pub outdated: bool,
    /// The install is broken/partial.
    pub failed: bool,
    /// A manifest exists under the origin bucket's `deprecated` folder.
    pub deprecated: bool,
    /// The origin bucket no longer has a live manifest for the app.
    pub removed: bool,
    /// The package is held.
    pub held: bool,
    /// Names of `depends` entries that are not installed.
    pub missing_deps: Vec<String>,
}

impl AppStatus {
    /// Whether this app needs to be listed in Scoop's "attention" view
    /// (upstream only shows apps that are outdated / failed / deprecated /
    /// removed / missing deps — a held-but-otherwise-healthy app is omitted).
    pub fn needs_attention(&self) -> bool {
        self.outdated
            || self.failed
            || self.deprecated
            || self.removed
            || !self.missing_deps.is_empty()
    }
}

/// Compute the status of every installed app.
///
/// Enumerates installed apps via [`query_installed`] with `Broken` +
/// `UpgradableCheck` (so partial installs are surfaced and upgradable state
/// is filled without filtering out up-to-date apps), then fills in the
/// deprecation / removal / missing-dependency dimensions per app.
///
/// # Errors
///
/// I/O errors reading the apps or buckets directory propagate.
pub fn status(session: &Session) -> Fallible<Vec<AppStatus>> {
    let packages = query_installed(
        session,
        &["*"],
        &[QueryOption::Broken, QueryOption::UpgradableCheck],
    )?;

    // Installed names in the current scope, used to detect missing deps.
    let mut installed_names = std::collections::HashSet::new();
    for pkg in &packages {
        installed_names.insert(pkg.name().to_lowercase());
    }

    let mut out = Vec::with_capacity(packages.len());
    for pkg in &packages {
        let installed_version = pkg.installed_version().map(str::to_owned);
        let latest = pkg.upgradable_version().map(str::to_owned);
        let bucket = pkg.bucket().to_owned();

        // Only bucket-installed apps can be deprecated/removed; isolated
        // (URL / local-path) installs have no origin bucket to check.
        let (deprecated, removed) = if pkg.is_isolated() {
            (false, false)
        } else {
            probe_origin_bucket(session, &bucket, pkg.name())
        };

        // Scoop's app_status checks the *explicit* depends field (not the
        // implicit installation helpers) for missing dependencies.
        let missing_deps = pkg
            .manifest()
            .depends()
            .unwrap_or_default()
            .into_iter()
            .map(|d| crate::package::extract_name(d).to_lowercase())
            .filter(|n| !installed_names.contains(n))
            .collect::<Vec<_>>();

        out.push(AppStatus {
            name: pkg.name().to_owned(),
            bucket,
            installed_version,
            latest_version: latest.clone(),
            outdated: latest.is_some(),
            failed: pkg.is_install_failed(),
            deprecated,
            removed,
            held: pkg.is_held(),
            missing_deps,
        });
    }

    Ok(out)
}

/// Determine the deprecated / removed state of a bucket-installed app.
///
/// Upstream `app_status` (lib/core.ps1:557-561):
///
/// \`\`text
/// $deprecated_dir = bucket + '\deprecated'
/// $status.deprecated = Get-ChildItem $deprecated_dir -Filter "$app.json" -Recurse
/// $manifest = manifest $app $bucket
/// $status.removed = (!$manifest)   # live manifest gone from the bucket
/// \`\`
///
/// A manifest under the `deprecated` subfolder counts as deprecated but *not*
/// removed (it is still findable by Scoop's recursive `manifest` lookup).
fn probe_origin_bucket(session: &Session, bucket: &str, app: &str) -> (bool, bool) {
    let bucket_dir = session.bucket_dir(bucket);
    if !bucket_dir.exists() {
        // Whole bucket gone -> the manifest is removed.
        return (false, true);
    }

    let filename = format!("{app}.json");
    // Deprecated manifests may sit at <bucket>/bucket/deprecated/ (categorized
    // layout) or directly under a deprecated folder. Mirror Scoop's recursive
    // search under the deprecated directory wherever it appears.
    let deprecated_hit = find_deprecated_manifest(&bucket_dir, &filename);

    // A live manifest elsewhere in the bucket (root / bucket / category),
    // excluding any deprecated folder.
    let live_hit = find_live_manifest(&bucket_dir, &filename);

    (deprecated_hit, !deprecated_hit && !live_hit)
}

/// Depth-first search for `filename` under a `deprecated` subfolder.
fn find_deprecated_manifest(bucket: &Path, filename: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(bucket) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir()
            && path.file_name().is_some_and(|n| n == "deprecated")
            && walk_for_file(&path, filename)
        {
            return true;
        }
    }
    // Also check a nested <bucket>/bucket/deprecated when the top-level walk
    // found nothing (defensive; some buckets nest).
    let nested = bucket.join("bucket").join("deprecated");
    if nested.is_dir() && walk_for_file(&nested, filename) {
        return true;
    }
    false
}

/// Depth-first search for a live manifest, skipping any `deprecated` folder.
fn find_live_manifest(bucket: &Path, filename: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(bucket) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "deprecated") {
                continue;
            }
            if find_live_manifest(&path, filename) {
                return true;
            }
        } else if path.file_name().is_some_and(|n| n == filename) {
            return true;
        }
    }
    false
}

/// Depth-first search for a file named `filename`, stopping when found.
fn walk_for_file(dir: &Path, filename: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if walk_for_file(&path, filename) {
                return true;
            }
        } else if path.file_name().is_some_and(|n| n == filename) {
            return true;
        }
    }
    false
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{mark_installed, test_session, tmpdir, write_bucket_manifest};

    const MINI: &str =
        "{\"version\": \"1.0.0\", \"homepage\": \"https://e.com\", \"license\": \"MIT\"}";

    fn install(root: &std::path::Path, name: &str, bucket: &str, version: &str) {
        mark_installed(
            root,
            name,
            bucket,
            &format!(
                "{{\"version\": \"{}\", \"homepage\": \"https://e.com\", \"license\": \"MIT\"}}",
                version
            ),
            false,
        );
    }

    #[test]
    fn empty_scope_yields_no_status() {
        let root = tmpdir("status_empty");
        let session = test_session(&root);
        assert!(status(&session).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn up_to_date_app_is_healthy() {
        let root = tmpdir("status_healthy");
        let session = test_session(&root);
        write_bucket_manifest(&root, "main", "7zip", MINI);
        install(&root, "7zip", "main", "1.0.0");
        let list = status(&session).unwrap();
        assert_eq!(list.len(), 1);
        let s = &list[0];
        assert_eq!(s.name, "7zip");
        assert_eq!(s.installed_version.as_deref(), Some("1.0.0"));
        assert!(
            !s.outdated && !s.failed && !s.deprecated && !s.removed && s.missing_deps.is_empty()
        );
        assert!(!s.needs_attention());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn outdated_app_reports_latest() {
        let root = tmpdir("status_outdated");
        let session = test_session(&root);
        write_bucket_manifest(&root, "main", "7zip", MINI);
        install(&root, "7zip", "main", "0.9.0");
        let list = status(&session).unwrap();
        let s = &list[0];
        assert!(s.outdated);
        assert_eq!(s.latest_version.as_deref(), Some("1.0.0"));
        assert!(s.needs_attention());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn deprecated_manifest_flagged_not_removed() {
        let root = tmpdir("status_deprecated");
        let session = test_session(&root);
        let dir = root.join("buckets").join("main").join("deprecated");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("legacy.json"), MINI).unwrap();
        install(&root, "legacy", "main", "1.0.0");
        let list = status(&session).unwrap();
        assert_eq!(list.len(), 1);
        assert!(
            list[0].deprecated,
            "app with only a deprecated manifest is deprecated"
        );
        assert!(
            !list[0].removed,
            "deprecated manifest is still findable -> not removed"
        );
        assert!(list[0].needs_attention());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn manifest_removed_from_live_bucket_is_removed() {
        let root = tmpdir("status_removed");
        let session = test_session(&root);
        install(&root, "gone", "main", "1.0.0");
        std::fs::create_dir_all(root.join("buckets").join("main")).unwrap();
        let list = status(&session).unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].removed);
        assert!(list[0].needs_attention());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_dep_reported() {
        let root = tmpdir("status_missing_dep");
        let session = test_session(&root);
        let deps = "{\"version\": \"1.0.0\", \"homepage\": \"https://e.com\", \"license\": \"MIT\", \"depends\": [\"helper\"]}";
        write_bucket_manifest(&root, "main", "a", deps);
        mark_installed(&root, "a", "main", deps, false);
        let list = status(&session).unwrap();
        assert_eq!(list[0].missing_deps, vec!["helper"]);
        assert!(list[0].needs_attention());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn satisfied_dep_not_reported() {
        let root = tmpdir("status_satisfied_dep");
        let session = test_session(&root);
        let deps = "{\"version\": \"1.0.0\", \"homepage\": \"https://e.com\", \"license\": \"MIT\", \"depends\": [\"helper\"]}";
        write_bucket_manifest(&root, "main", "a", deps);
        mark_installed(&root, "a", "main", deps, false);
        install(&root, "helper", "main", "1.0.0");
        let list = status(&session).unwrap();
        let a = list.iter().find(|s| s.name == "a").unwrap();
        assert!(a.missing_deps.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn isolated_install_not_removed_or_deprecated() {
        let root = tmpdir("status_isolated");
        let session = test_session(&root);
        let current = root.join("apps").join("portable").join("current");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("manifest.json"), MINI).unwrap();
        std::fs::write(
            current.join("install.json"),
            "{\"architecture\": \"64bit\", \"url\": \"https://e.com/portable.json\"}",
        )
        .unwrap();
        let list = status(&session).unwrap();
        assert_eq!(list.len(), 1);
        assert!(!list[0].deprecated && !list[0].removed);
        let _ = std::fs::remove_dir_all(&root);
    }
}
