//! Uninstall / reset pipeline of the sync transaction.
//!
//! This module contains the remove and reset flows — uninstall scripts,
//! shim/shortcut/env/persist cleanup, app-directory removal, and version
//! reset — split out of [`super`] (`sync.rs`). It is a private sub-module
//! of `sync`; the entry points are re-exported by `sync.rs`.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tracing::{debug, info};

use crate::constant::ISOLATED_PACKAGE_BUCKET;
use crate::package::{
    manifest::{InstallInfo, Manifest},
    operations, query, resolve, InstallState, InstallStateInstalled, Package,
};
use crate::{env, error::Fallible, internal, psmodule, shim, Error, Event, QueryOption, Session};

use super::{confirm_transaction, SyncOption, Transaction};
use super::sync_install::{check_not_running, expand_installer_vars, run_script};

/// Sync operation: remove packages.
pub fn remove(session: &Session, queries: &[&str], options: &[SyncOption]) -> Fallible<()> {
    let escape_hold = options.contains(&SyncOption::EscapeHold);
    let no_dependent_check = options.contains(&SyncOption::NoDependentCheck);
    let ignore_failure = options.contains(&SyncOption::IgnoreFailure);

    // Query target packages directly instead of scanning all installed.
    // Dependency checking (below) does the full scan when needed.
    let mut packages = vec![];
    for &name in queries {
        let matched = query::query_installed(session, &[name], &[QueryOption::Explicit])?;
        if matched.is_empty() {
            if ignore_failure {
                eprintln!(
                    "failed to remove '{}': {}",
                    name,
                    Error::PackageNotFound(name.to_string())
                );
                continue;
            }
            return Err(Error::PackageNotFound(name.to_string()));
        }
        let pkg = matched.into_iter().next().unwrap();
        if pkg.is_held() && !escape_hold {
            continue;
        }
        packages.push(pkg);
    }

    if !no_dependent_check {
        let installed = query::query_installed(session, &["*"], &[])?;
        let mut dependents = vec![];

        for pkg in packages.iter() {
            let mut result = installed
                .iter()
                .filter_map(|p| {
                    if packages.contains(p) {
                        return None;
                    }

                    let dep_names = p
                        .dependencies()
                        .into_iter()
                        .map(crate::package::extract_name)
                        .collect::<Vec<_>>();

                    if dep_names.contains(&pkg.name().to_owned()) {
                        // p depends on pkg
                        Some((p.name().to_owned(), pkg.name().to_owned()))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();

            if result.is_empty() {
                continue;
            }

            dependents.append(&mut result);
        }

        if !dependents.is_empty() {
            return Err(Error::PackageDependentFound(dependents));
        }
    }

    let is_cascade = options.contains(&SyncOption::Cascade);
    if is_cascade {
        resolve::resolve_cascade(session, &mut packages, escape_hold)?;
    }

    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackageResolveDone);
    }

    let transaction = Transaction::default();

    let (_packages_with_script, _packages): (Vec<_>, Vec<_>) =
        packages.iter().partition(|p| p.has_uninstall_script());

    transaction.set_remove(packages);

    let assume_yes = options.contains(&SyncOption::AssumeYes);
    if !assume_yes && !confirm_transaction(session, &transaction)? {
        return Ok(());
    }

    if let Some(packages) = transaction.remove_view() {
        let purge = options.contains(&SyncOption::Purge);
        let ignore_failure = options.contains(&SyncOption::IgnoreFailure);
        commit_remove(session, packages, purge, ignore_failure)?;
    }

    Ok(())
}

/// Execute the removal commit: run scripts, clean up shims/shortcuts/env,
/// remove app directory, and optionally purge persist data.
fn commit_remove(
    session: &Session,
    packages: &[Package],
    purge: bool,
    ignore_failure: bool,
) -> Fallible<()> {
    for package in packages.iter() {
        if let Err(e) = commit_one_remove(session, package, purge) {
            let msg = format!("failed to remove '{}': {}", package.name(), e);
            if ignore_failure {
                eprintln!("{}", msg);
                continue;
            }
            return Err(Error::Custom(msg));
        }
    }
    Ok(())
}

fn commit_one_remove(session: &Session, package: &Package, purge: bool) -> Fallible<()> {
    let root_dir = session.effective_root_path();

    debug!("remove: {} - starting", package.name());

    // Check if the app is currently running before proceeding
    check_not_running(session, package.name(), "uninstalling")?;

    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackageCommitStart(package.name().to_owned()));
    }

    let app_dir = root_dir.join("apps").join(package.name());

    run_script(
        session,
        package,
        &app_dir.join("current"),
        "pre_uninstall",
        "uninstall",
        package.manifest().pre_uninstall(),
    )?;

    if let Some(uninstaller) = package.manifest().uninstaller() {
        if let Some(script) = uninstaller.script() {
            run_script(
                session,
                package,
                &app_dir.join("current"),
                "uninstaller",
                "uninstall",
                Some(script),
            )?;
        } else if let Some(file) = uninstaller.file() {
            debug!("remove: {} - uninstaller.file", package.name());
            let exe_path = app_dir.join("current").join(file);
            let raw_args: Vec<&str> = uninstaller.args().unwrap_or_default();
            let expanded = expand_installer_vars(
                &raw_args,
                session,
                package,
                &app_dir.join("current"),
                "uninstall",
            );
            let args: Vec<&str> = expanded.iter().map(|s| s.as_str()).collect();
            crate::internal::os::run_gui(&exe_path, &args, Some(&app_dir.join("current")))
                .map_err(|e| {
                    Error::Custom(format!(
                        "failed to run uninstaller '{}' for '{}': {}",
                        file,
                        package.name(),
                        e
                    ))
                })?;
        }
    }

    debug!(
        "remove: {} - cleanup (shims/shortcuts/env/persist)",
        package.name()
    );
    shim::remove(session, package)?;
    operations::shortcut_remove(session, package)?;
    psmodule::remove(session, package)?;
    env::remove(session, package)?;
    operations::persist_unlink(session, package)?;

    operations::unlink_current(&app_dir)?;

    run_script(
        session,
        package,
        &app_dir,
        "post_uninstall",
        "uninstall",
        package.manifest().post_uninstall(),
    )?;

    internal::fs::remove_dir(&app_dir)?;

    if purge {
        operations::persist_purge(session, package.name())?;
    }

    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackageCommitDone(package.name().to_owned()));
    }

    Ok(())
}

/// Reset a package: re-link current version, re-create shims/shortcuts,
/// and run post_install. Unlike Scoop's original reset, this runs
/// post_install to reapply localization settings.
///
/// Package resolution mirrors upstream `scoop reset` (libexec/scoop-reset.ps1):
/// the app is considered installed when `apps/<name>` exists and the current
/// version can be resolved — reading `current\manifest.json`'s `version` first
/// (`Select-CurrentVersion`), then falling back to version directories with an
/// `install.json`, newest by modification time (`Get-InstalledVersion`).
/// Unlike `query_installed`, this does **not** require both
/// `current\manifest.json` and `current\install.json` to exist, so a
/// half-broken install (e.g. missing `install.json`) can still be reset.
pub fn reset(session: &Session, name: &str, target_version: Option<&str>) -> Fallible<()> {
    let (pkg, pkg_dir, version_dir) = resolve_reset_target(session, name, target_version)?;
    let version = pkg.installed_version().unwrap_or(pkg.version());

    info!("resetting {} ({})", name, version);

    // Re-create the `current` symlink
    operations::link_current(&pkg_dir, &version_dir)?;

    // Re-link persistent data
    operations::persist_link(session, &pkg)?;

    // Re-create shims + shortcuts
    shim::remove(session, &pkg)?;
    shim::add(session, &pkg)?;
    operations::shortcut_remove(session, &pkg)?;
    operations::shortcut_add(session, &pkg)?;

    // Re-apply env (mirrors scoop-reset.ps1: env_rm_path/env_rm then
    // env_add_path/env_set — unset all potential old env before re-adding)
    env::remove(session, &pkg)?;
    env::add(session, &pkg)?;

    // Run post_install to reapply localization (fixes Scoop bug)
    run_script(
        session,
        &pkg,
        &version_dir,
        "post_install",
        "install",
        pkg.manifest().post_install(),
    )?;

    Ok(())
}

/// Resolve the package and version directory a `reset` should operate on,
/// tolerating broken installs the same way upstream Scoop does.
///
/// - `apps/<name>` missing or version unresolvable → [`Error::PackageNotFound`]
/// - explicit `target_version` → that version directory (must exist; rejected
///   if it contains path separators or `..`)
/// - otherwise → [`select_current_version`]
/// - manifest is parsed from the **version directory** (`installed_manifest`),
///   not from `current/`, so a stale/broken `current` junction doesn't matter
/// - `install.json` is optional (`install_info` returns `$null` when missing);
///   bucket defaults to [`ISOLATED_PACKAGE_BUCKET`]
///
/// Returns `(package, apps/<name> dir, <version> dir)`.
fn resolve_reset_target(
    session: &Session,
    name: &str,
    target_version: Option<&str>,
) -> Fallible<(Package, PathBuf, PathBuf)> {
    let apps_dir = session.effective_root_path().join("apps");
    let pkg_dir = apps_dir.join(name);

    let version = match target_version {
        Some(v) => {
            if v.contains(['/', '\\']) || v == ".." {
                return Err(Error::Custom(format!("invalid version '{}'", v)));
            }
            v.to_owned()
        }
        None => match select_current_version(&pkg_dir) {
            Some(v) => v,
            None => return Err(Error::PackageNotFound(name.to_owned())),
        },
    };
    let version_dir = pkg_dir.join(&version);
    if !version_dir.is_dir() {
        return Err(Error::Custom(format!(
            "version '{}' of '{}' is not installed",
            version, name
        )));
    }

    // Parse the version directory's manifest (upstream `installed_manifest`).
    let manifest = Manifest::parse(version_dir.join("manifest.json"))
        .map_err(|_| Error::Custom(format!("'{}' ({}) isn't installed", name, version)))?;

    // install.json is optional; bucket defaults to the isolated bucket.
    let install_info = InstallInfo::parse(version_dir.join("install.json")).ok();
    let bucket = install_info
        .as_ref()
        .and_then(|i| i.bucket().map(|s| s.to_owned()))
        .unwrap_or_else(|| ISOLATED_PACKAGE_BUCKET.to_owned());

    let pkg = Package::from(name, &bucket, manifest);
    pkg.fill_install_state(InstallState::Installed(InstallStateInstalled {
        version: version.clone(),
        bucket: install_info
            .as_ref()
            .and_then(|i| i.bucket().map(|s| s.to_owned())),
        arch: install_info
            .as_ref()
            .map(|i| i.arch().to_owned())
            .unwrap_or_default(),
        held: install_info.as_ref().map(|i| i.is_held()).unwrap_or(false),
        url: install_info
            .as_ref()
            .and_then(|i| i.url().map(|s| s.to_owned())),
    }));

    Ok((pkg, pkg_dir, version_dir))
}

/// Resolve the current installed version of an app, mirroring upstream
/// `Select-CurrentVersion` (lib/versions.ps1):
///
/// 1. `current\manifest.json`'s `version` — a `nightly` version resolves to
///    the junction target's directory name;
/// 2. otherwise, the version directory whose `install.json` was most recently
///    modified (`Get-InstalledVersion`, excluding `current` and `_*.old*`).
fn select_current_version(pkg_dir: &Path) -> Option<String> {
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
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates.pop().map(|(_, v)| v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{test_session, tmpdir};

    fn mini_manifest(version: &str) -> String {
        format!(
            r#"{{"version": "{}", "homepage": "https://example.com", "license": "MIT"}}"#,
            version
        )
    }

    fn set_modified(path: &Path, time: SystemTime) {
        use std::fs::{File, FileTimes};
        let f = File::options().write(true).open(path).unwrap();
        f.set_times(FileTimes::new().set_modified(time)).unwrap();
    }

    #[test]
    fn select_current_version_reads_current_manifest() {
        let root = tmpdir("reset_current_manifest");
        let pkg_dir = root.join("apps").join("app");
        std::fs::create_dir_all(pkg_dir.join("current")).unwrap();
        std::fs::write(
            pkg_dir.join("current").join("manifest.json"),
            mini_manifest("13.24.5"),
        )
        .unwrap();

        assert_eq!(select_current_version(&pkg_dir).as_deref(), Some("13.24.5"));
    }

    #[test]
    fn select_current_version_falls_back_to_newest_version_dir() {
        let root = tmpdir("reset_fallback");
        let pkg_dir = root.join("apps").join("app");
        for v in ["1.0.0", "2.0.0"] {
            std::fs::create_dir_all(pkg_dir.join(v)).unwrap();
            std::fs::write(pkg_dir.join(v).join("install.json"), "{}").unwrap();
            std::fs::write(pkg_dir.join(v).join("manifest.json"), mini_manifest(v)).unwrap();
        }
        // Pin distinct modification times: 2.0.0 is the most recently touched.
        let old = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000);
        let new = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(2000);
        set_modified(&pkg_dir.join("1.0.0").join("install.json"), old);
        set_modified(&pkg_dir.join("2.0.0").join("install.json"), new);

        assert_eq!(select_current_version(&pkg_dir).as_deref(), Some("2.0.0"));
    }

    #[test]
    fn select_current_version_none_without_versions() {
        let root = tmpdir("reset_none");
        let pkg_dir = root.join("apps").join("app");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        assert_eq!(select_current_version(&pkg_dir), None);
    }

    /// The exact broken-install case from the report: version dir + manifest
    /// exist but `install.json` is missing, so `query_installed` used to skip
    /// the app and `reset` reported "package not found".
    #[test]
    fn resolve_reset_target_tolerates_missing_install_json() {
        let root = tmpdir("reset_missing_install_json");
        let session = test_session(&root);
        let pkg_dir = root.join("apps").join("app");
        std::fs::create_dir_all(pkg_dir.join("13.24.5")).unwrap();
        std::fs::write(
            pkg_dir.join("13.24.5").join("manifest.json"),
            mini_manifest("13.24.5"),
        )
        .unwrap();
        // `current` junction simulated by a plain dir exposing the same manifest
        std::fs::create_dir_all(pkg_dir.join("current")).unwrap();
        std::fs::write(
            pkg_dir.join("current").join("manifest.json"),
            mini_manifest("13.24.5"),
        )
        .unwrap();

        let (pkg, pkg_dir_out, version_dir) = resolve_reset_target(&session, "app", None).unwrap();
        assert_eq!(pkg.name(), "app");
        assert_eq!(pkg.installed_version(), Some("13.24.5"));
        assert_eq!(pkg.installed_bucket(), Some(ISOLATED_PACKAGE_BUCKET));
        assert_eq!(pkg_dir_out, pkg_dir);
        assert_eq!(version_dir, pkg_dir.join("13.24.5"));
    }

    #[test]
    fn resolve_reset_target_not_installed() {
        let root = tmpdir("reset_not_installed");
        let session = test_session(&root);

        let err = resolve_reset_target(&session, "missing", None).unwrap_err();
        assert!(matches!(err, Error::PackageNotFound(_)));
    }

    #[test]
    fn resolve_reset_target_explicit_version() {
        let root = tmpdir("reset_explicit_version");
        let session = test_session(&root);
        let pkg_dir = root.join("apps").join("app");
        for v in ["1.0.0", "2.0.0"] {
            std::fs::create_dir_all(pkg_dir.join(v)).unwrap();
            std::fs::write(pkg_dir.join(v).join("manifest.json"), mini_manifest(v)).unwrap();
        }

        let (pkg, _, version_dir) = resolve_reset_target(&session, "app", Some("1.0.0")).unwrap();
        assert_eq!(pkg.installed_version(), Some("1.0.0"));
        assert_eq!(version_dir, pkg_dir.join("1.0.0"));
    }

    #[test]
    fn resolve_reset_target_rejects_path_traversal_version() {
        let root = tmpdir("reset_traversal");
        let session = test_session(&root);
        std::fs::create_dir_all(root.join("apps").join("app")).unwrap();

        let err = resolve_reset_target(&session, "app", Some("..")).unwrap_err();
        assert!(matches!(err, Error::Custom(_)));
        let err = resolve_reset_target(&session, "app", Some("1.0.0\\..\\..")).unwrap_err();
        assert!(matches!(err, Error::Custom(_)));
    }
}
