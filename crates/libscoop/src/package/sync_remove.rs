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
    InstallState, InstallStateInstalled, Package, identity, install_state,
    manifest::parse_file,
    operations::{self, run_script},
    query, resolve,
};
use crate::{
    Error, Event, QueryOption, Session, env, error::Fallible, internal, persist, psmodule, shim,
    shortcut,
};

use super::sync_install::{
    RunningAction, RunningCheck, check_not_running, emit_step, joined_or_none,
    psmodule_display_name, shim_display_names, shortcut_display_names,
};
use super::{SyncOption, Transaction, confirm_transaction, should_gate};

/// Sync operation: remove packages.
pub async fn remove(session: &Session, queries: &[&str], options: &[SyncOption]) -> Fallible<()> {
    let no_dependent_check = options.contains(&SyncOption::NoDependentCheck);

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
            // Scoop's Confirm-InstallationStatus reports "'x' isn't installed."
            // and drops the app from the batch (`lib/core.ps1:1084-1121`); the
            // command still exits 0 (`scoop-uninstall.ps1:44-45`).
            session
                .output()
                .error(hok_i18n::t!("cmd.not_installed", name = name));
            continue;
        }
        let pkg = matched.into_iter().next().unwrap();
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
        resolve::resolve_cascade(session, &mut packages)?;
    }

    let transaction = Transaction::default();

    // Nothing to remove (every named app was missing, or all failed under
    // IgnoreFailures) — no confirmation needed for an empty transaction.
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
        let ignore_failures = options.contains(&SyncOption::IgnoreFailures);
        commit_remove(session, packages, purge, ignore_failures).await?;
    }

    Ok(())
}

/// Execute the removal commit: run scripts, clean up shims/shortcuts/env,
/// remove app directory, and optionally purge persist data.
async fn commit_remove(
    session: &Session,
    packages: &[Package],
    purge: bool,
    ignore_failures: bool,
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
            let msg = hok_i18n::t!("cmd.failed_remove", name = package.name(), error = e);
            if ignore_failures {
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
    // linked, or the manifest/install record is missing) carries no
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
        // (mirrors scoop-uninstall.ps1:62-67: the pre_uninstall hook runs
        // first, then test_running_process). Scoop skips the running app and
        // continues with the next one; the check printed the reason — unless
        // `ignore_running_processes` says to carry on with this one.
        if check_not_running(session, package.name(), RunningAction::Uninstall)?
            == RunningCheck::Running
        {
            return Ok(());
        }

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

/// One app named on the `reset` command line, resolved to the install scope and
/// the version the reset will act on.
struct ResetRequest {
    /// App name, with `bucket/` and `@version` already split off (`parse_app`).
    name: String,
    /// The `@version` part, if given. This is **not** a separate positional
    /// argument: every positional argument is an app (`scoop-reset.ps1:16,31`).
    version: Option<String>,
    /// Scope the reset runs in. Official has no `-g/--global`: the flag is set
    /// when the app is installed globally (`:33-36`), and `-a` fixes it per app
    /// by enumerating the two scopes itself (`:23-25`).
    global: bool,
}

/// Why a named `reset` app will not be attempted.
enum ResetUnavailable {
    /// The app — or the requested `@version` of it — is not installed
    /// (`scoop-reset.ps1:43-46` / `:55-58`). `Some` carries the requested
    /// version so the report can name it.
    NotInstalled(Option<String>),
    /// `apps/<name>` exists but no usable current version resolves.
    Broken,
}

/// Reset one or more installed apps, mirroring official `scoop reset`
/// (`libexec/scoop-reset.ps1`).
///
/// `queries` are the positional app arguments — `app`, `bucket/app`,
/// `app@version` and any mix of them, all parsed by `parse_app` exactly as
/// upstream does (`:31`). `all` is the `-a/--all` (or `*`) switch, which
/// replaces the app list with every installed app, local first then global
/// (`:22-26`).
///
/// Each app is handled independently: a failing app is reported and the
/// remaining ones still reset (upstream's `ForEach-Object` + `return`), and the
/// command itself still succeeds. The `(k/N)` commit counter spans only the
/// apps that will actually be attempted, so a reported-and-skipped app leaves
/// no gap behind.
pub async fn reset(session: &Session, queries: &[&str], all: bool) -> Fallible<()> {
    let mut attempts: Vec<ResetRequest> = Vec::new();

    if all {
        // Reset every installed app — local first, then global (mirrors
        // scoop-reset.ps1 `installed_apps $false` + `installed_apps $true`).
        for global in [false, true] {
            session.set_global(global);
            // `Broken` is included so a partial install shows up and is
            // either repaired (if a version resolves) or reported as broken,
            // mirroring Scoop's `installed_apps` (which lists every app dir).
            let pkgs = query::query_installed(session, &["*"], &[QueryOption::Broken])?;
            for pkg in &pkgs {
                if pkg.name() == "scoop" {
                    // Official scoop-reset.ps1 skips 'scoop' itself.
                    continue;
                }
                // Official warns and skips global apps without admin rights.
                if global && !session.is_admin() {
                    session
                        .output()
                        .warn(hok_i18n::t!("cmd.reset_admin_skip", name = pkg.name()));
                    continue;
                }
                let request = ResetRequest {
                    name: pkg.name().to_owned(),
                    version: None,
                    global,
                };
                // A genuinely not-installed-correctly app (no usable current
                // version) can't be reset: report it and leave it out of the
                // (k/N) count so the numbering stays contiguous. Repairable
                // partial installs still resolve here and remain in the batch.
                if reset_unavailable(session, &request).is_some() {
                    session
                        .output()
                        .warn(hok_i18n::t!("cmd.reset_broken_skip", name = pkg.name()));
                    continue;
                }
                attempts.push(request);
            }
        }
    } else {
        for query in queries {
            // Official `parse_app`s every positional argument; `scoop` itself
            // is skipped silently (`:38-41`).
            let Some((name, version)) = parse_reset_query(query) else {
                continue;
            };
            // Official has no `-g`: an app that is installed globally resets
            // globally (`:33-36`), and only then must the caller be an admin
            // (`:60-63`).
            let global = reset_installed_globally(session, &name);
            session.set_global(global);
            let request = ResetRequest {
                name,
                version,
                global,
            };
            match reset_unavailable(session, &request) {
                // Upstream reports the app and moves on; the run still exits 0
                // (`exit 0` at `:94`).
                Some(ResetUnavailable::NotInstalled(version)) => {
                    let message = match version.as_deref() {
                        Some(version) => hok_i18n::t!(
                            "cmd.not_installed_version",
                            name = request.name.as_str(),
                            version = version
                        ),
                        None => hok_i18n::t!("cmd.not_installed", name = request.name.as_str()),
                    };
                    session.output().error(message);
                    continue;
                }
                Some(ResetUnavailable::Broken) => {
                    session.output().warn(hok_i18n::t!(
                        "cmd.not_installed_correctly",
                        name = request.name.as_str()
                    ));
                    continue;
                }
                None => {}
            }
            if request.global && !session.is_admin() {
                session.output().warn(hok_i18n::t!(
                    "cmd.reset_admin_skip",
                    name = request.name.as_str()
                ));
                continue;
            }
            attempts.push(request);
        }
    }

    let total = attempts.len();
    for (k, request) in attempts.iter().enumerate() {
        // Restore the scope this app was resolved under before resetting it
        // (paths resolve against the session's scope).
        session.set_global(request.global);
        // Official scoop-reset.ps1 iterates apps independently
        // (ForEach-Object with `return`): a failing app is reported
        // and skipped, the remaining apps keep resetting.
        if let Err(e) = reset_one(session, request, k + 1, total).await {
            session.output().error(hok_i18n::t!(
                "cmd.reset_failed",
                name = request.name.as_str(),
                e = e
            ));
        }
    }
    // Leave the session back in the default (user-level) state.
    session.set_global(false);

    Ok(())
}

/// Split one positional `reset` argument the way upstream `parse_app`
/// (`lib/core.ps1`) does, dropping the bucket part: `scoop-reset.ps1` never
/// forwards `$bucket` to `installed` / `Select-CurrentVersion`.
///
/// Returns `None` for `scoop` itself, which upstream skips silently (`:38-41`).
fn parse_reset_query(query: &str) -> Option<(String, Option<String>)> {
    // [`identity::parse_app`] mirrors upstream's regex; like upstream it falls
    // back to the raw string when nothing matches (`return $app, $null, $null`).
    let (name, version) = match identity::parse_app(query) {
        Some(parsed) => (parsed.app, parsed.version),
        None => (query.to_owned(), None),
    };
    (name != "scoop").then_some((name, version))
}

/// Whether `name` has a resolvable install under the global root — upstream's
/// `installed $app $true` (`lib/core.ps1`), which `scoop reset` uses to set the
/// global flag when no scope was named explicitly (`:33-36`).
fn reset_installed_globally(session: &Session, name: &str) -> bool {
    let scope = session.is_global();
    session.set_global(true);
    let installed = install_state::resolve_current_version(
        &session.app_dir(name),
        session.config().no_junction(),
    )
    .is_some();
    session.set_global(scope);
    installed
}

/// Whether `request` resolves to a version `reset` can act on, and why not when
/// it does not — the guards upstream checks in order (`scoop-reset.ps1:43-46`,
/// then `:55-58`).
fn reset_unavailable(session: &Session, request: &ResetRequest) -> Option<ResetUnavailable> {
    if !session.app_dir(&request.name).exists() {
        // `installed $app` is false: upstream reports it and moves on.
        return Some(ResetUnavailable::NotInstalled(None));
    }
    match resolve_reset_target(session, &request.name, request.version.as_deref()) {
        Ok(_) => None,
        // An explicit `@version` with no version directory or manifest —
        // upstream's `installed_manifest` is $null.
        Err(_) if request.version.is_some() => {
            Some(ResetUnavailable::NotInstalled(request.version.clone()))
        }
        // The app dir is there, but no usable current version resolves.
        Err(_) => Some(ResetUnavailable::Broken),
    }
}

/// Reset one package: re-link current version, re-create shims/shortcuts,
/// and run post_install. Unlike Scoop's original reset, this runs
/// post_install to reapply localization settings.
///
/// Package resolution mirrors upstream `scoop reset` (libexec/scoop-reset.ps1):
/// the app is considered installed when `apps/<name>` exists and the current
/// version can be resolved — reading `current\scoop-manifest.json`'s (else
/// `current\manifest.json`'s) `version` first
/// (`Select-CurrentVersion`), then falling back to version directories with an
/// install record (`scoop-install.json`, else `install.json`), newest by
/// modification time (`Get-InstalledVersion`).
/// Unlike `query_installed`, this does **not** require both
/// the manifest and the install record to exist, so a
/// half-broken install (e.g. missing install record) can still be reset.
///
/// `index` / `total` position the package's `(k/N)` commit anchor. The batch
/// entry point [`reset`] numbers every app it will attempt across the whole
/// run — one app or many — so the counter never restarts at `(1, 1)`.
async fn reset_one(
    session: &Session,
    request: &ResetRequest,
    index: usize,
    total: usize,
) -> Fallible<()> {
    let name = request.name.as_str();
    let (pkg, pkg_dir, version_dir) =
        match resolve_reset_target(session, name, request.version.as_deref()) {
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
    // scoop-reset.ps1:71-74: test_running_process before re-linking). A
    // running app is skipped — the check printed the reason — instead of
    // aborting the reset, unless `ignore_running_processes` says to carry on.
    if check_not_running(session, name, RunningAction::Reset)? == RunningCheck::Running {
        return Ok(());
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

/// Resolve the package and version directory a `reset` should operate on,
/// tolerating broken installs the same way upstream Scoop does.
///
/// - `apps/<name>` missing or version unresolvable → [`Error::PackageNotFound`]
/// - explicit `target_version` → that version directory (must exist; rejected
///   if it contains path separators or `..`)
/// - otherwise → [`resolve_current_version`]
/// - manifest is parsed from the **version directory** (`installed_manifest`),
///   not from `current/`, so a stale/broken `current` junction doesn't matter
/// - the install record is optional (`install_info` returns `$null` when missing);
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

    // Parse the version directory's manifest (upstream `installed_manifest`:
    // `scoop-manifest.json`, else the legacy `manifest.json`).
    let not_installed = || Error::Custom(format!("'{}' ({}) isn't installed", name, version));
    let manifest_path =
        install_state::installed_manifest_path(&version_dir).ok_or_else(&not_installed)?;
    let manifest = parse_file(manifest_path).map_err(|_| not_installed())?;

    // The install record is optional; bucket defaults to the isolated bucket.
    let install_info = install_state::install_info_path(&version_dir)
        .and_then(|path| install_state::parse_install_info(&path).ok());
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

    /// Create `apps/<name>/<version>/{manifest.json, install.json}` under
    /// `root`, so `resolve_current_version` can fall back to that version dir.
    fn install_version(root: &Path, name: &str, version: &str) {
        let dir = root.join("apps").join(name).join(version);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.json"), mini_manifest(version)).unwrap();
        std::fs::write(
            dir.join("install.json"),
            r#"{"architecture": "64bit", "bucket": "main"}"#,
        )
        .unwrap();
    }

    /// Expose `apps/<name>/current/manifest.json` as a plain directory — how a
    /// current version resolves before `link_current` turns it into a junction.
    fn set_current_manifest(root: &Path, name: &str, version: &str) {
        let current = root.join("apps").join(name).join("current");
        let _ = std::fs::remove_dir_all(&current);
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("manifest.json"), mini_manifest(version)).unwrap();
    }

    /// A session whose global root lives inside the test directory: the real
    /// `global_path` is `$SCOOP_GLOBAL` or `%ProgramData%\scoop`, which a test
    /// must never read or write.
    fn test_session_with_global(root: &Path, global: &Path) -> Session {
        let config_path = root.join("hok.json");
        let escaped = |path: &Path| path.to_string_lossy().replace('\\', "\\\\");
        std::fs::write(
            &config_path,
            format!(
                r#"{{"root_path": "{}", "cache_path": "{}", "global_path": "{}"}}"#,
                escaped(root),
                escaped(&root.join("cache")),
                escaped(global)
            ),
        )
        .unwrap();
        Session::new_with(&config_path).unwrap()
    }

    /// Capture every `Output::Warn` / `Output::Error` the session emits.
    fn capture_messages(session: &Session) -> std::sync::Arc<std::sync::Mutex<Vec<String>>> {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = {
            let seen = seen.clone();
            std::sync::Arc::new(move |out: crate::output::Output| {
                if let crate::output::Output::Warn(msg) | crate::output::Output::Error(msg) = out {
                    seen.lock().unwrap().push(msg);
                }
            })
        };
        session.set_output(sink).unwrap();
        seen
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
        let messages = capture_messages(&session);
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
        reset(&session, &["doubao"], false).await.unwrap();

        // reset must not remove the app dir.
        assert!(pkg_dir.exists(), "reset must not remove the broken app dir");
        assert!(
            messages
                .lock()
                .unwrap()
                .iter()
                .any(|m| m.contains("doubao")),
            "the broken install must be reported: {:?}",
            messages.lock().unwrap()
        );
    }

    /// Every positional argument goes through upstream's `parse_app`
    /// (`scoop-reset.ps1:31`): `bucket/app`, `app@version`, both together — and
    /// `scoop` itself, which upstream skips silently (`:38-41`).
    #[test]
    fn parse_reset_query_mirrors_parse_app() {
        assert_eq!(parse_reset_query("app"), Some(("app".to_owned(), None)));
        assert_eq!(
            parse_reset_query("app@1.2.3"),
            Some(("app".to_owned(), Some("1.2.3".to_owned())))
        );
        assert_eq!(
            parse_reset_query("extras/app"),
            Some(("app".to_owned(), None))
        );
        assert_eq!(
            parse_reset_query("extras/app@2.0"),
            Some(("app".to_owned(), Some("2.0".to_owned())))
        );
        assert_eq!(parse_reset_query("scoop"), None);
    }

    /// `hok reset app@1.0.0` switches the current link to an already-installed
    /// version — the form that replaced hok's old separate `<version>`
    /// positional (`scoop-reset.ps1:16,31`).
    #[tokio::test]
    async fn reset_app_at_version_switches_the_current_link() {
        let root = tmpdir("reset_app_at_version");
        let session = test_session(&root);
        install_version(&root, "app", "1.0.0");
        install_version(&root, "app", "2.0.0");
        set_current_manifest(&root, "app", "2.0.0");

        reset(&session, &["app@1.0.0"], false).await.unwrap();

        assert_eq!(
            resolve_current_version(&root.join("apps").join("app"), false).map(|r| r.version),
            Some("1.0.0".to_owned()),
            "`app@1.0.0` must re-link `current` to the installed 1.0.0"
        );
    }

    /// A `bucket/app` query is parsed and the bucket part dropped, exactly like
    /// upstream (`scoop-reset.ps1` never forwards `$bucket` to `installed`).
    #[tokio::test]
    async fn reset_accepts_a_bucket_qualified_query() {
        let root = tmpdir("reset_bucket_qualified");
        let session = test_session(&root);
        install_version(&root, "app", "1.0.0");
        let pkg_dir = root.join("apps").join("app");
        // No `current`: the reset has to create the link for the assertion to
        // mean anything.
        assert!(!pkg_dir.join("current").exists());

        reset(&session, &["extras/app"], false).await.unwrap();

        assert!(
            pkg_dir.join("current").exists(),
            "reset must create `current`"
        );
        assert_eq!(
            resolve_current_version(&pkg_dir, false).map(|r| r.version),
            Some("1.0.0".to_owned())
        );
    }

    /// `reset a b c`: a failing app is reported, the rest keep resetting, and
    /// the `(k/N)` counter is contiguous — the skipped app is not counted
    /// (upstream's `ForEach-Object` + `return`, `scoop-reset.ps1:28-46`).
    #[tokio::test]
    async fn reset_multi_app_continues_and_numbers_contiguously() {
        let root = tmpdir("reset_multi_app");
        let session = test_session(&root);
        install_version(&root, "alpha", "1.0.0");
        install_version(&root, "beta", "1.0.0");
        let messages = capture_messages(&session);
        let mut rx = session.event_bus().receiver().expect("event bus receiver");

        reset(&session, &["alpha", "ghost", "beta"], false)
            .await
            .unwrap();

        let mut commits = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let Event::PackageCommitStart {
                name, index, total, ..
            } = event
            {
                commits.push((name, index, total));
            }
        }
        assert_eq!(
            commits,
            vec![("alpha".to_owned(), 1, 2), ("beta".to_owned(), 2, 2)],
            "the missing app must be dropped from the count, not leave a gap"
        );
        assert!(
            messages
                .lock()
                .unwrap()
                .iter()
                .any(|m| m.contains("ghost") && m.contains("isn't installed")),
            "the missing app must be reported: {:?}",
            messages.lock().unwrap()
        );
        for name in ["alpha", "beta"] {
            assert!(
                root.join("apps").join(name).join("current").exists(),
                "`{name}` must still have been reset"
            );
        }
    }

    /// Official `scoop reset` has no `-g/--global`: an app that is installed
    /// globally resets globally without a flag (`scoop-reset.ps1:33-36`).
    ///
    /// The scope predicate is asserted directly (deterministic), and the batch
    /// below proves which app dir the reset actually targeted.
    #[tokio::test]
    async fn reset_auto_detects_a_global_install() {
        let root = tmpdir("reset_global_detect");
        let global = root.join("global");
        let session = test_session_with_global(&root, &global);
        // Local install of the same name whose `current` cannot resolve.
        std::fs::create_dir_all(root.join("apps").join("app").join("current")).unwrap();
        // Global install, resolvable through its version dir, with no
        // `current` link yet — so "was reset" is observable as "link created".
        install_version(&global, "app", "2.0.0");
        let global_pkg_dir = global.join("apps").join("app");
        assert!(!global_pkg_dir.join("current").exists());

        assert!(
            reset_installed_globally(&session, "app"),
            "a global install must be auto-detected"
        );
        assert!(
            !reset_installed_globally(&session, "only-local"),
            "an app with no global install must not be detected"
        );

        let messages = capture_messages(&session);
        reset(&session, &["app"], false).await.unwrap();

        let local = root.join("apps").join("app");
        assert_eq!(
            resolve_current_version(&local, false).map(|r| r.version),
            None,
            "the local (unresolvable) install must not be the one reset"
        );
        if session.is_admin() {
            assert_eq!(
                resolve_current_version(&global_pkg_dir, false).map(|r| r.version),
                Some("2.0.0".to_owned()),
                "the auto-detected global install must have been reset"
            );
        } else {
            // `Session::is_admin()` reads the real process token and has no
            // test seam, so this arm is the one that runs on a non-elevated
            // host; it still proves the *scope* was global, because a local
            // attempt would never emit the admin warning.
            assert!(
                messages.lock().unwrap().iter().any(|m| m.contains("admin")),
                "a non-admin reset of a global app must warn and skip: {:?}",
                messages.lock().unwrap()
            );
            assert!(
                !global_pkg_dir.join("current").exists(),
                "the non-admin global reset must have been skipped"
            );
        }
    }

    /// `reset_unavailable` reports a truly broken install (version dir +
    /// manifest but no usable current version) as `Broken`, a repairable
    /// partial (usable `current` manifest) as resolvable, and a missing app as
    /// not installed at all — the three guards of `scoop-reset.ps1:43-58`.
    #[test]
    fn reset_unavailable_distinguishes_broken_from_repairable() {
        let request = |name: &str| ResetRequest {
            name: name.to_owned(),
            version: None,
            global: false,
        };

        // Not installed correctly: version dir + manifest, no install.json,
        // no `current` — reset would warn and skip.
        let root = tmpdir("reset_unavailable_broken");
        let session = test_session(&root);
        let broken = root.join("apps").join("broken");
        std::fs::create_dir_all(broken.join("1.0.0")).unwrap();
        std::fs::write(
            broken.join("1.0.0").join("manifest.json"),
            mini_manifest("1.0.0"),
        )
        .unwrap();
        assert!(
            matches!(
                reset_unavailable(&session, &request("broken")),
                Some(ResetUnavailable::Broken)
            ),
            "truly broken must not be resettable"
        );

        // Repairable partial: `current` exposes a manifest, so reset can
        // resolve a version and the app stays in the batch.
        let repairable = root.join("apps").join("repairable");
        std::fs::create_dir_all(repairable.join("2.0.0")).unwrap();
        std::fs::write(
            repairable.join("2.0.0").join("manifest.json"),
            mini_manifest("2.0.0"),
        )
        .unwrap();
        set_current_manifest(&root, "repairable", "2.0.0");
        assert!(
            reset_unavailable(&session, &request("repairable")).is_none(),
            "repairable partial must stay in the batch"
        );

        // Not installed at all: reported as such, not as broken.
        assert!(matches!(
            reset_unavailable(&session, &request("ghost")),
            Some(ResetUnavailable::NotInstalled(None))
        ));
    }
}
