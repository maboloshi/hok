//! Install / upgrade pipeline of the sync transaction.
//!
//! This module contains the install flow — download, integrity check,
//! extraction, shim/shortcut creation, persist-linking, and script
//! execution — split out of [`super`] (`sync.rs`). It is a private
//! sub-module of `sync`; the entry point is re-exported by `sync.rs`.

use crate::internal::hash::ChecksumBuilder;
use std::collections::HashSet;
use std::io::Read;
use std::path::Path;
use tracing::{debug, info};

use crate::package::{download, identity, operations, query, resolve, Package};
use crate::{error::Fallible, env, internal, shim, Error, Event, QueryOption, Session};

use super::{confirm_transaction, SyncOption, TempFileGuard, Transaction};

/// Execute a PowerShell script defined in a package manifest.
///
/// `script_lines` is an array of PowerShell command lines that will be joined
/// and executed via `powershell.exe`. The function is a no-op if `script_lines`
/// is `None`.
///
/// Environment variables set for the script:
/// - `SCOOP` — the Scoop root directory
/// - `SCOOP_APP_DIR` — the package's installation directory
/// - `SCOOP_PACKAGE_NAME` — the package name
/// - `SCOOP_PACKAGE_VERSION` — the installed version
/// - `version` — same as SCOOP_PACKAGE_VERSION (Scoop convention)
pub(super) fn run_script(
    session: &Session,
    package: &Package,
    working_dir: &Path,
    stage: &str,
    cmd: &str,
    script_lines: Option<Vec<&str>>,
) -> Fallible<()> {
    let lines = match script_lines {
        Some(l) if !l.is_empty() => l,
        _ => return Ok(()),
    };

    debug!(
        "run_script: {} stage={} ({} lines)",
        package.name(),
        stage,
        lines.len()
    );

    let script = lines.join("\r\n");

    // Embed PS helper scripts so package scripts can use functions
    // like Expand-InnoArchive, Expand-7zipArchive, Get-HelperPath, etc.
    const CORE_PS1: &str = include_str!("../../../../asset_scripts/core.ps1");
    const DECOMPRESS_PS1: &str = include_str!("../../../../asset_scripts/decompress.ps1");
    let preamble = format!(
        r#"
# Hok embedded helpers (always available)
{core}
{decompress}
# Fallback: load Scoop lib if installed
$hokScoopLib = Join-Path $env:SCOOP "apps\scoop\current\lib"
if (Test-Path $hokScoopLib) {{
    Get-ChildItem $hokScoopLib -Filter *.ps1 | ForEach-Object {{ . $_.FullName -ErrorAction SilentlyContinue }}
}}

# Notify hok about undefined commands (missing helpers)
trap {{
    if ($_.Exception -is [System.Management.Automation.CommandNotFoundException]) {{
        Write-Host "[[HOK_MISSING_HELPER]]$($_.Exception.CommandName)"
    }}
    continue
}}

# Scoop-compatible variables for package scripts
$dir = $env:SCOOP_APP_DIR
$original_dir = $dir
$scoopdir = $env:SCOOP
$bucketsdir = Join-Path $scoopdir "buckets"
$persist_dir = Join-Path $scoopdir "persist" $env:SCOOP_PACKAGE_NAME
$version = $env:SCOOP_PACKAGE_VERSION
$app = $env:SCOOP_PACKAGE_NAME
$bucket = $env:SCOOP_PACKAGE_BUCKET
$architecture = "64bit"
$global = $false
$cmd = $env:SCOOP_PACKAGE_CMD
"#,
        core = CORE_PS1,
        decompress = DECOMPRESS_PS1
    );
    let full_script = format!("{preamble}\r\n{script}");

    // Write script to a temp file in the working dir
    let script_path = working_dir.join(format!("{}.ps1", stage));
    if let Some(parent) = script_path.parent() {
        internal::fs::ensure_dir(parent)?;
    }
    std::fs::write(&script_path, &full_script)?;

    // Build environment variables
    let root_path = session.effective_root_path();
    let pkg_dir = working_dir.to_path_buf(); // $dir = version dir (not current)

    let version = package.version();

    // Create marker file for P2 extraction routing
    let marker_path = working_dir.join("hok_extract_markers.txt");
    let _ = std::fs::remove_file(&marker_path); // clean from previous runs

    // Ensure both temp files are cleaned up on return, even via `?`
    let _guard = TempFileGuard::new(vec![script_path.clone(), marker_path.clone()]);

    // Prefer pwsh.exe (PowerShell Core, faster startup)
    let ps_exe = if crate::internal::os::is_pwsh_available() {
        "pwsh.exe"
    } else {
        "powershell.exe"
    };

    let status = std::process::Command::new(ps_exe)
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(&script_path)
        .env("SCOOP", root_path.as_os_str())
        .env("SCOOP_APP_DIR", pkg_dir.as_os_str())
        .env("SCOOP_PACKAGE_NAME", package.name())
        .env("SCOOP_PACKAGE_VERSION", version)
        .env("SCOOP_PACKAGE_BUCKET", package.bucket())
        .env("SCOOP_PACKAGE_CMD", cmd)
        .env("version", version)
        .env("HOK_EXTRACT_FILE", marker_path.as_os_str())
        .status()
        .map_err(|e| {
            Error::Custom(format!(
                "failed to run {} script for '{}': {}",
                stage,
                package.name(),
                e
            ))
        })?;

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        return Err(Error::Custom(format!(
            "{} script for '{}' exited with code {}",
            stage,
            package.name(),
            code
        )));
    }

    // Process extraction markers (P2: Rust native extraction)
    operations::extract_markers(session, working_dir);

    Ok(())
}
/// Expand Scoop-style variables (`$dir`, `$scoopdir`, `$persist_dir`, etc.)
/// in installer/uninstaller args, replacing them with the actual filesystem paths.
///
/// This mirrors the variable definitions in `run_script`'s PowerShell preamble,
/// so that `installer.file` and `uninstaller.file` (which run via `run_gui`
/// rather than through PowerShell) get equivalent variable expansion.
pub(crate) fn expand_installer_vars(
    args: &[&str],
    session: &Session,
    pkg: &Package,
    working_dir: &Path,
    cmd: &str,
) -> Vec<String> {
    let root_path = session.effective_root_path();
    let persist_dir = root_path.join("persist").join(pkg.name());
    let buckets_dir = root_path.join("buckets");
    let version = pkg.version();
    let app = pkg.name();
    let bucket = pkg.bucket();
    let architecture = if cfg!(target_arch = "x86_64") {
        "64bit"
    } else if cfg!(target_arch = "x86") {
        "32bit"
    } else {
        "arm64"
    };

    let working_dir_str = working_dir.to_string_lossy().to_string();
    let root_path_str = root_path.to_string_lossy().to_string();
    let persist_dir_str = persist_dir.to_string_lossy().to_string();
    let buckets_dir_str = buckets_dir.to_string_lossy().to_string();

    args.iter()
        .map(|arg| {
            let mut s = arg.to_string();
            // Longer/replace more specific patterns first to avoid partial overlap
            s = s.replace("$original_dir", &working_dir_str);
            s = s.replace("$persist_dir", &persist_dir_str);
            s = s.replace("$bucketsdir", &buckets_dir_str);
            s = s.replace("$scoopdir", &root_path_str);
            s = s.replace("$architecture", architecture);
            s = s.replace("$version", version);
            s = s.replace("$app", app);
            s = s.replace("$bucket", bucket);
            s = s.replace("$global", "false");
            s = s.replace("$cmd", cmd);
            s = s.replace("$dir", &working_dir_str);
            s
        })
        .collect()
}

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
                    eprintln!(
                        "failed to resolve '{}': {}",
                        q,
                        Error::PackageNotFound(q.to_owned())
                    );
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
        let synced = query::query_synced(session, queries, &[])?;

        for &query in queries {
            let mut matched = synced
                .iter()
                .filter(|&p| {
                    let (query_bucket, query_name) = identity::split_bucket_query(query);
                    let bucket_matched = query_bucket
                        .as_deref()
                        .map_or(true, |b| p.bucket() == b);
                    let name_matched = p.name() == query_name;
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
                        eprintln!(
                            "failed to resolve '{}': {}",
                            query,
                            Error::PackageNotFound(query.to_owned())
                        );
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
                            eprintln!("failed to resolve '{}': {}", query, e);
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
    if !no_hash_check {
        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageIntegrityCheckStart);
        }

        let config = session.config();
        let cache_root = config.cache_path();

        let mut buf = [0; 1024 * 64];

        for &pkg in packages.iter() {
            if pkg.version() == "nightly" {
                info!("skip hash check for nightly package '{}'", pkg.name());
                continue;
            }

            let files = pkg.download_filenames();
            let hashes = pkg.download_hashes();
            let files_cnt = files.len();

            let result = (|| -> Fallible<()> {
                for (idx, (filename, hash)) in files.into_iter().zip(hashes).enumerate() {
                    let path = cache_root.join(filename);

                    let mut hasher = ChecksumBuilder::new().algo(hash.algorithm())?.build();

                    if let Some(tx) = session.emitter() {
                        let progress = format!("{} ({}/{})", pkg.name(), idx + 1, files_cnt);
                        let _ = tx.send(Event::PackageIntegrityCheckProgress(progress));
                    }

                    let mut file = std::fs::File::open(path)?;
                    loop {
                        let len = file.read(&mut buf)?;
                        if len == 0 {
                            break;
                        }
                        hasher.consume(&buf[..len]);
                    }

                    let actual = hasher.finalize();
                    let expected = hash.value();
                    if actual != expected {
                        let name = pkg.name().to_owned();
                        let url = pkg.download_urls()[idx].to_owned();
                        let ctx = crate::package::HashMismatchContext::new(
                            name,
                            url,
                            expected.to_owned(),
                            actual,
                        );
                        return Err(Error::HashMismatch(ctx));
                    }
                }
                Ok(())
            })();

            if let Err(e) = result {
                if ignore_failure {
                    failed.insert(pkg.ident());
                    eprintln!("failed to verify '{}': {}", pkg.name(), e);
                } else {
                    return Err(e);
                }
            }
        }

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageIntegrityCheckDone);
        }
    }

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

/// Commit package installation: extract files, run scripts, create symlinks,
/// shims, and shortcuts.
/// Check if the given package's process is currently running under the apps
/// directory. Returns an error if so, to prevent install/upgrade/uninstall
/// while the app is in use (matches PS1's test_running_process).
pub(super) fn check_not_running(session: &Session, name: &str, action: &str) -> Fallible<()> {
    let apps_dir = session.effective_root_path().join("apps");
    let running = internal::os::running_apps(&apps_dir).unwrap_or_default();
    if running.iter().any(|p| p.eq_ignore_ascii_case(name)) {
        return Err(Error::Custom(format!(
            "'{}' is still running! Close the app(s) before {}.",
            name, action
        )));
    }
    Ok(())
}

fn commit_install(session: &Session, packages: &[&Package], ignore_failure: bool) -> Fallible<()> {
    for &pkg in packages.iter() {
        if let Err(e) = commit_one_install(session, pkg) {
            let msg = format!("failed to install '{}': {}", pkg.name(), e);
            if ignore_failure {
                eprintln!("{}", msg);
                continue;
            }
            return Err(Error::Custom(msg));
        }
    }
    Ok(())
}

fn commit_one_install(session: &Session, pkg: &Package) -> Fallible<()> {
    let config = session.config();
    let apps_dir = session.effective_root_path().join("apps");

    // Check if the app is currently running before installing/upgrading
    check_not_running(session, pkg.name(), "installing")?;

    let working_dir = apps_dir.join(pkg.name()).join(pkg.version());
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
        run_script(
            session,
            pkg,
            &working_dir,
            "pre_install",
            "install",
            pkg.manifest().pre_install(),
        )?;
    }

    // 3. installer, $dir = version dir)
    if let Some(installer) = pkg.manifest().installer() {
        if let Some(script) = installer.script() {
            debug!(
                "commit: {} v{} - installer.script",
                pkg.name(),
                pkg.version()
            );
            run_script(
                session,
                pkg,
                &working_dir,
                "installer",
                "install",
                Some(script),
            )?;
        } else if let Some(file) = installer.file() {
            debug!("commit: {} v{} - installer.file", pkg.name(), pkg.version());
            let exe_path = working_dir.join(file);
            let raw_args: Vec<&str> = installer.args().unwrap_or_default();
            let expanded = expand_installer_vars(&raw_args, session, pkg, &working_dir, "install");
            let args: Vec<&str> = expanded.iter().map(|s| s.as_str()).collect();
            crate::internal::os::run_gui(&exe_path, &args, Some(&working_dir)).map_err(|e| {
                Error::Custom(format!(
                    "failed to run installer '{}' for '{}': {}",
                    file,
                    pkg.name(),
                    e
                ))
            })?;
        }
    }

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
        let version = if config.no_junction() {
            old_version.to_owned()
        } else {
            "current".to_owned()
        };
        // Locate the old manifest: with junctions, `current` still points at
        // the old version; without junctions, use the versioned dir directly.
        // Fall back to the new manifest if the old one can't be read.
        let old_manifest_path = if config.no_junction() {
            apps_dir
                .join(pkg.name())
                .join(old_version)
                .join("manifest.json")
        } else {
            apps_dir
                .join(pkg.name())
                .join("current")
                .join("manifest.json")
        };
        let old_manifest = crate::package::Manifest::parse(old_manifest_path).ok();
        match old_manifest {
            Some(m) => env::remove_with_manifest(session, pkg, &m, &version)?,
            None => env::remove(session, pkg)?,
        }
    }

    // 4. link_current (Scoop order: after installer, before shims)
    debug!("commit: {} v{} - link_current", pkg.name(), pkg.version());
    operations::link_current(&apps_dir.join(pkg.name()), &working_dir)?;

    // 5. shims + shortcuts
    debug!(
        "commit: {} v{} - shims/shortcuts",
        pkg.name(),
        pkg.version()
    );
    shim::add(session, pkg)?;
    operations::shortcut_add(session, pkg)?;

    // 5.5 env (Scoop order: after shims/shortcuts, before persist)
    debug!("commit: {} v{} - env", pkg.name(), pkg.version());
    env::add(session, pkg)?;

    // 6. persist (Scoop order: after shims, before post_install)
    debug!("commit: {} v{} - persist", pkg.name(), pkg.version());
    operations::persist_link(session, pkg)?;

    // 7. post_install (Scoop order: last hook)
    if pkg.manifest().post_install().is_some() {
        debug!("commit: {} v{} - post_install", pkg.name(), pkg.version());
        run_script(
            session,
            pkg,
            &working_dir,
            "post_install",
            "install",
            pkg.manifest().post_install(),
        )?;
    }

    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackageCommitDone(pkg.name().to_owned()));
    }

    // Emit post-install notes if the manifest has them
    if let Some(notes) = pkg.manifest().notes() {
        let notes_text = notes.join("\n");
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
    let current_dir = apps_dir.join(pkg.name()).join("current");

    // 1. Copy manifest from bucket to current/manifest.json
    // Use bucket path (manifest.path() may be virtual when loaded from cache)
    let bucket_path = config.root_path().join("buckets").join(pkg.bucket());
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
    let manifest_dst = current_dir.join("manifest.json");
    match std::fs::copy(&manifest_src, manifest_dst) {
        Ok(_) => {}
        Err(e) => {
            return Err(Error::Custom(format!(
                "could not copy manifest from {:?}: {}",
                manifest_src, e
            )))
        }
    }

    // 2. Write current/install.json
    let arch = if cfg!(target_arch = "x86_64") {
        "64bit"
    } else if cfg!(target_arch = "x86") {
        "32bit"
    } else {
        "arm64"
    };
    let install_info = serde_json::json!({
        "architecture": arch,
        "bucket": pkg.bucket(),
    });
    if let Err(e) = internal::fs::write_json(current_dir.join("install.json"), &install_info) {
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
        let ps = if crate::internal::os::is_pwsh_available() {
            "pwsh.exe"
        } else {
            "powershell.exe"
        };
        let exit_code = crate::internal::os::run_gui(
            &std::path::PathBuf::from(ps),
            &[
                "-NoProfile",
                "-Command",
                &format!(
                    "New-Item -Path '{}' -ItemType File -Force | Out-Null",
                    marker.display()
                ),
            ],
            Some(&tmp),
        )
        .unwrap();
        assert_eq!(exit_code, 0, "powershell script should exit 0");
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

    /// Helper to create a test environment for expand_installer_vars tests.
    /// Cleans up the temp dir on drop via the returned guard.
    struct TestDirGuard(std::path::PathBuf);
    impl Drop for TestDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn setup_expand_vars_test(
        test_name: &str,
    ) -> (crate::Session, Package, std::path::PathBuf, TestDirGuard) {
        let tmp = crate::test_utils::tmpdir(&format!("expand_vars_{}", test_name));
        let guard = TestDirGuard(tmp.clone());
        let root = &tmp;

        // Write minimal Scoop config
        let config_path = root.join("config.json");
        let root_escaped = root.to_string_lossy().replace('\\', "\\\\");
        std::fs::write(
            &config_path,
            format!(r#"{{"root_path": "{}"}}"#, root_escaped),
        )
        .unwrap();

        let session = crate::Session::new_with(&config_path).unwrap();
        let manifest = crate::package::Manifest::from_json(
            "test-pkg",
            r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
        )
        .unwrap();
        let pkg = Package::from("test-pkg", "test-bucket", manifest);
        let working_dir = root.join("apps").join("test-pkg").join("1.0.0");

        (session, pkg, working_dir, guard)
    }

    /// Test that $dir is expanded to the working directory path.
    #[test]
    fn test_expand_dir_var() {
        let (session, pkg, working_dir, _tmp) = setup_expand_vars_test("dir");
        let args = vec!["/DIR=\"$dir\""];
        let expanded = expand_installer_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded.len(), 1);
        assert_eq!(
            expanded[0],
            format!("/DIR=\"{}\"", working_dir.to_string_lossy())
        );
    }

    /// Test that $scoopdir is expanded to the Scoop root path.
    #[test]
    fn test_expand_scoopdir_var() {
        let (session, pkg, working_dir, tmp) = setup_expand_vars_test("scoopdir");
        let root = &tmp.0;
        let args = vec!["$scoopdir"];
        let expanded = expand_installer_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0], root.to_string_lossy().to_string());
    }

    /// Test that $persist_dir is expanded correctly.
    #[test]
    fn test_expand_persist_dir_var() {
        let (session, pkg, working_dir, tmp) = setup_expand_vars_test("persist");
        let root = &tmp.0;
        let expected = root.join("persist").join("test-pkg");
        let args = vec!["$persist_dir"];
        let expanded = expand_installer_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0], expected.to_string_lossy().to_string());
    }

    /// Test that $version, $app, and $bucket are expanded.
    #[test]
    fn test_expand_identity_vars() {
        let (session, pkg, working_dir, _tmp) = setup_expand_vars_test("identity");
        let args = vec!["$version", "$app", "$bucket"];
        let expanded = expand_installer_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded[0], "1.0.0");
        assert_eq!(expanded[1], "test-pkg");
        assert_eq!(expanded[2], "test-bucket");
    }

    /// Test that $cmd is expanded to "install" or "uninstall" accordingly.
    #[test]
    fn test_expand_cmd_var() {
        let (session, pkg, working_dir, _tmp) = setup_expand_vars_test("cmd");
        let args = vec!["$cmd"];
        let expanded_install =
            expand_installer_vars(&args, &session, &pkg, &working_dir, "install");
        let expanded_uninstall =
            expand_installer_vars(&args, &session, &pkg, &working_dir, "uninstall");
        assert_eq!(expanded_install[0], "install");
        assert_eq!(expanded_uninstall[0], "uninstall");
    }

    /// Test that all variables together in a realistic installer arg string are expanded.
    #[test]
    fn test_expand_all_vars_in_args() {
        let (session, pkg, working_dir, tmp) = setup_expand_vars_test("all_vars");
        let root = &tmp.0;
        let args = vec![
            "/VERYSILENT",
            "/DIR=\"$dir\"",
            "/D=\"$persist_dir\"",
            "/SCOOP=\"$scoopdir\"",
        ];
        let expanded = expand_installer_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded.len(), 4);
        assert_eq!(expanded[0], "/VERYSILENT");
        assert_eq!(
            expanded[1],
            format!("/DIR=\"{}\"", working_dir.to_string_lossy())
        );
        assert_eq!(
            expanded[2],
            format!(
                "/D=\"{}\"",
                root.join("persist").join("test-pkg").to_string_lossy()
            )
        );
        assert_eq!(
            expanded[3],
            format!("/SCOOP=\"{}\"", root.to_string_lossy())
        );
    }

    /// Test that $original_dir produces the same value as $dir.
    #[test]
    fn test_expand_original_dir_var() {
        let (session, pkg, working_dir, _tmp) = setup_expand_vars_test("original_dir");
        let args = vec!["$original_dir"];
        let expanded = expand_installer_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded[0], working_dir.to_string_lossy().to_string());
    }

    /// Test that $architecture is expanded (not empty, one of the known values).
    #[test]
    fn test_expand_architecture_var() {
        let (session, pkg, working_dir, _tmp) = setup_expand_vars_test("arch");
        let args = vec!["$architecture"];
        let expanded = expand_installer_vars(&args, &session, &pkg, &working_dir, "install");
        assert!(
            expanded[0] == "64bit" || expanded[0] == "32bit" || expanded[0] == "arm64",
            "expected 64bit/32bit/arm64, got {}",
            expanded[0]
        );
    }

    /// Test that variable names inside longer words do NOT get replaced (false positive).
    #[test]
    fn test_expand_no_false_positive() {
        let (session, pkg, working_dir, _tmp) = setup_expand_vars_test("false_positive");
        // "directory" contains "dir" but should not be replaced
        let args = vec!["directory", "scoopdirectory"];
        let expanded = expand_installer_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded[0], "directory");
        assert_eq!(expanded[1], "scoopdirectory");
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
}
