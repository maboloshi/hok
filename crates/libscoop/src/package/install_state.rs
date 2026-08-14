//! Installed-package state resolution.
//!
//! Split from `package/query.rs`: reading an app's install state (version
//! selection, `install.json`/`manifest.json` parsing, upgradability against
//! the origin bucket manifest) is a distinct concern from the query walkers
//! in `query.rs`.

use std::path::Path;
use std::time::SystemTime;
use tracing::info;

use crate::bucket::Bucket;
use crate::constant::ISOLATED_PACKAGE_BUCKET;
use crate::internal::compare_versions;
use crate::package::manifest::{InstallInfo, Manifest};
use crate::package::{InstallState, InstallStateInstalled, Package};

use super::query_matcher::QueryOption;

/// Resolve the current installed version of an app, mirroring upstream
/// `Select-CurrentVersion` (lib/versions.ps1):
///
/// 1. `current\manifest.json`'s `version` — a `nightly` version resolves to
///    the junction target's directory name;
/// 2. otherwise, the version directory whose `install.json` was most recently
///    modified (`Get-InstalledVersion`, excluding `current` and `_*.old*`).
///
/// Under `NO_JUNCTION` there is no `current` junction, so step 1 fails and
/// the mtime fallback of step 2 (which scans the versioned dirs) applies.
pub(crate) fn select_current_version(pkg_dir: &Path) -> Option<String> {
    // 1. current\manifest.json version
    if let Ok(manifest) = Manifest::parse(pkg_dir.join("current").join("manifest.json")) {
        let version = manifest.version().to_owned();
        if version == "nightly" {
            if let Ok(target) = std::fs::read_link(pkg_dir.join("current")) {
                if let Some(name) = target.file_name().and_then(|s| s.to_str()) {
                    return Some(name.to_owned());
                }
            }
        } else {
            return Some(version);
        }
    }

    // 2. version dirs with install.json, newest by modification time
    let mut candidates: Vec<(SystemTime, String)> = vec![];
    if let Ok(entries) = std::fs::read_dir(pkg_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "current" || (name.starts_with('_') && name.contains(".old")) {
                continue;
            }
            let install_json = entry.path().join("install.json");
            if let Ok(meta) = std::fs::metadata(&install_json) {
                let time = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                candidates.push((time, name));
            }
        }
    }
    candidates.sort_by_key(|c| c.0);
    candidates.pop().map(|(_, v)| v)
}

pub(crate) fn load_install_state(
    apps_dir: &Path,
    name: &str,
    no_junction: bool,
) -> Option<InstallState> {
    let pkg_dir = apps_dir.join(name);
    // Under NO_JUNCTION there is no `current` junction: resolve the current
    // version via the `Select-CurrentVersion` fallback and read the versioned
    // dir's metadata (upstream `currentdir`, lib/core.ps1).
    let meta_dir = if no_junction {
        pkg_dir.join(select_current_version(&pkg_dir)?)
    } else {
        pkg_dir.join("current")
    };
    let install_info = InstallInfo::parse(meta_dir.join("install.json")).ok()?;
    let install_manifest = Manifest::parse(meta_dir.join("manifest.json")).ok()?;

    Some(InstallState::Installed(InstallStateInstalled {
        version: install_manifest.version().to_owned(),
        bucket: install_info.bucket().map(|s| s.to_owned()),
        arch: install_info.arch().to_owned(),
        held: install_info.is_held(),
        url: install_info.url().map(|s| s.to_owned()),
    }))
}

pub(crate) fn fill_install_state(
    package: &Package,
    apps_dir: &Path,
    name: &str,
    no_junction: bool,
) {
    package.fill_install_state(
        load_install_state(apps_dir, name, no_junction).unwrap_or(InstallState::NotInstalled),
    );
}

pub(crate) fn load_bucket_manifest(root_path: &Path, bucket: &str, name: &str) -> Option<Manifest> {
    let bucket_path = crate::internal::path::bucket_dir(root_path, bucket);
    let bucket = Bucket::from(&bucket_path).ok()?;
    let manifest_path = bucket.path_of_manifest(name)?;
    Manifest::parse(manifest_path).ok()
}

pub(crate) fn maybe_fill_upgradable(
    root_path: &Path,
    package: &Package,
    name: &str,
    bucket: &str,
    current_version: &str,
    state: &InstallState,
    options: &[QueryOption],
) -> bool {
    let filter_non_upgradable = options.contains(&QueryOption::Upgradable);
    let check_upgradable = filter_non_upgradable || options.contains(&QueryOption::UpgradableCheck);

    if !check_upgradable {
        return true;
    }

    if bucket == ISOLATED_PACKAGE_BUCKET {
        if filter_non_upgradable {
            info!("ignored isolated package '{}'", name);
            return false;
        }
        return true;
    }

    let Some(origin_manifest) = load_bucket_manifest(root_path, bucket, name) else {
        return !filter_non_upgradable;
    };

    let is_upgradable =
        compare_versions(origin_manifest.version(), current_version) == std::cmp::Ordering::Greater;

    if !is_upgradable {
        return !filter_non_upgradable;
    }

    let origin_pkg = Package::from(name, bucket, origin_manifest);
    origin_pkg.fill_install_state(state.clone());
    package.fill_upgradable(origin_pkg);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils;

    #[test]
    fn load_install_state_no_junction_reads_version_dir() {
        let root = test_utils::tmpdir("query_no_junction_state");
        let vdir = root.join("apps").join("app").join("1.2.3");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(
            vdir.join("manifest.json"),
            r#"{"version": "1.2.3", "homepage": "https://example.com", "license": "MIT"}"#,
        )
        .unwrap();
        std::fs::write(
            vdir.join("install.json"),
            r#"{"architecture": "64bit", "bucket": "test"}"#,
        )
        .unwrap();

        let state = load_install_state(&root.join("apps"), "app", true).unwrap();
        match state {
            InstallState::Installed(s) => assert_eq!(s.version, "1.2.3"),
            _ => panic!("expected installed state"),
        }
        // With junctions enabled and no `current` link, nothing resolves.
        assert!(load_install_state(&root.join("apps"), "app", false).is_none());
    }
}
