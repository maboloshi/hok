//! Uninstall / reset pipeline of the sync transaction.
//!
//! This module contains the remove and reset flows — uninstall scripts,
//! shim/shortcut/env/persist cleanup, app-directory removal, and version
//! reset — split out of [`super`] (`sync.rs`). It is a private sub-module
//! of `sync`; the entry points are re-exported by `sync.rs`.

use std::path::PathBuf;

use tracing::{debug, info};

use crate::constant::ISOLATED_PACKAGE_BUCKET;
use crate::package::{
    manifest::{InstallInfo, Manifest},
    operations::{self, run_script},
    query, resolve, InstallState, InstallStateInstalled, Package,
};
use crate::{
    env, error::Fallible, internal, persist, psmodule, shim, shortcut, Error, Event, QueryOption,
    Session,
};

use super::sync_install::check_not_running;
use super::{confirm_transaction, SyncOption, Transaction};

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
                session.output().error(format!(
                    "failed to remove '{}': {}",
                    name,
                    Error::PackageNotFound(name.to_string())
                ));
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
            if matches!(e, Error::AppRunning(_)) && ignore_failure {
                // App is still running: skip this package and continue with
                // the rest (its process-in-use failure is covered by
                // `ignore_failures`). Without it, the whole batch aborts.
                session.output().warn(format!(
                    "Running process detected, skip uninstalling '{}'.",
                    package.name()
                ));
                continue;
            }
            let msg = format!("failed to remove '{}': {}", package.name(), e);
            if ignore_failure {
                session.output().error(msg);
                continue;
            }
            // Return the original error (keeps the AppRunning variant and
            // its i18n key) instead of wrapping it in Error::Custom.
            return Err(e);
        }
    }
    Ok(())
}

fn commit_one_remove(session: &Session, package: &Package, purge: bool) -> Fallible<()> {
    debug!("remove: {} - starting", package.name());

    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackageCommitStart(package.name().to_owned()));
    }

    let app_dir = session.app_dir(package.name());

    // Resolve the current version directory up-front (mirrors Scoop
    // `Select-CurrentVersion` + `versiondir` in scoop-uninstall.ps1); used
    // for persist unlink and directory removal below. Must run before
    // `unlink_current` drops the `current` junction. When no current version
    // resolves (broken install), upstream still proceeds to the old-version
    // cleanup loop below, so this is `Option` — the current-version block is
    // skipped and removal falls through to the old-version loop.
    let version_dir = query::select_current_version(&app_dir)
        .or_else(|| package.installed_version().map(str::to_owned))
        .map(|v| app_dir.join(&v));

    // Scripts run against the version dir (`$dir`), mirroring
    // scoop-uninstall.ps1 (`$dir = versiondir $app $version $global`); with
    // junctions the `current` junction resolves to it, and under `NO_JUNCTION`
    // the version dir is used directly. Fall back to `current` only when no
    // version could be resolved (broken install).
    let fallback_dir = app_dir.join("current");
    let script_dir = version_dir.as_deref().unwrap_or(&fallback_dir);

    run_script(
        session,
        package,
        script_dir,
        None,
        "pre_uninstall",
        "uninstall",
        package.manifest().pre_uninstall(),
    )?;

    // Check if the app is currently running after running pre_uninstall
    // (mirrors scoop-uninstall.ps1: pre_uninstall hook runs first, then
    // test_running_process). An AppRunning error is handled by the caller
    // (commit_remove) which skips this package and continues the batch.
    check_not_running(session, package.name(), "uninstalling")?;

    if let Some(uninstaller) = package.manifest().uninstaller() {
        if let Some(script) = uninstaller.script() {
            run_script(
                session,
                package,
                script_dir,
                None,
                "uninstaller",
                "uninstall",
                Some(script),
            )?;
        } else if let Some(file) = uninstaller.file() {
            let raw_args: Vec<&str> = uninstaller.args().unwrap_or_default();
            operations::run_installer_file(
                session,
                package,
                script_dir,
                "uninstaller",
                "uninstall",
                file,
                &raw_args,
            )?;
        }
    }

    debug!(
        "remove: {} - cleanup (shims/shortcuts/current/env/persist)",
        package.name()
    );
    shim::remove(session, package)?;
    shortcut::remove(session, package)?;

    // Scoop order: `unlink_current` runs right after shims/shortcuts and
    // before psmodule/env (scoop-uninstall.ps1). `psmodule::remove` and
    // `env::remove` resolve their own paths, so they don't need the
    // `$refdir` return value that upstream passes along.
    operations::unlink_current(&app_dir, session.config().no_junction())?;
    psmodule::remove(session, package)?;
    env::remove(session, package)?;

    // Remove the current version directory: unlink persist links first,
    // then delete (Scoop order: `unlink_persist_data $manifest $dir` +
    // `Remove-Item $dir`). If the directory is gone already (e.g. the
    // uninstaller removed it), keep going like upstream does. Skipped when
    // no current version could be resolved (broken install).
    if let Some(version_dir) = &version_dir {
        persist::unlink(session, package, version_dir)?;
        match internal::fs::remove_dir(version_dir) {
            Ok(()) => {}
            Err(_e) if !version_dir.exists() => {}
            Err(e) => {
                return Err(Error::Custom(format!(
                    "couldn't remove '{}'; it may be in use: {}",
                    version_dir.display(),
                    e
                )));
            }
        }

        // post_uninstall runs after the version dir has been removed
        // (Scoop order in scoop-uninstall.ps1).
        run_script(
            session,
            package,
            &app_dir,
            None,
            "post_uninstall",
            "uninstall",
            package.manifest().post_uninstall(),
        )?;
    }

    // Remove older versions one by one, unlinking persist data in each
    // (Scoop order: loop over `Get-ChildItem $appDir -Exclude 'current'`).
    if let Ok(entries) = std::fs::read_dir(&app_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "current" {
                continue;
            }
            let old_dir = app_dir.join(&name);
            debug!(
                "remove: {} - removing older version {}",
                package.name(),
                name
            );
            persist::unlink(session, package, &old_dir)?;
            match internal::fs::remove_dir(&old_dir) {
                Ok(()) => {}
                Err(_e) if !old_dir.exists() => {}
                Err(e) => {
                    return Err(Error::Custom(format!(
                        "couldn't remove '{}'; it may be in use: {}",
                        old_dir.display(),
                        e
                    )));
                }
            }
        }
    }

    // Remove a leftover `current` link, if any (Scoop re-checks after the
    // version loop; e.g. when NO_JUNCTION is set).
    let current_lnk = app_dir.join("current");
    if current_lnk.exists() {
        let _ = internal::fs::remove_symlink(&current_lnk);
    }

    // Remove the app dir only when it is empty (Scoop: `if (!(Get-ChildItem
    // $appDir)) { Remove-Item $appdir }`).
    if let Ok(mut entries) = std::fs::read_dir(&app_dir) {
        if entries.next().is_none() {
            internal::fs::remove_dir(&app_dir)?;
        }
    }

    if purge {
        persist::purge(session, package.name())?;
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

    // Check if the app is currently running before resetting (mirrors
    // scoop-reset.ps1: test_running_process before re-linking). The check is
    // per-package only: when `ignore_running_processes` is enabled, print a
    // warning (with the process list, already done by check_not_running) and
    // proceed with the reset; without it, AppRunning aborts.
    match check_not_running(session, name, "resetting") {
        Ok(_) => {}
        Err(e) => return Err(e),
    }

    info!("resetting {} ({})", name, version);

    // Re-create the `current` symlink
    operations::link_current(&pkg_dir, &version_dir, session.config().no_junction())?;

    // Re-link persistent data
    persist::link(session, &pkg)?;

    // Re-create shims + shortcuts
    shim::remove(session, &pkg)?;
    shim::add(session, &pkg)?;
    shortcut::remove(session, &pkg)?;
    shortcut::add(session, &pkg)?;

    // Re-apply env (mirrors scoop-reset.ps1: env_rm_path/env_rm then
    // env_add_path/env_set — unset all potential old env before re-adding)
    env::remove(session, &pkg)?;
    env::add(session, &pkg)?;

    // Run post_install to reapply localization (fixes Scoop bug)
    run_script(
        session,
        &pkg,
        &version_dir,
        None,
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
    let apps_dir = session.apps_dir();
    let pkg_dir = apps_dir.join(name);

    let version = match target_version {
        Some(v) => {
            if v.contains(['/', '\\']) || v == ".." {
                return Err(Error::Custom(format!("invalid version '{}'", v)));
            }
            v.to_owned()
        }
        None => match query::select_current_version(&pkg_dir) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::query::select_current_version;
    use crate::test_utils::{test_session, tmpdir};
    use std::path::Path;
    use std::time::SystemTime;

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
