//! Installed-package state resolution.
//!
//! Split from `package/query.rs`: reading an app's install state (version
//! selection, install-record/manifest parsing, upgradability against
//! the origin bucket manifest) is a distinct concern from the query walkers
//! in `query.rs`.
//!
//! This module also owns the two per-version metadata names: Scoop >= 0.6.0
//! writes `scoop-install.json` / `scoop-manifest.json` and reads the new name
//! first with the pre-0.6.0 `install.json` / `manifest.json` as fallback
//! (`lib/manifest.ps1:123-154`). [`install_info_path`], [`installed_manifest_path`]
//! and [`install_info_mtime`] are that rule in one place.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tracing::{info, warn};

use crate::bucket::Bucket;
use crate::constant::ISOLATED_PACKAGE_BUCKET;
use crate::error::Fallible;
use crate::internal::compare_versions;
use crate::package::manifest::{InstallInfo, Manifest, parse_file};
use crate::package::{InstallState, InstallStateInstalled, Package};

use super::query_matcher::QueryOption;

/// Read and parse a version dir's install record into an [`InstallInfo`].
///
/// The file-I/O boundary for the pure [`InstallInfo`] type. The path itself
/// comes from [`install_info_path`].
pub(crate) fn parse_install_info(path: &Path) -> Fallible<InstallInfo> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;

    let info = serde_json::from_slice(&bytes).inspect_err(|e| {
        warn!(
            "failed to parse install_info {} (err: {})",
            path.display(),
            e
        );
    })?;

    Ok(info)
}

/// The install-record name Scoop >= 0.6.0 writes (`save_install_info`,
/// lib/manifest.ps1:141-147).
pub(crate) const SCOOP_INSTALL_JSON: &str = "scoop-install.json";
/// The installed-manifest name Scoop >= 0.6.0 writes (`save_installed_manifest`,
/// lib/manifest.ps1:123-132).
pub(crate) const SCOOP_MANIFEST_JSON: &str = "scoop-manifest.json";
/// The pre-0.6.0 install-record name, still read as a fallback.
pub(crate) const LEGACY_INSTALL_JSON: &str = "install.json";
/// The pre-0.6.0 manifest name, still read as a fallback.
pub(crate) const LEGACY_MANIFEST_JSON: &str = "manifest.json";

/// Resolve a version dir's metadata file: the new name when it exists, the
/// legacy name otherwise — upstream's `Test-Path` then fallback
/// (lib/manifest.ps1:136-138 and :151-153). `None` when neither exists.
fn metadata_path(dir: &Path, current: &str, legacy: &str) -> Option<PathBuf> {
    let current_path = dir.join(current);
    if current_path.exists() {
        return Some(current_path);
    }
    let legacy_path = dir.join(legacy);
    legacy_path.exists().then_some(legacy_path)
}

/// The install record of a version dir: `<dir>/scoop-install.json` when it
/// exists, else the legacy `<dir>/install.json` — upstream `install_info`
/// (lib/manifest.ps1:149-154). `None` when neither exists.
pub(crate) fn install_info_path(dir: &Path) -> Option<PathBuf> {
    metadata_path(dir, SCOOP_INSTALL_JSON, LEGACY_INSTALL_JSON)
}

/// The installed manifest of a version dir: `<dir>/scoop-manifest.json` when it
/// exists, else the legacy `<dir>/manifest.json` — upstream `installed_manifest`
/// (lib/manifest.ps1:134-139). `None` when neither exists.
pub(crate) fn installed_manifest_path(dir: &Path) -> Option<PathBuf> {
    metadata_path(dir, SCOOP_MANIFEST_JSON, LEGACY_MANIFEST_JSON)
}

/// The install record's modification time — the ordering key of upstream
/// `Get-InstalledVersion` (lib/versions.ps1:99-100), which globs both names.
/// `None` when the dir carries no install record.
///
/// Divergence from upstream, deliberate: a dir carrying *both* names (only
/// reachable by mixing a pre-0.6.0 write with a later one into the same
/// version dir) has two LastWriteTimes, and upstream's sort-then-`Select-Object
/// -Unique` makes the *older* one decide the dir's position; here the resolved
/// file's time is used. Pathological mixture, no semantic worth pinning.
pub(crate) fn install_info_mtime(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(install_info_path(dir)?)
        .and_then(|meta| meta.modified())
        .ok()
}

/// Where a resolved current version came from — the upstream
/// `Select-CurrentVersion` branch that produced it.
///
/// Callers that only need the version string can ignore this; callers that
/// must distinguish "resolved from an intact install" from "fell back to a
/// version dir" (the broken-install detection in `query.rs`) match on it
/// instead of re-implementing the resolution to find out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CurrentVersionSource {
    /// `current\scoop-manifest.json` (else `current\manifest.json`) carried a
    /// concrete (non-`nightly`) version.
    CurrentManifest,
    /// That manifest said `nightly`; the version is the junction
    /// target's directory name.
    NightlyJunction,
    /// Versions were resolved from the version dirs' install-record mtimes
    /// (`Get-InstalledVersion` fallback). Reached when the metadata under
    /// `current\` is absent or unreadable, and always under `NO_JUNCTION`,
    /// which has no `current` junction at all.
    VersionDirs,
}

/// A resolved current version plus the branch it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CurrentVersion {
    /// The resolved version directory name.
    pub(crate) version: String,
    /// Which `Select-CurrentVersion` branch resolved it.
    pub(crate) source: CurrentVersionSource,
}

impl CurrentVersion {
    fn manifest(version: String) -> Self {
        Self {
            version,
            source: CurrentVersionSource::CurrentManifest,
        }
    }

    fn nightly(version: String) -> Self {
        Self {
            version,
            source: CurrentVersionSource::NightlyJunction,
        }
    }

    fn fallback(version: String) -> Self {
        Self {
            version,
            source: CurrentVersionSource::VersionDirs,
        }
    }
}

/// The single entry point for answering "which version of this package is
/// current?" — the hok counterpart of upstream `Select-CurrentVersion`
/// (lib/versions.ps1:31-69).
///
/// `no_junction` carries upstream's mode switch (`versions.ps1:53`), and the
/// switch lives **inside** this function exactly as it does upstream; no
/// caller may re-derive it. Resolving one version one way in some flows and
/// another way in others is how a package ends up with two different
/// "current" answers (and is why `cleanup` could delete the live version
/// dir — see the module tests).
///
/// The branches, in upstream order:
///
/// 1. `current\scoop-manifest.json` (else `current\manifest.json`)'s `version`,
///    read **only when junctions are enabled**. A `nightly` version — or a
///    junction sitting there without a usable manifest (an interrupted upgrade)
///    — resolves to the junction target's directory name when it can be read.
/// 2. Otherwise the version dir whose install record (`scoop-install.json`,
///    else `install.json`) was modified most recently, excluding `current` and
///    `_*.old*` (upstream `Get-InstalledVersion`, `versions.ps1:71-105`).
///
/// Returns `None` when neither branch yields a version (nothing installed, or
/// an app dir whose version dirs carry no install record).
///
/// Divergence from upstream, deliberately not emulated: when several version
/// dirs share one install-record mtime, upstream's order comes out of
/// `Sort-Object` and its pipeline plumbing (observed to reverse enumeration
/// order, `1.0.0,2.0.0` → last `1.0.0`); here the last enumerated dir wins.
/// Same predicate, nondeterministic order — not a semantic to pin.
pub(crate) fn resolve_current_version(pkg_dir: &Path, no_junction: bool) -> Option<CurrentVersion> {
    // 1. `current\scoop-manifest.json` (else `current\manifest.json`) —
    //    upstream reads it only when the junction mode is on
    //    (`versions.ps1:53-56`). Under NO_JUNCTION a leftover `current`
    //    directory is a stale artifact, not the active version.
    if !no_junction {
        let current_dir = pkg_dir.join("current");
        if let Some(path) = installed_manifest_path(&current_dir)
            && let Ok(manifest) = parse_file(path)
        {
            let version = manifest.version().to_owned();
            if version != "nightly" {
                return Some(CurrentVersion::manifest(version));
            }
            if let Ok(target) = std::fs::read_link(&current_dir)
                && let Some(name) = target.file_name().and_then(|s| s.to_str())
            {
                return Some(CurrentVersion::nightly(name.to_owned()));
            }
        }
    }

    // 2. version dirs with an install record, newest by modification time
    newest_version_dir(pkg_dir).map(CurrentVersion::fallback)
}

/// The version dir whose install record was modified most recently, excluding
/// `current` and `_*.old*` (upstream `Get-InstalledVersion`).
fn newest_version_dir(pkg_dir: &Path) -> Option<String> {
    let mut candidates: Vec<(SystemTime, String)> = vec![];
    if let Ok(entries) = std::fs::read_dir(pkg_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "current" || (name.starts_with('_') && name.contains(".old")) {
                continue;
            }
            if let Some(time) = install_info_mtime(&entry.path()) {
                candidates.push((time, name));
            }
        }
    }
    candidates.sort_by_key(|c| c.0);
    candidates.pop().map(|(_, v)| v)
}

/// The version dir a package's metadata lives in: `current` under junctions,
/// otherwise the resolved version dir (upstream `currentdir`, core.ps1:355).
pub(crate) fn current_meta_dir(
    pkg_dir: &Path,
    no_junction: bool,
) -> Option<(PathBuf, CurrentVersionSource)> {
    let resolved = resolve_current_version(pkg_dir, no_junction)?;
    let dir = if no_junction {
        pkg_dir.join(&resolved.version)
    } else {
        pkg_dir.join("current")
    };
    Some((dir, resolved.source))
}

pub(crate) fn load_install_state(
    apps_dir: &Path,
    name: &str,
    no_junction: bool,
) -> Option<InstallState> {
    let pkg_dir = apps_dir.join(name);
    // Under NO_JUNCTION there is no `current` junction: resolve the current
    // version via the shared resolver and read the versioned dir's metadata
    // (upstream `currentdir`, lib/core.ps1).
    let (meta_dir, _) = current_meta_dir(&pkg_dir, no_junction)?;
    let install_info = parse_install_info(&install_info_path(&meta_dir)?).ok()?;
    let install_manifest = parse_file(installed_manifest_path(&meta_dir)?).ok()?;

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

/// The on-disk path of `bucket/name`'s manifest, when that bucket exists and
/// declares a manifest for the name.
///
/// Shared by [`load_bucket_manifest`] and the query layer: a caller that has
/// to report *why* a manifest could not be used needs the path, not just the
/// parsed result (or the absence of one).
pub(crate) fn bucket_manifest_path(root_path: &Path, bucket: &str, name: &str) -> Option<PathBuf> {
    let bucket_path = crate::internal::path::bucket_dir(root_path, bucket);
    let bucket = Bucket::from(&bucket_path).ok()?;
    bucket.path_of_manifest(name)
}

pub(crate) fn load_bucket_manifest(root_path: &Path, bucket: &str, name: &str) -> Option<Manifest> {
    parse_file(bucket_manifest_path(root_path, bucket, name)?).ok()
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
    use std::time::{Duration, SystemTime};

    fn mini_manifest(version: &str) -> String {
        format!(
            r#"{{"version": "{}", "homepage": "https://example.com", "license": "MIT"}}"#,
            version
        )
    }

    /// Create a package dir under a fresh temp root and return both.
    fn test_pkg(test_name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = test_utils::tmpdir(test_name);
        let pkg_dir = root.join("apps").join("app");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        (root, pkg_dir)
    }

    /// Create `apps/app/<version>/{manifest,install}.json` with a pinned
    /// `install.json` mtime (the resolver's ordering key).
    fn add_version(pkg_dir: &Path, version: &str, mtime_secs: u64) {
        add_version_with(
            pkg_dir,
            version,
            mtime_secs,
            LEGACY_MANIFEST_JSON,
            LEGACY_INSTALL_JSON,
        );
    }

    /// [`add_version`] with explicit metadata file names.
    fn add_version_with(
        pkg_dir: &Path,
        version: &str,
        mtime_secs: u64,
        manifest_name: &str,
        install_name: &str,
    ) {
        let dir = pkg_dir.join(version);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(manifest_name), mini_manifest(version)).unwrap();
        std::fs::write(
            dir.join(install_name),
            r#"{"architecture": "64bit", "bucket": "test"}"#,
        )
        .unwrap();
        set_modified(
            &dir.join(install_name),
            SystemTime::UNIX_EPOCH + Duration::from_secs(mtime_secs),
        );
    }

    /// Create `apps/app/<version>` with no metadata at all.
    fn add_bare_version(pkg_dir: &Path, version: &str) {
        std::fs::create_dir_all(pkg_dir.join(version)).unwrap();
    }

    fn set_modified(path: &Path, time: SystemTime) {
        use std::fs::{File, FileTimes};
        let f = File::options().write(true).open(path).unwrap();
        f.set_times(FileTimes::new().set_modified(time)).unwrap();
    }

    fn write_manifest(dir: &Path, version: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("manifest.json"), mini_manifest(version)).unwrap();
    }

    fn assert_resolved(
        actual: Option<CurrentVersion>,
        version: &str,
        source: CurrentVersionSource,
    ) {
        let actual = actual.expect("expected a resolved current version");
        assert_eq!(actual.version, version);
        assert_eq!(actual.source, source);
    }

    // Fixture results below are pinned against upstream Scoop's
    // `Select-CurrentVersion` / `Get-InstalledVersion`, executed on Windows
    // PowerShell 5.1 over the same fixture shapes (see the official ground
    // truth table in .agents/docs/active/audit/optimization-audit-2026-09.md).

    /// Junction mode, intact `current\manifest.json` → its `version`.
    /// Official: `J1-junction-normal` → `1.2.3`.
    #[test]
    fn junction_mode_reads_current_manifest() {
        let (root, pkg_dir) = test_pkg("resolve_junction_manifest");
        add_version(&pkg_dir, "1.0.0", 1_000);
        add_version(&pkg_dir, "1.2.3", 2_000);
        crate::internal::fs::symlink_dir(pkg_dir.join("1.2.3"), pkg_dir.join("current")).unwrap();

        assert_resolved(
            resolve_current_version(&pkg_dir, false),
            "1.2.3",
            CurrentVersionSource::CurrentManifest,
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Junction mode, `nightly` manifest → the junction target's dir name.
    /// Official: `J2-junction-nightly` → `1.2.3-nightly`.
    #[test]
    fn junction_mode_nightly_follows_link_target() {
        let (root, pkg_dir) = test_pkg("resolve_junction_nightly");
        add_version(&pkg_dir, "1.2.3-nightly", 1_000);
        crate::internal::fs::symlink_dir(pkg_dir.join("1.2.3-nightly"), pkg_dir.join("current"))
            .unwrap();
        write_manifest(&pkg_dir.join("current"), "nightly");

        assert_resolved(
            resolve_current_version(&pkg_dir, false),
            "1.2.3-nightly",
            CurrentVersionSource::NightlyJunction,
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Junction mode, `current` exists with no `manifest.json` (interrupted
    /// upgrade) → mtime fallback, flagged as such. Official:
    /// `J3-junction-no-manifest` → `2.0.0`.
    #[test]
    fn junction_mode_broken_current_falls_back_to_version_dirs() {
        let (root, pkg_dir) = test_pkg("resolve_junction_no_manifest");
        add_version(&pkg_dir, "1.0.0", 1_000);
        add_version(&pkg_dir, "2.0.0", 2_000);
        crate::internal::fs::symlink_dir(pkg_dir.join("2.0.0"), pkg_dir.join("current")).unwrap();
        std::fs::remove_file(pkg_dir.join("current").join("manifest.json")).unwrap();

        assert_resolved(
            resolve_current_version(&pkg_dir, false),
            "2.0.0",
            CurrentVersionSource::VersionDirs,
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Junction mode, unparseable manifest → mtime fallback (upstream `warn`s
    /// and falls through). Official: `J4-junction-garbage` → `2.0.0`.
    #[test]
    fn junction_mode_garbage_manifest_falls_back_to_version_dirs() {
        let (root, pkg_dir) = test_pkg("resolve_junction_garbage");
        add_version(&pkg_dir, "1.0.0", 1_000);
        add_version(&pkg_dir, "2.0.0", 2_000);
        crate::internal::fs::symlink_dir(pkg_dir.join("2.0.0"), pkg_dir.join("current")).unwrap();
        std::fs::write(pkg_dir.join("current").join("manifest.json"), "{ not json").unwrap();

        assert_resolved(
            resolve_current_version(&pkg_dir, false),
            "2.0.0",
            CurrentVersionSource::VersionDirs,
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// NO_JUNCTION, two version dirs → newest `install.json` mtime.
    /// Official: `NJ5-two-versions` → `2.0.0`.
    #[test]
    fn no_junction_picks_newest_version_dir() {
        let (root, pkg_dir) = test_pkg("resolve_no_junction_newest");
        add_version(&pkg_dir, "1.0.0", 1_000);
        add_version(&pkg_dir, "2.0.0", 2_000);

        assert_resolved(
            resolve_current_version(&pkg_dir, true),
            "2.0.0",
            CurrentVersionSource::VersionDirs,
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// NO_JUNCTION ignores a leftover `current\manifest.json` — it is a stale
    /// artifact, not the active version. Official: `NJ4-stale-current` →
    /// `2.0.0` (0.9.9 ignored). Before the mode switch moved inside the
    /// resolver, hok answered `0.9.9` here, and `cleanup`/`reset` acted on it.
    #[test]
    fn no_junction_ignores_stale_current_manifest() {
        let (root, pkg_dir) = test_pkg("resolve_no_junction_stale_current");
        add_version(&pkg_dir, "1.0.0", 1_000);
        add_version(&pkg_dir, "2.0.0", 2_000);
        write_manifest(&pkg_dir.join("current"), "0.9.9");

        assert_resolved(
            resolve_current_version(&pkg_dir, true),
            "2.0.0",
            CurrentVersionSource::VersionDirs,
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Neither branch yields a version: no `install.json` anywhere.
    #[test]
    fn resolve_current_version_none_without_candidates() {
        let (root, pkg_dir) = test_pkg("resolve_none");
        add_bare_version(&pkg_dir, "6.0.20261.7291");

        assert_eq!(resolve_current_version(&pkg_dir, false), None);
        assert_eq!(resolve_current_version(&pkg_dir, true), None);
        assert_eq!(current_meta_dir(&pkg_dir, false), None);
        assert_eq!(current_meta_dir(&pkg_dir, true), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Equal mtimes are order-nondeterministic upstream (not a semantic worth
    /// pinning); assert only that a real candidate resolves.
    #[test]
    fn resolve_current_version_tolerates_equal_mtimes() {
        let (root, pkg_dir) = test_pkg("resolve_tie_mtime");
        add_version(&pkg_dir, "1.0.0", 1_000);
        add_version(&pkg_dir, "2.0.0", 1_000);

        let resolved = resolve_current_version(&pkg_dir, true).unwrap();
        assert!(
            resolved.version == "1.0.0" || resolved.version == "2.0.0",
            "unexpected version {}",
            resolved.version
        );
        assert_eq!(resolved.source, CurrentVersionSource::VersionDirs);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `current_meta_dir` points at the junction under junctions and at the
    /// resolved version dir under NO_JUNCTION.
    #[test]
    fn current_meta_dir_follows_mode() {
        let (root, pkg_dir) = test_pkg("resolve_meta_dir");
        add_version(&pkg_dir, "1.2.3", 1_000);
        crate::internal::fs::symlink_dir(pkg_dir.join("1.2.3"), pkg_dir.join("current")).unwrap();

        let (junction_dir, junction_source) = current_meta_dir(&pkg_dir, false).unwrap();
        assert_eq!(junction_dir, pkg_dir.join("current"));
        assert_eq!(junction_source, CurrentVersionSource::CurrentManifest);

        let (version_dir, version_source) = current_meta_dir(&pkg_dir, true).unwrap();
        assert_eq!(version_dir, pkg_dir.join("1.2.3"));
        assert_eq!(version_source, CurrentVersionSource::VersionDirs);
        let _ = std::fs::remove_dir_all(&root);
    }

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

    // ── Scoop >= 0.6.0 metadata names ─────────────────────────────────────
    // (`scoop-manifest.json` / `scoop-install.json`, read new-first with the
    //  pre-0.6.0 names as fallback — lib/manifest.ps1:134-154)

    /// A version dir carrying ONLY the new names is a first-class install:
    /// junction mode reads `current\scoop-manifest.json`'s version, and the
    /// mtime fallback enumerates the dir through `scoop-install.json`.
    #[test]
    fn resolve_current_version_reads_scoop_names_only() {
        let (root, pkg_dir) = test_pkg("resolve_scoop_names");
        add_version_with(
            &pkg_dir,
            "1.0.0",
            1_000,
            SCOOP_MANIFEST_JSON,
            SCOOP_INSTALL_JSON,
        );
        add_version_with(
            &pkg_dir,
            "2.0.0",
            2_000,
            SCOOP_MANIFEST_JSON,
            SCOOP_INSTALL_JSON,
        );
        assert!(
            !pkg_dir.join("2.0.0").join(LEGACY_MANIFEST_JSON).exists(),
            "fixture must carry the new names only"
        );

        // NO_JUNCTION: a new-name-only version dir is still a candidate.
        assert_resolved(
            resolve_current_version(&pkg_dir, true),
            "2.0.0",
            CurrentVersionSource::VersionDirs,
        );

        // Junction mode: the version comes from `current\scoop-manifest.json`.
        crate::internal::fs::symlink_dir(pkg_dir.join("1.0.0"), pkg_dir.join("current")).unwrap();
        assert_resolved(
            resolve_current_version(&pkg_dir, false),
            "1.0.0",
            CurrentVersionSource::CurrentManifest,
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// With both names present the new one wins, for both files.
    #[test]
    fn scoop_names_win_over_legacy_names() {
        let (root, pkg_dir) = test_pkg("resolve_names_precedence");
        let dir = pkg_dir.join("1.0.0");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(SCOOP_MANIFEST_JSON), mini_manifest("9.9.9")).unwrap();
        std::fs::write(dir.join(LEGACY_MANIFEST_JSON), mini_manifest("1.0.0")).unwrap();
        std::fs::write(
            dir.join(SCOOP_INSTALL_JSON),
            r#"{"architecture": "64bit", "bucket": "new"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join(LEGACY_INSTALL_JSON),
            r#"{"architecture": "32bit", "bucket": "old"}"#,
        )
        .unwrap();
        crate::internal::fs::symlink_dir(&dir, pkg_dir.join("current")).unwrap();

        assert_resolved(
            resolve_current_version(&pkg_dir, false),
            "9.9.9",
            CurrentVersionSource::CurrentManifest,
        );

        let state = load_install_state(&root.join("apps"), "app", false).unwrap();
        match state {
            InstallState::Installed(s) => {
                assert_eq!(s.bucket.as_deref(), Some("new"), "new record wins");
                assert_eq!(s.arch, "64bit");
            }
            _ => panic!("expected installed state"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The path resolvers fall back to the legacy names and answer `None` only
    /// when neither name exists.
    #[test]
    fn metadata_path_resolution_prefers_new_then_legacy() {
        let root = test_utils::tmpdir("metadata_path_resolution");
        let dir = root.join("apps").join("app").join("1.0.0");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(install_info_path(&dir), None);
        assert_eq!(installed_manifest_path(&dir), None);
        assert_eq!(install_info_mtime(&dir), None);

        std::fs::write(dir.join(LEGACY_INSTALL_JSON), "{}").unwrap();
        std::fs::write(dir.join(LEGACY_MANIFEST_JSON), "{}").unwrap();
        assert_eq!(install_info_path(&dir), Some(dir.join(LEGACY_INSTALL_JSON)));
        assert_eq!(
            installed_manifest_path(&dir),
            Some(dir.join(LEGACY_MANIFEST_JSON))
        );
        assert!(install_info_mtime(&dir).is_some());

        std::fs::write(dir.join(SCOOP_INSTALL_JSON), "{}").unwrap();
        std::fs::write(dir.join(SCOOP_MANIFEST_JSON), "{}").unwrap();
        assert_eq!(install_info_path(&dir), Some(dir.join(SCOOP_INSTALL_JSON)));
        assert_eq!(
            installed_manifest_path(&dir),
            Some(dir.join(SCOOP_MANIFEST_JSON))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `load_install_state` reads a new-name-only version dir under NO_JUNCTION.
    #[test]
    fn load_install_state_reads_scoop_names() {
        let root = test_utils::tmpdir("load_state_scoop_names");
        let vdir = root.join("apps").join("app").join("1.2.3");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join(SCOOP_MANIFEST_JSON), mini_manifest("1.2.3")).unwrap();
        std::fs::write(
            vdir.join(SCOOP_INSTALL_JSON),
            r#"{"architecture": "64bit", "bucket": "main", "hold": true}"#,
        )
        .unwrap();

        let state = load_install_state(&root.join("apps"), "app", true).unwrap();
        match state {
            InstallState::Installed(s) => {
                assert_eq!(s.version, "1.2.3");
                assert_eq!(s.bucket.as_deref(), Some("main"));
                assert!(s.held);
            }
            _ => panic!("expected installed state"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
