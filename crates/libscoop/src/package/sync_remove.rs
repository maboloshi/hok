//! Uninstall / reset pipeline of the sync transaction.
//!
//! This module contains the remove and reset flows — uninstall scripts,
//! shim/shortcut/env/persist cleanup, app-directory removal, and version
//! reset — split out of [`super`] (`sync.rs`). It is a private sub-module
//! of `sync`; the entry points are re-exported by `sync.rs`.

use std::path::PathBuf;

use std::io::IsTerminal;
use tracing::{debug, info};

use crate::constant::ISOLATED_PACKAGE_BUCKET;
use crate::package::{
    InstallState, InstallStateInstalled, Package, install_state,
    manifest::parse_file,
    operations::{self, run_script},
    query, resolve,
};
use crate::{
    Error, Event, QueryOption, Session, env, error::Fallible, internal, persist, psmodule, shim,
    shortcut,
};

use super::sync_install::{
    check_not_running, emit_step, joined_or_none, psmodule_display_name, shim_display_names,
    shortcut_display_names,
};
use super::{SyncOption, Transaction, confirm_transaction, should_gate};

/// Sync operation: remove packages.
pub async fn remove(session: &Session, queries: &[&str], options: &[SyncOption]) -> Fallible<()> {
    let escape_hold = options.contains(&SyncOption::EscapeHold);
    let no_dependent_check = options.contains(&SyncOption::NoDependentCheck);
    let ignore_failure = options.contains(&SyncOption::IgnoreFailure);

    // Query target packages directly instead of scanning all installed.
    // Dependency checking (below) does the full scan when needed. `Broken` is
    // included so a partial/failed install (e.g. missing `current` junction or
    // manifest) is still uninstallable — Scoop's `Confirm-InstallationStatus`
    // only checks that `apps/<name>` exists, so it can be torn down too.
    let mut packages = vec![];
    for &name in queries {
        let matched = query::query_installed(
            session,
            &[name],
            &[QueryOption::Explicit, QueryOption::Broken],
        )?;
        if matched.is_empty() {
            if ignore_failure {
                session.output().error(hok_i18n::t!(
                    "cmd.failed_remove",
                    name = name,
                    error = Error::PackageNotFound(name.to_string())
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

    let transaction = Transaction::default();

    // Nothing to remove (all held, or all failed under IgnoreFailure) —
    // no confirmation needed for an empty transaction.
    if packages.is_empty() {
        return Ok(());
    }

    transaction.set_remove(packages);

    let gate = should_gate(session, options, std::io::stdin().is_terminal());
    if !confirm_transaction(session, &transaction, gate).await? {
        // User declined: exit 0 but no success message (see sync_install).
        return Err(Error::UserAborted);
    }

    if let Some(packages) = transaction.remove_view() {
        let purge = options.contains(&SyncOption::Purge);
        let ignore_failure = options.contains(&SyncOption::IgnoreFailure);
        commit_remove(session, packages, purge, ignore_failure).await?;
    }

    Ok(())
}

/// Execute the removal commit: run scripts, clean up shims/shortcuts/env,
/// remove app directory, and optionally purge persist data.
async fn commit_remove(
    session: &Session,
    packages: &[Package],
    purge: bool,
    ignore_failure: bool,
) -> Fallible<()> {
    let total = packages.len();
    for (i, package) in packages.iter().enumerate() {
        let index = i + 1;
        if let Err(e) = commit_one_remove(session, package, purge, index, total).await {
            if let Some(tx) = session.emitter() {
                let _ = tx.try_send(Event::PackageCommitFailed {
                    name: package.name().to_owned(),
                    index,
                    total,
                    error: e.to_string(),
                });
            }
            if matches!(e, Error::AppRunning(_)) && ignore_failure {
                // App is still running: skip this package and continue with
                // the rest (its process-in-use failure is covered by
                // `ignore_failures`). Without it, the whole batch aborts.
                session.output().warn(hok_i18n::t!(
                    "cmd.running_process_uninstall",
                    name = package.name()
                ));
                continue;
            }
            let msg = hok_i18n::t!("cmd.failed_remove", name = package.name(), error = e);
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

async fn commit_one_remove(
    session: &Session,
    package: &Package,
    purge: bool,
    index: usize,
    total: usize,
) -> Fallible<()> {
    debug!("remove: {} - starting", package.name());

    if let Some(tx) = session.emitter() {
        let _ = tx.try_send(Event::PackageCommitStart {
            name: package.name().to_owned(),
            index,
            total,
            old_version: String::new(),
            new_version: String::new(),
        });
    }

    let app_dir = session.app_dir(package.name());

    // Resolve the current version directory up-front (mirrors Scoop
    // `Select-CurrentVersion` + `versiondir` in scoop-uninstall.ps1); used
    // for persist unlink and directory removal below. Must run before
    // `unlink_current` drops the `current` junction. When no current version
    // resolves (broken install), upstream still proceeds to the old-version
    // cleanup loop below, so this is `Option` — the current-version block is
    // skipped and removal falls through to the old-version loop.
    let version_dir =
        install_state::resolve_current_version(&app_dir, session.config().no_junction())
            .map(|resolved| app_dir.join(resolved.version))
            .or_else(|| package.installed_version().map(|v| app_dir.join(v)));

    // A broken/partial install (e.g. install failed before `current` was
    // linked, or `manifest.json`/`install.json` is missing) carries no
    // reliable manifest or install state. Mirror Scoop: `Confirm-InstallationStatus`
    // only checks that `apps/<name>` exists (so the app is still uninstallable),
    // but scoop-uninstall.ps1 skips the whole `if ($version)` block — the
    // pre_uninstall/uninstaller hooks and the shim/shortcut/env/psmodule cleanup —
    // and only tears down the directories. The shim/shortcut/env/psmodule/persist
    // helpers also `assert!(package.is_installed())`, which a broken package (no
    // install state) would trip, so they are skipped here too.
    if !package.is_install_failed() {
        // Scripts run against the version dir (`$dir`), mirroring
        // scoop-uninstall.ps1 (`$dir = versiondir $app $version $global`); with
        // junctions the `current` junction resolves to it, and under
        // `NO_JUNCTION` the version dir is used directly. Fall back to `current`
        // only when no version could be resolved (broken install).
        let fallback_dir = app_dir.join("current");
        let script_dir = version_dir.as_deref().unwrap_or(&fallback_dir);

        if package.manifest().pre_uninstall().is_some() {
            emit_step(
                session,
                package.name(),
                crate::PackageStepKind::RunHookScript,
                Some("pre_uninstall".to_owned()),
            );
        }
        run_script(
            session,
            package,
            script_dir,
            None,
            "pre_uninstall",
            "uninstall",
            package.manifest().pre_uninstall(),
        )
        .await?;

        // Check if the app is currently running after running pre_uninstall
        // (mirrors scoop-uninstall.ps1: pre_uninstall hook runs first, then
        // test_running_process). An AppRunning error is handled by the caller
        // (commit_remove) which skips this package and continues the batch.
        check_not_running(session, package.name(), &hok_i18n::t!("action.uninstall"))?;

        if let Some(uninstaller) = package.manifest().uninstaller() {
            match uninstaller.file() {
                Some(file) => {
                    let mut object = file.to_owned();
                    if let Some(args) = uninstaller.args()
                        && !args.is_empty()
                    {
                        object.push(' ');
                        object.push_str(&args.join(" "));
                    }
                    emit_step(
                        session,
                        package.name(),
                        crate::PackageStepKind::RunInstallerFile,
                        Some(object),
                    );
                }
                None => {
                    emit_step(
                        session,
                        package.name(),
                        crate::PackageStepKind::RunHookScript,
                        Some("uninstaller".to_owned()),
                    );
                }
            }
            operations::run_hook(
                session,
                package,
                script_dir,
                "uninstaller",
                "uninstall",
                uninstaller.file(),
                uninstaller.args(),
                uninstaller.keep(),
                uninstaller.script(),
            )
            .await?;
        }

        debug!(
            "remove: {} - cleanup (shims/shortcuts/current/env/persist)",
            package.name()
        );
        for shim_name in shim_display_names(package) {
            emit_step(
                session,
                package.name(),
                crate::PackageStepKind::RemoveShim,
                Some(shim_name),
            );
        }
        shim::remove(session, package)?;
        for shortcut_name in shortcut_display_names(package) {
            emit_step(
                session,
                package.name(),
                crate::PackageStepKind::RemoveShortcut,
                Some(shortcut_name),
            );
        }
        shortcut::remove(session, package)?;

        // Scoop order: `unlink_current` runs right after shims/shortcuts and
        // before psmodule/env (scoop-uninstall.ps1). `psmodule::remove` and
        // `env::remove` resolve their own paths, so they don't need the
        // `$refdir` return value that upstream passes along.
        emit_step(
            session,
            package.name(),
            crate::PackageStepKind::UnlinkCurrent,
            None,
        );
        operations::unlink_current(&app_dir, session.config().no_junction())?;
        if let Some(module) = psmodule_display_name(package) {
            emit_step(
                session,
                package.name(),
                crate::PackageStepKind::RemovePsModule,
                Some(module),
            );
        }
        psmodule::remove(session, package)?;

        if let Some(paths) = package.manifest().env_add_path()
            && !paths.is_empty()
        {
            emit_step(
                session,
                package.name(),
                crate::PackageStepKind::RemoveEnvPath,
                joined_or_none(paths.iter().map(|s| s.to_string()).collect()),
            );
        }
        if let Some(vars) = package.manifest().env_set() {
            let mut keys = vars.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                emit_step(
                    session,
                    package.name(),
                    crate::PackageStepKind::RemoveEnvVar,
                    Some(key),
                );
            }
        }
        env::remove(session, package)?;
    }

    // Remove the current version directory: unlink persist links first,
    // then delete (Scoop order: `unlink_persist_data $manifest $dir` +
    // `Remove-Item $dir`). If the directory is gone already (e.g. the
    // uninstaller removed it), keep going like upstream does. Skipped when
    // no current version could be resolved (broken install).
    if let Some(version_dir) = &version_dir {
        // persist::unlink asserts the package is installed; a broken package
        // has no persist metadata anyway, so skip it (Scoop's
        // unlink_persist_data is a no-op for a placeholder manifest too).
        if !package.is_install_failed() {
            if let Some(persist) = package.manifest().persist() {
                for item in persist {
                    let source = item
                        .first()
                        .copied()
                        .unwrap_or_default()
                        .trim_end_matches(['/', '\\']);
                    emit_step(
                        session,
                        package.name(),
                        crate::PackageStepKind::PersistUnlink,
                        Some(source.to_owned()),
                    );
                }
            }
            persist::unlink(session, package, version_dir)?;
        }
        emit_step(
            session,
            package.name(),
            crate::PackageStepKind::RemoveDir,
            Some(version_dir.to_string_lossy().into_owned()),
        );
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
        if package.manifest().post_uninstall().is_some() {
            emit_step(
                session,
                package.name(),
                crate::PackageStepKind::RunHookScript,
                Some("post_uninstall".to_owned()),
            );
        }
        run_script(
            session,
            package,
            &app_dir,
            None,
            "post_uninstall",
            "uninstall",
            package.manifest().post_uninstall(),
        )
        .await?;
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
            // Only unlink persist links for a healthy install; persist::unlink
            // asserts the package is installed (a broken one has no persist
            // metadata to unlink anyway).
            if !package.is_install_failed() {
                persist::unlink(session, package, &old_dir)?;
            }
            // A broken install can leave stray files directly in the app dir
            // (not in a version subdir); remove them like Scoop's `Remove-Item`.
            let remove_one = || -> std::io::Result<()> {
                if old_dir.is_dir() {
                    internal::fs::remove_dir(&old_dir)
                } else {
                    std::fs::remove_file(&old_dir)
                }
            };
            match remove_one() {
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
    if let Ok(mut entries) = std::fs::read_dir(&app_dir)
        && entries.next().is_none()
    {
        internal::fs::remove_dir(&app_dir)?;
    }

    if purge {
        persist::purge(session, package.name())?;
    }

    if let Some(tx) = session.emitter() {
        let _ = tx.try_send(Event::PackageCommitDone {
            name: package.name().to_owned(),
            index,
            total,
        });
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
///
/// `index` / `total` position the package's `(k/N)` commit anchor. A
/// single-package `hok reset <app>` passes `(1, 1)`; `reset --all` numbers
/// each attempted package across its whole batch.
pub async fn reset(
    session: &Session,
    name: &str,
    target_version: Option<&str>,
    index: usize,
    total: usize,
) -> Fallible<()> {
    let (pkg, pkg_dir, version_dir) = match resolve_reset_target(session, name, target_version) {
        Ok(r) => r,
        // A broken/partial install (app dir exists but no usable current
        // version) can't be reset — mirror Scoop (`scoop-reset.ps1` reports
        // it as "isn't installed" and moves on). Warn so the user knows how
        // to recover instead of hard-erroring.
        Err(Error::PackageNotFound(_)) if session.app_dir(name).exists() => {
            session
                .output()
                .warn(hok_i18n::t!("cmd.not_installed_correctly", name = name));
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    let version = pkg.installed_version().unwrap_or(pkg.version());

    // Check if the app is currently running before resetting (mirrors
    // scoop-reset.ps1: test_running_process before re-linking). The check is
    // per-package only: when `ignore_running_processes` is enabled, print a
    // warning (with the process list, already done by check_not_running) and
    // proceed with the reset; without it, AppRunning aborts.
    match check_not_running(session, name, &hok_i18n::t!("action.reset")) {
        Ok(_) => {}
        Err(e) => return Err(e),
    }

    info!("resetting {} ({})", name, version);

    // Anchor the reset in the commit sequence like install/remove do: a bare
    // `(k/N) name` header (no version arrow — a reset re-applies the current
    // version, it doesn't transition versions) closed by `✓ name`. Emitted
    // only on the success path after the running-process/broken checks above.
    if let Some(tx) = session.emitter() {
        let _ = tx.try_send(Event::PackageCommitStart {
            name: name.to_owned(),
            index,
            total,
            old_version: String::new(),
            new_version: String::new(),
        });
    }

    // Re-create the `current` symlink
    operations::link_current(&pkg_dir, &version_dir, session.config().no_junction())?;
    emit_step(session, name, crate::PackageStepKind::LinkCurrent, None);

    // Re-link persistent data
    if let Some(persist) = pkg.manifest().persist() {
        for item in persist {
            let source = item
                .first()
                .copied()
                .unwrap_or_default()
                .trim_end_matches(['/', '\\']);
            emit_step(
                session,
                name,
                crate::PackageStepKind::PersistLink,
                Some(source.to_owned()),
            );
        }
    }
    persist::link(session, &pkg)?;
    // persist_permission (upstream reset.ps1:91): global apps reset their
    // persist root grant as well (idempotent).
    persist::persist_permission(session, &pkg)?;

    // Re-create shims + shortcuts (reported as a single "refreshing" step per
    // item — reset removes then re-creates each in place).
    for shim_name in shim_display_names(&pkg) {
        emit_step(
            session,
            name,
            crate::PackageStepKind::RefreshShim,
            Some(shim_name),
        );
    }
    shim::remove(session, &pkg)?;
    shim::add(session, &pkg)?;
    for shortcut_name in shortcut_display_names(&pkg) {
        emit_step(
            session,
            name,
            crate::PackageStepKind::RefreshShortcut,
            Some(shortcut_name),
        );
    }
    shortcut::remove(session, &pkg)?;
    shortcut::add(session, &pkg)?;

    // Re-apply env (mirrors scoop-reset.ps1: env_rm_path/env_rm then
    // env_add_path/env_set — unset all potential old env before re-adding)
    env::remove(session, &pkg)?;
    env::add(session, &pkg)?;

    // Run post_install to reapply localization (fixes Scoop bug)
    if pkg.manifest().post_install().is_some() {
        emit_step(
            session,
            name,
            crate::PackageStepKind::RunHookScript,
            Some("post_install".to_owned()),
        );
    }
    run_script(
        session,
        &pkg,
        &version_dir,
        None,
        "post_install",
        "install",
        pkg.manifest().post_install(),
    )
    .await?;

    if let Some(tx) = session.emitter() {
        let _ = tx.try_send(Event::PackageCommitDone {
            name: name.to_owned(),
            index,
            total,
        });
    }

    Ok(())
}

/// Whether `reset` can actually act on `name` — i.e. a usable current (or
/// versioned) install resolves, so `reset` will anchor and do work rather than
/// warn "isn't installed correctly" and skip.
///
/// `reset --all` uses this to keep its `(k/N)` counter contiguous: genuinely
/// not-installed-correctly apps are reported and excluded *before* counting,
/// while repairable partial installs (a broken `current` junction but a valid
/// versioned install, or a current manifest without `install.json`) still
/// resolve and stay in the batch.
pub fn reset_eligible(session: &Session, name: &str) -> bool {
    resolve_reset_target(session, name, None).is_ok()
}

/// Resolve the package and version directory a `reset` should operate on,
/// tolerating broken installs the same way upstream Scoop does.
///
/// - `apps/<name>` missing or version unresolvable → [`Error::PackageNotFound`]
/// - explicit `target_version` → that version directory (must exist; rejected
///   if it contains path separators or `..`)
/// - otherwise → [`resolve_current_version`]
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
        None => {
            match install_state::resolve_current_version(&pkg_dir, session.config().no_junction()) {
                Some(resolved) => resolved.version,
                None => return Err(Error::PackageNotFound(name.to_owned())),
            }
        }
    };
    let version_dir = pkg_dir.join(&version);
    if !version_dir.is_dir() {
        return Err(Error::Custom(format!(
            "version '{}' of '{}' is not installed",
            version, name
        )));
    }

    // Parse the version directory's manifest (upstream `installed_manifest`).
    let manifest = parse_file(version_dir.join("manifest.json"))
        .map_err(|_| Error::Custom(format!("'{}' ({}) isn't installed", name, version)))?;

    // install.json is optional; bucket defaults to the isolated bucket.
    let install_info = install_state::parse_install_info(&version_dir.join("install.json")).ok();
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
    use crate::package::install_state::resolve_current_version;
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
    fn resolve_current_version_reads_current_manifest() {
        let root = tmpdir("reset_current_manifest");
        let pkg_dir = root.join("apps").join("app");
        std::fs::create_dir_all(pkg_dir.join("current")).unwrap();
        std::fs::write(
            pkg_dir.join("current").join("manifest.json"),
            mini_manifest("13.24.5"),
        )
        .unwrap();

        assert_eq!(
            resolve_current_version(&pkg_dir, false).map(|r| r.version),
            Some("13.24.5".to_owned())
        );
    }

    #[test]
    fn resolve_current_version_falls_back_to_newest_version_dir() {
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

        assert_eq!(
            resolve_current_version(&pkg_dir, false).map(|r| r.version),
            Some("2.0.0".to_owned())
        );
    }

    /// Under NO_JUNCTION a leftover `current` dir is stale, and no version dir
    /// carries `install.json` — so nothing resolves (upstream reads the
    /// manifest only when junctions are enabled).
    #[test]
    fn resolve_current_version_none_without_versions() {
        let root = tmpdir("reset_none");
        let pkg_dir = root.join("apps").join("app");
        std::fs::create_dir_all(pkg_dir.join("current")).unwrap();
        std::fs::write(
            pkg_dir.join("current").join("manifest.json"),
            mini_manifest("9.9.9"),
        )
        .unwrap();

        assert_eq!(resolve_current_version(&pkg_dir, true), None);
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

    /// The broken-install uninstall case: `apps/<name>` exists but has no
    /// usable `current` junction (install failed), so `query_installed` would
    /// otherwise skip it and `remove` reported "package not found". With
    /// `QueryOption::Broken` the placeholder package is found and the app dir
    /// is torn down.
    #[tokio::test]
    async fn remove_uninstalls_broken_install_with_version_dir() {
        let root = tmpdir("remove_broken_version");
        let session = test_session(&root);
        let pkg_dir = root.join("apps").join("doubao");
        // Version dir with metadata but no `current` junction: broken.
        std::fs::create_dir_all(pkg_dir.join("1.0.0")).unwrap();
        std::fs::write(
            pkg_dir.join("1.0.0").join("manifest.json"),
            mini_manifest("1.0.0"),
        )
        .unwrap();
        std::fs::write(
            pkg_dir.join("1.0.0").join("install.json"),
            r#"{"architecture": "64bit", "bucket": "main"}"#,
        )
        .unwrap();

        remove(
            &session,
            &["doubao"],
            &[SyncOption::Remove, SyncOption::AssumeYes],
        )
        .await
        .unwrap();

        assert!(
            !pkg_dir.exists(),
            "broken install app dir should be removed"
        );
    }

    /// The degenerate broken-install case: `apps/<name>` exists but contains
    /// neither `manifest.json` nor `install.json` (nor a version dir) — only
    /// stray leftover files. Scoop still uninstalls it
    /// (`Confirm-InstallationStatus` only checks the app dir exists), so hok
    /// must too.
    #[tokio::test]
    async fn remove_uninstalls_broken_install_with_stray_files_only() {
        let root = tmpdir("remove_broken_stray");
        let session = test_session(&root);
        let pkg_dir = root.join("apps").join("doubao");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("partial-download.bin"), b"junk").unwrap();

        remove(
            &session,
            &["doubao"],
            &[SyncOption::Remove, SyncOption::AssumeYes],
        )
        .await
        .unwrap();

        assert!(
            !pkg_dir.exists(),
            "app dir with only stray files should be removed"
        );
    }

    /// A broken/partial install (app dir exists but no usable current version)
    /// cannot be reset — like Scoop it is reported and skipped, not a hard
    /// error, and the broken dir is left untouched.
    #[tokio::test]
    async fn reset_broken_install_warns_and_returns_ok() {
        let root = tmpdir("reset_broken_warn");
        let session = test_session(&root);
        let pkg_dir = root.join("apps").join("doubao");
        // Version dir with a manifest but no install.json and no `current`:
        // a broken install whose version cannot be resolved.
        std::fs::create_dir_all(pkg_dir.join("1.0.0")).unwrap();
        std::fs::write(
            pkg_dir.join("1.0.0").join("manifest.json"),
            mini_manifest("1.0.0"),
        )
        .unwrap();

        // Must not hard-error: a broken install is reported and skipped.
        reset(&session, "doubao", None, 1, 1).await.unwrap();

        // reset must not remove the app dir.
        assert!(pkg_dir.exists(), "reset must not remove the broken app dir");
    }

    /// `reset_eligible` reports a truly broken install (version dir + manifest
    /// but no usable current version) as ineligible, but a repairable partial
    /// (usable `current` manifest) as eligible.
    #[test]
    fn reset_eligible_distinguishes_broken_from_repairable() {
        // Not installed correctly: version dir + manifest, no install.json,
        // no `current` junction — reset would warn and skip.
        let root = tmpdir("reset_eligible_broken");
        let session = test_session(&root);
        let broken = root.join("apps").join("broken");
        std::fs::create_dir_all(broken.join("1.0.0")).unwrap();
        std::fs::write(
            broken.join("1.0.0").join("manifest.json"),
            mini_manifest("1.0.0"),
        )
        .unwrap();
        assert!(
            !reset_eligible(&session, "broken"),
            "truly broken must be ineligible"
        );

        // Repairable partial: `current` junction exposes a manifest, so reset
        // can resolve a version and should remain in the batch.
        let repairable = root.join("apps").join("repairable");
        std::fs::create_dir_all(repairable.join("2.0.0")).unwrap();
        std::fs::write(
            repairable.join("2.0.0").join("manifest.json"),
            mini_manifest("2.0.0"),
        )
        .unwrap();
        std::fs::create_dir_all(repairable.join("current")).unwrap();
        std::fs::write(
            repairable.join("current").join("manifest.json"),
            mini_manifest("2.0.0"),
        )
        .unwrap();
        assert!(
            reset_eligible(&session, "repairable"),
            "repairable partial must be eligible"
        );
    }
}
