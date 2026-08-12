//! Install / upgrade pipeline of the sync transaction.
//!
//! This module contains the install flow — download, integrity check,
//! extraction, shim/shortcut creation, persist-linking, and script
//! execution — split out of [`super`] (`sync.rs`). It is a private
//! sub-module of `sync`; the entry point is re-exported by `sync.rs`.

use std::collections::HashSet;
use std::path::Path;
use tracing::debug;

use crate::package::{download, identity, manifest_source, operations, query, resolve, Package};
use crate::{
    constant::ISOLATED_PACKAGE_BUCKET, env, error::Fallible, internal, persist, psmodule, shim,
    shortcut, Error, Event, QueryOption, Session,
};

use super::{confirm_transaction, SyncOption, Transaction};

/// Sync operation: install and/or upgrade packages.
pub fn install(session: &Session, queries: &[&str], options: &[SyncOption]) -> Fallible<()> {
    let mut packages = vec![];

    let only_upgrade = options.contains(&SyncOption::OnlyUpgrade);
    let escape_hold = options.contains(&SyncOption::EscapeHold);
    let ignore_failure = options.contains(&SyncOption::IgnoreFailure);

    if only_upgrade {
        // Named packages must already be installed. The Upgradable filter
        // below would otherwise silently treat a nonexistent package (or a
        // mistyped bucket/name separator) as "nothing to do" and report
        // "all apps are up to date".
        //
        // Use the same (regex substring) matching as the Upgradable query
        // below — not Explicit — so queries like `update gcc` that match an
        // installed `gcc-libs` keep working.
        for &q in queries {
            if q == "*" {
                continue;
            }
            let matched = query::query_installed(session, &[q], &[])?;
            if matched.is_empty() {
                if ignore_failure {
                    session.output().error(format!(
                        "failed to resolve '{}': {}",
                        q,
                        Error::PackageNotFound(q.to_owned())
                    ));
                    continue;
                }
                return Err(Error::PackageNotFound(q.to_owned()));
            }
        }

        packages = query::query_installed(session, queries, &[QueryOption::Upgradable])?;

        // Replace the packages with their upgradable references.
        // Packages without an upgradable version are skipped (filter_map).
        packages = packages
            .into_iter()
            .filter_map(|p| {
                let upgradable = p.upgradable().cloned();
                if upgradable.is_none() {
                    debug!(
                        "package '{}' has no upgradable reference, skipping",
                        p.name()
                    );
                }
                upgradable
            })
            .collect::<Vec<_>>();
    } else {
        // Split queries into isolated installs (URL / local path / `@version`)
        // and regular bucket queries. Isolated installs resolve their manifest
        // directly (upstream `Get-Manifest` + `generate_user_manifest`) and are
        // installed under the `__isolated__` bucket; regular queries keep the
        // bucket-scan matching below.
        let mut isolated: Vec<Package> = Vec::new();
        let mut regular: Vec<&str> = Vec::new();

        for &query in queries {
            let aq = identity::parse_app(query);

            // `app@version` — generate (or reuse) a manifest for the version.
            if let Some(aq) = aq.as_ref().filter(|aq| aq.version.is_some()) {
                let version = aq.version.as_deref().unwrap_or_default();
                match manifest_source::generate_user_manifest(
                    session,
                    &aq.app,
                    aq.bucket.as_deref(),
                    version,
                ) {
                    Ok(resolved) => {
                        let p = Package::from(
                            &resolved.name,
                            ISOLATED_PACKAGE_BUCKET,
                            resolved.manifest,
                        );
                        if !isolated.contains(&p) {
                            isolated.push(p);
                        }
                    }
                    Err(e) => {
                        if ignore_failure {
                            session
                                .output()
                                .error(format!("failed to resolve '{}': {}", query, e));
                            continue;
                        }
                        return Err(e);
                    }
                }
                continue;
            }

            // URL / local-path manifest — resolve and install in isolation.
            if let Some(aq) = aq.as_ref() {
                let is_local = Path::new(&aq.app).exists();
                if identity::is_manifest_url(&aq.app) || is_local {
                    match manifest_source::resolve_manifest(session, &aq.app, None) {
                        Ok(resolved) => {
                            let p = Package::from(
                                &resolved.name,
                                ISOLATED_PACKAGE_BUCKET,
                                resolved.manifest,
                            );
                            if !isolated.contains(&p) {
                                isolated.push(p);
                            }
                        }
                        Err(e) => {
                            if ignore_failure {
                                session
                                    .output()
                                    .error(format!("failed to resolve '{}': {}", query, e));
                                continue;
                            }
                            return Err(e);
                        }
                    }
                    continue;
                }
            }

            regular.push(query);
        }

        // Regular bucket queries: exact-match against the synced scan.
        let synced = if regular.is_empty() {
            vec![]
        } else {
            query::query_synced(session, &regular, &[])?
        };

        for &query in &regular {
            let mut matched = synced
                .iter()
                .filter(|&p| {
                    let (query_bucket, query_name) = identity::split_bucket_query(query);
                    let bucket_matched = query_bucket
                        .as_deref()
                        .is_none_or(|b| p.bucket().eq_ignore_ascii_case(b));
                    // Exact name match, case-insensitive — Scoop is
                    // case-insensitive here (Windows FS lookup).
                    let name_matched = p.name().eq_ignore_ascii_case(query_name);
                    bucket_matched && name_matched
                })
                .cloned()
                .collect::<Vec<_>>();

            // Debug: log how many synced packages for diagnosis
            debug!(
                "query '{}': {} synced packages, {} exact matches",
                query,
                synced.len(),
                matched.len()
            );

            match matched.len() {
                0 => {
                    if ignore_failure {
                        session.output().error(format!(
                            "failed to resolve '{}': {}",
                            query,
                            Error::PackageNotFound(query.to_owned())
                        ));
                        continue;
                    }
                    return Err(Error::PackageNotFound(query.to_owned()));
                }
                1 => {
                    let p = matched.pop().unwrap();

                    if p.is_held() && !escape_hold {
                        // Skipping held package returns nothing to frontend...
                        continue;
                    }

                    if !packages.contains(&p) {
                        packages.push(p);
                    }
                }
                _ => {
                    let is_held = matched.iter().any(|p| p.is_held());

                    if is_held && !escape_hold {
                        continue;
                    }

                    if let Err(e) = resolve::select_candidate(session, &mut matched) {
                        if ignore_failure {
                            session
                                .output()
                                .error(format!("failed to resolve '{}': {}", query, e));
                            continue;
                        }
                        return Err(e);
                    }
                    let p = matched.pop().unwrap();
                    if !packages.contains(&p) {
                        packages.push(p);
                    }
                }
            }
        }

        packages.extend(isolated);
    };

    if packages.is_empty() {
        return Ok(());
    }

    let transaction = Transaction::default();

    let no_dependencies = options.contains(&SyncOption::NoDependencies);
    if !no_dependencies {
        resolve::resolve_dependencies(session, &mut packages, ignore_failure)?;
    }

    let (installed, installable): (Vec<_>, Vec<_>) =
        packages.into_iter().partition(|p| p.is_installed());

    let (upgradable, replaceable): (Vec<_>, Vec<_>) = installed
        .into_iter()
        .partition(|p| p.is_strictly_installed());

    if !only_upgrade && !installable.is_empty() {
        transaction.set_install(installable);
    }

    let upgradable = upgradable
        .into_iter()
        .filter(|p| p.upgradable_version().is_some())
        .collect::<Vec<_>>();

    let no_upgrade = options.contains(&SyncOption::NoUpgrade);
    if !no_upgrade && !upgradable.is_empty() {
        if !escape_hold {
            let (held, upgradable_list): (Vec<_>, Vec<_>) =
                upgradable.into_iter().partition(|p| p.is_held());

            // Emit PackageHeld for each held package
            for p in &held {
                if let Some(tx) = session.emitter() {
                    let _ = tx.send(Event::PackageHeld {
                        name: p.name().to_string(),
                        version: p.version().to_string(),
                    });
                }
            }

            if !upgradable_list.is_empty() {
                transaction.set_upgrade(upgradable_list);
            }
        } else {
            transaction.set_upgrade(upgradable);
        }
    }

    let no_replace = options.contains(&SyncOption::NoReplace);
    if !no_replace && !replaceable.is_empty() {
        transaction.set_replace(replaceable);
    }

    let reuse_cache = !options.contains(&SyncOption::IgnoreCache);

    let packages = transaction.add_view();
    if packages.is_empty() {
        return Ok(());
    }

    let mut set = download::PackageSet::new(session, &packages, reuse_cache)?;
    set.set_ignore_failure(ignore_failure);

    let assume_yes = options.contains(&SyncOption::AssumeYes);
    let offline = options.contains(&SyncOption::Offline);
    let mut should_offline = true;

    if !offline {
        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageDownloadSizingStart);
        }

        let download_size = set.calculate_download_size()?;
        should_offline = download_size.total == 0;
        transaction.set_download_size(download_size);
    }

    if !assume_yes && !confirm_transaction(session, &transaction)? {
        return Ok(());
    }

    // Detect apps that are still running before downloading anything
    // (mirrors scoop-update.ps1: test_running_process runs before
    // downloading the new version). A running app aborts the whole batch
    // unless `ignore_failures` is enabled — the app's failure (including
    // process-in-use) is then skipped while the rest of the batch
    // continues. Newly installed apps have no apps/<name> directory yet, so
    // they never match.
    let packages = {
        let mut kept = Vec::new();
        for &pkg in packages.iter() {
            match check_not_running(session, pkg.name(), "updating") {
                // Not running, or running but ignored (warning printed):
                // update proceeds either way (matches Scoop's
                // IGNORE_RUNNING_PROCESSES branch, which continues).
                Ok(_) => kept.push(pkg),
                Err(Error::AppRunning(name)) if ignore_failure => {
                    session
                        .output()
                        .warn(format!("Running process detected, skip updating '{name}'."));
                }
                Err(e) => return Err(e),
            }
        }
        kept
    };

    // Idents of packages that failed to download / verify and must be skipped.
    // Only populated when IgnoreFailure is enabled.
    let mut failed: HashSet<String> = HashSet::new();

    if !should_offline {
        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageDownloadStart);
        }

        failed = set.download()?.into_iter().collect();

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageDownloadDone);
        }
    }

    // Drop packages that failed to download (IgnoreFailure mode).
    let packages = if failed.is_empty() {
        packages
    } else {
        packages
            .into_iter()
            .filter(|p| !failed.contains(&p.ident()))
            .collect::<Vec<_>>()
    };

    let no_hash_check = options.contains(&SyncOption::NoHashCheck);
    failed.extend(download::verify_downloads(
        session,
        &packages,
        no_hash_check,
        ignore_failure,
    )?);

    // Drop packages that failed to download or verify (IgnoreFailure mode).
    let packages = if failed.is_empty() {
        packages
    } else {
        packages
            .into_iter()
            .filter(|p| !failed.contains(&p.ident()))
            .collect::<Vec<_>>()
    };

    let download_only = options.contains(&SyncOption::DownloadOnly);
    if !download_only {
        commit_install(session, &packages, ignore_failure)?;
    }

    Ok(())
}

/// Result of a running-process check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunningCheck {
    /// No process is running under `apps/<name>`; the operation may proceed.
    NotRunning,
    /// The app is running but `ignore_running_processes` is enabled.
    /// A warning (with the process list) was already printed; the caller
    /// decides whether to proceed or skip this package.
    Ignored,
}

/// Check if the given package's process is currently running under
/// `apps/<name>`. Returns an error if so, to prevent install/upgrade/
/// uninstall/reset while the app is in use (matches PS1's
/// `test_running_process`).
///
/// When `ignore_running_processes` is enabled, prints a warning listing the
/// running processes and returns `Ok(RunningCheck::Ignored)` instead — the
/// caller decides whether to proceed or skip the package (matches PS1's
/// `test_running_process` + `IGNORE_RUNNING_PROCESSES` branch).
pub fn check_not_running(session: &Session, name: &str, action: &str) -> Fallible<RunningCheck> {
    let app_dir = session.app_dir(name);
    let mut running = internal::os::running_processes_under(&app_dir).unwrap_or_default();

    // Exclude the current process: `hok update hok` runs from inside the
    // very app it is about to replace. Upstream Scoop never checks the
    // running scoop process (scoop-update.ps1 removes 'scoop' from the app
    // list before test_running_process); the running binary is replaced by
    // version-dir switching, so proceeding is safe.
    let self_pid = std::process::id();
    running.retain(|p| p.pid != self_pid);

    if running.is_empty() {
        return Ok(RunningCheck::NotRunning);
    }
    if session.config().ignore_running_processes() {
        let procs = running
            .iter()
            .map(|p| format!("  {} (pid {})", p.name, p.pid))
            .collect::<Vec<_>>()
            .join("\n");
        session.output().warn(format!(
            "'{name}' is still running. hok is configured to ignore this condition \
             (ignore_running_processes), continuing to {action}.\n{procs}"
        ));
        return Ok(RunningCheck::Ignored);
    }
    Err(Error::AppRunning(name.to_owned()))
}

/// Commit package installation: extract files, run scripts, create symlinks,
/// shims, and shortcuts.
fn commit_install(session: &Session, packages: &[&Package], ignore_failure: bool) -> Fallible<()> {
    for &pkg in packages.iter() {
        if let Err(e) = commit_one_install(session, pkg) {
            let msg = format!("failed to install '{}': {}", pkg.name(), e);
            if ignore_failure {
                session.output().error(msg);
                continue;
            }
            return Err(Error::Custom(msg));
        }
    }
    Ok(())
}

fn commit_one_install(session: &Session, pkg: &Package) -> Fallible<()> {
    let config = session.config();
    let apps_dir = session.apps_dir();

    // The install layout version: upstream rewrites `nightly` to
    // `nightly-YYYYMMDD` (lib/install.ps1:21-25), giving each daily build
    // its own versioned directory.
    let version = pkg.effective_version();

    let working_dir = session.versioned_dir(pkg.name(), &version);
    internal::fs::ensure_dir(&working_dir)?;

    debug!("commit: {} v{} - starting", pkg.name(), pkg.version());

    if let Some(tx) = session.emitter() {
        let old_ver = pkg.installed_version().unwrap_or_default().to_owned();
        let new_ver = pkg.version().to_owned();
        let _ = tx.send(Event::PackageVersionKnown {
            name: pkg.name().to_owned(),
            old_version: old_ver,
            new_version: new_ver,
        });
        let _ = tx.send(Event::PackageCommitStart(pkg.name().to_owned()));
    }

    // 1. extract/copy downloaded files
    let archives = operations::extract_archives(session, pkg, &working_dir)?;
    operations::copy_downloaded_files(session, pkg, &working_dir, &archives)?;

    // 2. pre_install (Scoop order: after extract/copy, before link_current)
    if pkg.manifest().pre_install().is_some() {
        debug!("commit: {} v{} - pre_install", pkg.name(), pkg.version());
        operations::run_script(
            session,
            pkg,
            &working_dir,
            None,
            "pre_install",
            "install",
            pkg.manifest().pre_install(),
        )?;
    }

    // 3. installer, $dir = version dir)
    if let Some(installer) = pkg.manifest().installer() {
        // 1. Installer file. Runs when `installer.file` is set *or* when
        //    only `installer.args` is given — upstream falls back to the
        //    first download URL's filename (Invoke-Installer,
        //    lib/install.ps1:110-115). `installer.script` is executed
        //    afterwards regardless (see step 2).
        let raw_args: Vec<&str> = installer.args().unwrap_or_default();
        if installer.file().is_some() || !raw_args.is_empty() {
            let file = match installer.file() {
                Some(f) => f.to_owned(),
                None => {
                    let first_url = pkg.manifest().url().first().copied().unwrap_or("");
                    internal::url::url_filename(first_url).to_owned()
                }
            };
            debug!(
                "commit: {} v{} - installer.file ({file})",
                pkg.name(),
                pkg.version()
            );
            operations::run_installer_file(
                session,
                pkg,
                &working_dir,
                "installer",
                "install",
                &file,
                &raw_args,
            )?;
            // Don't remove the installer file when `keep` is set (Scoop
            // order: `!$installer.keep` -> Remove-Item, lib/install.ps1).
            if !installer.keep() {
                let installer_path = working_dir.join(&file);
                if installer_path.exists() {
                    std::fs::remove_file(&installer_path)?;
                }
            }
        }

        // 2. installer.script runs regardless of the file step (upstream
        //    Invoke-HookScript is called after Invoke-Installer,
        //    lib/install.ps1:144,163-166).
        if let Some(script) = installer.script() {
            debug!(
                "commit: {} v{} - installer.script",
                pkg.name(),
                pkg.version()
            );
            operations::run_script(
                session,
                pkg,
                &working_dir,
                None,
                "installer",
                "install",
                Some(script),
            )?;
        }
    }

    // 3.25 Undo installers that added the app directory to PATH
    // (Scoop order: `ensure_install_dir_not_in_path` right after the
    // installer, before `link_current`).
    env::ensure_install_dir_not_in_path(session, &working_dir)?;

    // 3.5 Upgrade: clean up the old version's env before relinking
    // (mirrors scoop-update.ps1, which runs env_rm_path/env_rm against
    // $old_manifest before the new version is installed).
    if let Some(old_version) = pkg.installed_version() {
        debug!(
            "commit: {} v{} - env remove (old {})",
            pkg.name(),
            pkg.version(),
            old_version
        );
        let version = session.current_dir_name(old_version);
        // Locate the old manifest: with junctions, `current` still points at
        // the old version; without junctions, use the versioned dir directly.
        // Fall back to the new manifest if the old one can't be read.
        let old_manifest_path = if config.no_junction() {
            apps_dir
                .join(pkg.name())
                .join(old_version)
                .join("manifest.json")
        } else {
            session.current_dir(pkg.name()).join("manifest.json")
        };
        let old_manifest = crate::package::Manifest::parse(old_manifest_path).ok();
        match old_manifest {
            Some(m) => env::remove_with_manifest(session, pkg, &m, version)?,
            None => env::remove(session, pkg)?,
        }
    }

    // 4. link_current (Scoop order: after installer, before shims)
    debug!("commit: {} v{} - link_current", pkg.name(), pkg.version());
    operations::link_current(
        &apps_dir.join(pkg.name()),
        &working_dir,
        config.no_junction(),
    )?;

    // 5. shims + shortcuts
    debug!(
        "commit: {} v{} - shims/shortcuts",
        pkg.name(),
        pkg.version()
    );
    shim::add(session, pkg)?;
    shortcut::add(session, pkg)?;

    // 5.25 psmodule (Scoop order: after shims/shortcuts, before env)
    debug!("commit: {} v{} - psmodule", pkg.name(), pkg.version());
    psmodule::add(session, pkg)?;

    // 5.5 env (Scoop order: after shims/shortcuts, before persist)
    debug!("commit: {} v{} - env", pkg.name(), pkg.version());
    env::add(session, pkg)?;

    // 6. persist (Scoop order: after shims, before post_install)
    debug!("commit: {} v{} - persist", pkg.name(), pkg.version());
    persist::link(session, pkg)?;

    // 7. post_install (Scoop order: last hook). Runs with `$dir` = the
    // `current` junction (upstream `link_current` reassigns $dir), while
    // `$original_dir` stays the real version dir.
    if pkg.manifest().post_install().is_some() {
        debug!("commit: {} v{} - post_install", pkg.name(), pkg.version());
        let runtime_dir = session
            .app_dir(pkg.name())
            .join(session.current_dir_name(&version));
        operations::run_script(
            session,
            pkg,
            &runtime_dir,
            Some(&working_dir),
            "post_install",
            "install",
            pkg.manifest().post_install(),
        )?;
    }

    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackageCommitDone(pkg.name().to_owned()));
    }

    // Emit post-install notes if the manifest has them.
    // Mirror Scoop's `show_notes` (install.ps1): substitute the Scoop path
    // placeholders (`$dir`, `$original_dir`, `$persist_dir`, etc.) with real
    // paths — full expansion via `expand_scoop_str` matches Scoop's
    // `substitute` (all params, not just the three path vars).
    if let Some(notes) = pkg.manifest().notes() {
        let notes_text = notes
            .iter()
            .map(|n| operations::expand_scoop_str(n, session, pkg, &working_dir, "install"))
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageNotes(notes_text));
        }
    }

    debug!(
        "commit: {} v{} - writing metadata",
        pkg.name(),
        pkg.version()
    );

    // 7. Write install metadata
    // The metadata dir mirrors the `$dir` upstream uses after `link_current`
    // (lib/install.ps1): the junction path with junctions, the version dir
    // under `NO_JUNCTION`.
    let meta_dir = session
        .app_dir(pkg.name())
        .join(session.current_dir_name(&version));

    // 1. Copy manifest from bucket to <meta_dir>/manifest.json
    let manifest_dst = meta_dir.join("manifest.json");
    if pkg.bucket() == ISOLATED_PACKAGE_BUCKET {
        // Isolated installs (URL / local path / generated version): persist
        // the raw manifest text captured at resolve time; fall back to
        // copying the source file.
        match pkg.manifest().raw() {
            Some(raw) => match std::fs::write(&manifest_dst, raw) {
                Ok(_) => {}
                Err(e) => {
                    return Err(Error::Custom(format!(
                        "could not write manifest to {:?}: {}",
                        manifest_dst, e
                    )))
                }
            },
            None => match std::fs::copy(pkg.manifest().path(), &manifest_dst) {
                Ok(_) => {}
                Err(e) => {
                    return Err(Error::Custom(format!(
                        "could not copy manifest from {:?}: {}",
                        pkg.manifest().path(),
                        e
                    )))
                }
            },
        }
    } else {
        // Use bucket path (manifest.path() may be virtual when loaded from cache)
        let bucket_path = session.bucket_dir(pkg.bucket());
        let manifest_src = {
            let primary = bucket_path
                .join("bucket")
                .join(format!("{}.json", pkg.name()));
            let fallback = bucket_path.join(format!("{}.json", pkg.name()));
            if primary.exists() {
                primary
            } else {
                fallback
            }
        };
        match std::fs::copy(&manifest_src, manifest_dst) {
            Ok(_) => {}
            Err(e) => {
                return Err(Error::Custom(format!(
                    "could not copy manifest from {:?}: {}",
                    manifest_src, e
                )))
            }
        }
    }

    // 2. Write <meta_dir>/install.json
    // (upstream `save_install_info` stores architecture, url, bucket —
    //  lib/install.ps1:73; the architecture is the *selected* one, i.e.
    //  after the `default_architecture` config and `-a/--arch` overrides)
    let arch = crate::internal::arch::Arch::current().name();
    // Isolated installs record the manifest source (URL / local path /
    // generated workspace file) as `url`, mirroring upstream
    // `save_install_info`; bucket installs keep the download URL.
    let install_url = if pkg.bucket() == ISOLATED_PACKAGE_BUCKET {
        Some(pkg.manifest().path().display().to_string())
    } else {
        pkg.download_urls().first().map(|s| s.to_string())
    };
    let install_info = serde_json::json!({
        "architecture": arch,
        "url": install_url,
        // Isolated installs carry no bucket (upstream `save_install_info`
        // drops null values; readers fall back to `ISOLATED_PACKAGE_BUCKET`).
        "bucket": if pkg.bucket() == ISOLATED_PACKAGE_BUCKET {
            serde_json::Value::Null
        } else {
            serde_json::json!(pkg.bucket())
        },
    });
    if let Err(e) = internal::fs::write_json(meta_dir.join("install.json"), &install_info) {
        return Err(Error::Custom(format!("install.json write: {}", e)));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that installer.file execution path works correctly.
    #[test]
    fn test_installer_file_execution() {
        let tmp = crate::test_utils::tmpdir("installer_file");

        // Test 1: capture exit code via cmd.exe /c
        let exit_code = crate::internal::os::run_gui(
            &std::path::PathBuf::from("cmd.exe"),
            &["/c", "exit /b 42"],
            Some(&tmp),
        )
        .unwrap();
        assert_eq!(exit_code, 42, "should capture exit code from cmd.exe /c");

        // Test 2: create a file via PowerShell (used in many Scoop installer scripts)
        let marker = tmp.join("ran.txt");
        let status = crate::internal::os::ps_command()
            .arg("-Command")
            .arg(format!(
                "New-Item -Path '{}' -ItemType File -Force | Out-Null",
                marker.display()
            ))
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(0), "powershell script should exit 0");
        assert!(
            marker.exists(),
            "powershell should have created marker file"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Test URL fragment rename: url#/installer.exe → copy directly as installer.exe
    #[test]
    fn test_url_fragment_rename() {
        let tmp = crate::test_utils::tmpdir("fragment_rename");

        // Simulate cache file with hash-based name
        let cache_file = tmp.join("pkg#1.0#abc1234.exe");
        std::fs::write(&cache_file, b"dummy").unwrap();
        let work_dir = tmp.join("work");
        std::fs::create_dir_all(&work_dir).unwrap();

        // Simulate the copy logic: url#/installer.exe → target = "installer.exe"
        let url = "https://example.com/setup.exe#/installer.exe";
        let cache_name = "pkg#1.0#abc1234.exe";
        let target_name = url.split('#').nth(1).unwrap().trim_start_matches('/');
        assert_eq!(target_name, "installer.exe");

        let dst = work_dir.join(target_name);
        std::fs::copy(&cache_file, &dst).unwrap();
        assert!(dst.exists(), "should copy directly as installer.exe");
        assert!(!work_dir.join(cache_name).exists(), "no hash-named copy");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Test URL without fragment: use original filename from URL path
    #[test]
    fn test_url_filename_without_fragment() {
        let url = "https://example.com/dopus_patcher.exe";
        let filename = std::path::Path::new(url)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap();
        assert_eq!(filename, "dopus_patcher.exe");
    }

    // ── only_upgrade existence check ─────────────────────────────────────────

    const TEST_MANIFEST: &str =
        r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#;

    /// A named package that is not installed must fail fast instead of
    /// silently reporting "all apps are up to date".
    #[test]
    fn only_upgrade_missing_package_errors() {
        let root = crate::test_utils::tmpdir("only_upgrade_missing");
        let session = crate::test_utils::test_session(&root);

        let err = install(&session, &["ghost"], &[SyncOption::OnlyUpgrade]).unwrap_err();
        assert!(matches!(err, Error::PackageNotFound(name) if name == "ghost"));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// An installed package without a newer version must NOT be reported as
    /// missing — the Upgradable filter below silently skips it.
    #[test]
    fn only_upgrade_installed_without_upgrade_is_ok() {
        let root = crate::test_utils::tmpdir("only_upgrade_no_new");
        let session = crate::test_utils::test_session(&root);

        crate::test_utils::mark_installed(&root, "curl", "main", TEST_MANIFEST, false);

        // No bucket manifest → nothing upgradable → Ok, no error raised.
        install(&session, &["curl"], &[SyncOption::OnlyUpgrade]).unwrap();

        let _ = std::fs::remove_dir_all(&root);
    }

    /// With IgnoreFailure, a missing package is skipped (reported to stderr)
    /// instead of aborting the whole upgrade.
    #[test]
    fn only_upgrade_ignore_failure_skips_missing() {
        let root = crate::test_utils::tmpdir("only_upgrade_ignore");
        let session = crate::test_utils::test_session(&root);

        crate::test_utils::mark_installed(&root, "curl", "main", TEST_MANIFEST, false);

        // "ghost" is missing but IgnoreFailure lets the rest proceed.
        install(
            &session,
            &["ghost", "curl"],
            &[SyncOption::OnlyUpgrade, SyncOption::IgnoreFailure],
        )
        .unwrap();

        let _ = std::fs::remove_dir_all(&root);
    }

    // ── isolated installs (URL / local path / @version) ────────────────────

    const NO_DOWNLOAD_MANIFEST: &str = r#"{
        "version": "1.0.0",
        "homepage": "https://example.com",
        "license": "MIT"
    }"#;

    /// A local-path manifest installs as an isolated package: `manifest.json`
    /// persists the raw source text and `install.json` records the source
    /// path with no bucket.
    #[test]
    fn install_local_path_manifest_is_isolated() {
        let root = crate::test_utils::tmpdir("install_local_path");
        let session = crate::test_utils::test_session(&root);

        let manifest_path = root.join("myapp.json");
        std::fs::write(&manifest_path, NO_DOWNLOAD_MANIFEST).unwrap();

        install(&session, &[manifest_path.to_str().unwrap()], &[]).unwrap();

        let app_dir = root.join("apps").join("myapp");
        let meta_dir = app_dir.join("current");
        assert!(meta_dir.is_dir(), "app should be installed under current");

        let saved = std::fs::read_to_string(meta_dir.join("manifest.json")).unwrap();
        assert_eq!(saved, NO_DOWNLOAD_MANIFEST, "raw manifest text persisted");

        let install_info: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(meta_dir.join("install.json")).unwrap())
                .unwrap();
        assert!(
            install_info["bucket"].is_null(),
            "isolated installs carry no bucket: {}",
            install_info
        );
        let url = install_info["url"].as_str().unwrap();
        assert!(
            url.ends_with("myapp.json"),
            "install.json url records the manifest source: {}",
            url
        );
        assert!(
            url.contains("apps") == false,
            "url is the source, not an app path"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// An `app@version` query where the version matches the resolved
    /// manifest installs that manifest as an isolated package.
    #[test]
    fn install_version_matching_manifest_is_isolated() {
        let root = crate::test_utils::tmpdir("install_version_match");
        let session = crate::test_utils::test_session(&root);

        let manifest_path = root.join("myapp.json");
        std::fs::write(&manifest_path, NO_DOWNLOAD_MANIFEST).unwrap();

        install(
            &session,
            &[&format!("{}@1.0.0", manifest_path.display())],
            &[],
        )
        .unwrap();

        let meta_dir = root.join("apps").join("myapp").join("current");
        let saved = std::fs::read_to_string(meta_dir.join("manifest.json")).unwrap();
        assert_eq!(saved, NO_DOWNLOAD_MANIFEST);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A `name@version` query without autoupdate capability fails with the
    /// upstream error instead of installing the wrong version.
    #[test]
    fn install_version_without_autoupdate_errors() {
        let root = crate::test_utils::tmpdir("install_version_no_au");
        let session = crate::test_utils::test_session(&root);

        let manifest_path = root.join("myapp.json");
        std::fs::write(&manifest_path, NO_DOWNLOAD_MANIFEST).unwrap();

        let err = install(
            &session,
            &[&format!("{}@2.0.0", manifest_path.display())],
            &[],
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("does not have autoupdate capability"),
            "{}",
            err
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // ── case-insensitive package queries ──────────────────────────────────

    /// `hok install Obsidian` must resolve the lowercase `obsidian` manifest,
    /// mirroring Scoop's case-insensitive package lookup (Windows FS).
    #[test]
    fn install_query_is_case_insensitive() {
        let root = crate::test_utils::tmpdir("install_case_insensitive");
        let session = crate::test_utils::test_session(&root);
        crate::test_utils::write_bucket_manifest(&root, "main", "obsidian", NO_DOWNLOAD_MANIFEST);

        install(&session, &["Obsidian"], &[]).unwrap();

        let app_dir = root.join("apps").join("obsidian");
        assert!(app_dir.join("current").is_dir(), "app should be installed");
        assert!(
            app_dir.join("current").join("manifest.json").exists(),
            "manifest persisted"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Bucket prefix matching is case-insensitive too: `Main/Obsidian` finds
    /// the `main` bucket's `obsidian` manifest.
    #[test]
    fn install_query_bucket_prefix_is_case_insensitive() {
        let root = crate::test_utils::tmpdir("install_bucket_case");
        let session = crate::test_utils::test_session(&root);
        crate::test_utils::write_bucket_manifest(&root, "main", "obsidian", NO_DOWNLOAD_MANIFEST);

        install(&session, &["Main/Obsidian"], &[]).unwrap();

        let app_dir = root.join("apps").join("obsidian");
        assert!(app_dir.join("current").is_dir(), "app should be installed");

        let _ = std::fs::remove_dir_all(&root);
    }
}
