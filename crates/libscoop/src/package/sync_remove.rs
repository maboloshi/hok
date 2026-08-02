//! Uninstall / reset pipeline of the sync transaction.
//!
//! This module contains the remove and reset flows — uninstall scripts,
//! shim/shortcut/env/persist cleanup, app-directory removal, and version
//! reset — split out of [`super`] (`sync.rs`). It is a private sub-module
//! of `sync`; the entry points are re-exported by `sync.rs`.

use tracing::{debug, info};

use crate::package::{operations, query, resolve, Package};
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
pub fn reset(session: &Session, name: &str, target_version: Option<&str>) -> Fallible<()> {
    let query = query::query_installed(session, &["*"], &[])?;
    let pkg = query
        .iter()
        .find(|p| p.name() == name)
        .ok_or_else(|| Error::PackageNotFound(name.to_owned()))?;

    let config = session.config();
    let apps_dir = config.root_path().join("apps");
    let pkg_dir = apps_dir.join(pkg.name());

    let installed_ver = pkg.installed_version().unwrap_or(pkg.version());
    let version = target_version.unwrap_or(installed_ver);
    let version_dir = pkg_dir.join(version);

    if !version_dir.exists() {
        return Err(Error::Custom(format!(
            "version '{}' of '{}' is not installed",
            version, name
        )));
    }

    info!("resetting {} ({})", name, version);

    // Re-create the `current` symlink
    operations::link_current(&pkg_dir, &version_dir)?;

    // Re-link persistent data
    operations::persist_link(session, pkg)?;

    // Re-create shims + shortcuts
    shim::remove(session, pkg)?;
    shim::add(session, pkg)?;
    operations::shortcut_remove(session, pkg)?;
    operations::shortcut_add(session, pkg)?;

    // Run post_install to reapply localization (fixes Scoop bug)
    run_script(
        session,
        pkg,
        &version_dir,
        "post_install",
        "install",
        pkg.manifest().post_install(),
    )?;

    Ok(())
}
