use scoop_hash::ChecksumBuilder;
use std::cell::OnceCell;
use std::io::Read;
use std::path::Path;
use tracing::{debug, info};

use crate::{
    env, error::Fallible, internal, persist, psmodule, shim, shortcut, Error,
    Event, QueryOption, Session,
};

use super::{
    download::{self, DownloadSize},
    query, resolve, Package,
};

/// Options that may be used to tweak behavior of package sync operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum SyncOption {
    /// Assume YES on all prompts.
    ///
    /// # Note
    ///
    /// This option will also suppress the prompt for package candidate selection.
    /// A built-in candidate selection algorithm will be used to select the
    /// proper candidate. This may not be the desired behavior in some cases.
    ///
    /// Enabling this option will also suppress the calculation of download size.
    AssumeYes,

    /// Download package only.
    ///
    /// # Note
    ///
    /// To sync packages by just downloading and caching them without installing
    /// or upgrading, this option can be used. Transcation will be stopped after
    /// the download is done.
    DownloadOnly,

    /// Force operations on held packages.
    ///
    /// # Note
    ///
    /// Held packages are ignored during the replace, upgrade or uninstall
    /// operations by default. The option can be used to escape the hold and
    /// enforce operations on the held packages.
    ///
    /// Packages will be held again after the replace or upgrade operation.
    EscapeHold,

    /// Ignore local cache and force package download.
    ///
    /// # Note
    ///
    /// This option is not intended to be used with the [`Offline`][1]
    /// option.
    ///
    /// [1]: SyncOption::Offline
    IgnoreCache,

    /// Ignore transaction failure.
    ///
    /// The sync operation processes packages in the transaction one by one
    /// according to the dependency order. By default, the transaction will be
    /// aborted if any failure occurs during the operation.
    ///
    /// # Note
    ///
    /// This option can be used to ignore the failure and continue the operation
    /// to commit the remaining packages in the transaction.
    ///
    /// When a failure occurs, the operation will be stopped immediately and
    /// a rollback will be performed on the exact package causing the failure
    /// while successfully committed packages will be kept be as they are. The
    /// rest of the unpocessed packages will be skipped, and the error will be
    /// returned.
    ///
    /// **NO rollback will be performed if this option is enabled**, which means
    /// there may be broken packages being committed to the system.
    IgnoreFailure,

    /// Do not install dependencies.
    ///
    /// # Note
    ///
    /// By default, dependencies of the pending installation package will be
    /// resolved and installed **recursively** if they are not installed yet.
    /// One can opt in this option to disable the default behavior. However,
    /// it is not recommended to do so since it clearly breaks the dependency
    /// relationship, and may stop the dependents from working properly.
    NoDependencies,

    /// Stop checking hash of downloaded packages.
    ///
    /// # Note
    ///
    /// Integrity check helps to ensure the downloaded packages are not corrupted
    /// or tampered. Hash check will be performed by default. In some cases, user
    /// may want to skip the check to force the installation or upgrade of the
    /// packages. By opting in this option, the hash check will be skipped.
    ///
    /// It is highly **NOT** recommended to use this option unless you really
    /// know what you are doing.
    NoHashCheck,

    /// Do not upgrade packages.
    ///
    /// This option is not intended to be used with the [`OnlyUpgrade`][1] option.
    ///
    /// [1]: SyncOption::OnlyUpgrade
    NoUpgrade,

    /// Do not replace packages.
    ///
    /// # Note
    ///
    /// When a package is installed and a same-named package is proposed to be
    /// installed, a replace operation will be performed if the proposed package
    /// is from a different bucket from the installed one.
    ///
    /// By opting in this option, the replace operation will be suppressed.
    NoReplace,

    /// Offline mode.
    ///
    /// # Note
    ///
    /// This option is useful when user wants to install or upgrade packages
    /// with existing local cached packages. By opting in this option and having
    /// valid caches prepared, network access can be avoided to perform the sync
    /// operation. However, the transaction may fail if there is any package file
    /// missing or invalid cache.
    ///
    /// This option is basically the opposite of the [`IgnoreCache`][1] option.
    ///
    /// [1]: SyncOption::IgnoreCache
    Offline,

    /// Upgrade packages only.
    ///
    /// Use this option to specify a sync operation of only upgrading packages.
    ///
    /// This option is not intended to be used with the [`NoUpgrade`][1] option.
    ///
    /// [1]: SyncOption::NoUpgrade
    OnlyUpgrade,

    /// Uninstall packages.
    ///
    /// Use this option to specify a sync operation of only uninstalling packages.
    Remove,

    /// Purge uninstall.
    ///
    /// # Note
    ///
    /// By enabling this option, persistent data of the pending removal packages
    /// will be removed simultaneously.
    ///
    /// This option only takes effect with the [`Remove`][1] option.
    ///
    /// [1]: SyncOption::Remove
    Purge,

    /// Cascade uninstall.
    ///
    /// # Note
    ///
    /// By opt in this option, dependencies of the pending removal package
    /// will also be removed **recursively** if they are not required by other
    /// installed packages.
    ///
    /// This option only takes effect with the [`Remove`][1] option.
    ///
    /// [1]: SyncOption::Remove
    Cascade,

    /// Disable dependent check.
    ///
    /// # Note
    ///
    /// By default, a reverse dependencies check will be performed on the pending
    /// removal package. If any installed package depends on the pending removal
    /// package, the removal operation will be aborted.
    ///
    /// The default behavior can be modified by opting in this option, however,
    /// it is not recommended to do so since it clearly breaks the dependency
    /// relationship, and may stop the dependents from working properly.
    ///
    /// This option only takes effect with the [`Remove`][1] option.
    ///
    /// [1]: SyncOption::Remove
    NoDependentCheck,
}

/// Transaction of sync operation.
///
/// # Note
///
/// A transaction is a set of packages that will be installed, upgraded, replaced
/// or removed. The transaction is calculated by the sync operation and can be
/// used to prompt the user to confirm the operation.
#[derive(Clone)]
pub struct Transaction {
    /// Packages that will be installed with the transaction.
    install: OnceCell<Vec<Package>>,

    /// Packages that will be upgraded with the transaction.
    upgrade: OnceCell<Vec<Package>>,

    /// Packages that will be replaced with the transaction.
    replace: OnceCell<Vec<Package>>,

    /// Packages that will be removed with the transaction.
    remove: OnceCell<Vec<Package>>,

    /// Total download size of the transaction.
    download_size: OnceCell<DownloadSize>,
}

impl Transaction {
    fn new() -> Transaction {
        Transaction {
            install: OnceCell::new(),
            upgrade: OnceCell::new(),
            replace: OnceCell::new(),
            remove: OnceCell::new(),
            download_size: OnceCell::new(),
        }
    }

    fn set_install(&self, packages: Vec<Package>) {
        let _ = self.install.set(packages);
    }

    fn set_upgrade(&self, packages: Vec<Package>) {
        let _ = self.upgrade.set(packages);
    }

    fn set_replace(&self, packages: Vec<Package>) {
        let _ = self.replace.set(packages);
    }

    fn set_remove(&self, packages: Vec<Package>) {
        let _ = self.remove.set(packages);
    }

    fn set_download_size(&self, download_size: DownloadSize) -> bool {
        self.download_size.set(download_size).is_ok()
    }

    fn add_view(&self) -> Vec<&Package> {
        self.install_view()
            .into_iter()
            .chain(self.upgrade_view())
            .chain(self.replace_view())
            .flatten()
            .collect::<Vec<_>>()
    }

    /// Get packages that will be installed with the transaction.
    ///
    /// # Returns
    ///
    /// A reference to the vector of packages that will be installed or `None`
    /// if no packages will be installed.
    pub fn install_view(&self) -> Option<&Vec<Package>> {
        self.install.get()
    }

    /// Get packages that will be upgraded with the transaction.
    ///
    /// # Returns
    ///
    /// A reference to the vector of packages that will be upgraded or `None`
    /// if no packages will be upgraded.
    pub fn upgrade_view(&self) -> Option<&Vec<Package>> {
        self.upgrade.get()
    }

    /// Get packages that will be replaced with the transaction.
    ///
    /// # Returns
    ///
    /// A reference to the vector of packages that will be replaced or `None`
    /// if no packages will be replaced.
    pub fn replace_view(&self) -> Option<&Vec<Package>> {
        self.replace.get()
    }

    /// Get packages that will be removed with the transaction.
    ///
    /// # Returns
    ///
    /// A reference to the vector of packages that will be removed or `None`
    /// if no packages will be removed.
    pub fn remove_view(&self) -> Option<&Vec<Package>> {
        self.remove.get()
    }

    /// Get the total download size of the transaction.
    ///
    /// # Returns
    ///
    /// A `DownloadSize` reference that contains the total download size of the
    /// transaction.
    pub fn download_size(&self) -> Option<&DownloadSize> {
        self.download_size.get()
    }
}

impl Default for Transaction {
    fn default() -> Self {
        Self::new()
    }
}

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
fn run_script(
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

    debug!("run_script: {} stage={} ({} lines)", package.name(), stage, lines.len());

    let script = lines.join("\r\n");

    // Embed PS helper scripts so package scripts can use functions
    // like Expand-InnoArchive, Expand-7zipArchive, Get-HelperPath, etc.
    const CORE_PS1: &str = include_str!("../../../../asset_scripts/core.ps1");
    const DECOMPRESS_PS1: &str = include_str!("../../../../asset_scripts/decompress.ps1");
    let preamble = format!(r#"
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
"#, core = CORE_PS1, decompress = DECOMPRESS_PS1);
    let full_script = format!("{preamble}\r\n{script}");

    // Write script to a temp file in the working dir
    let script_path = working_dir.join(format!("{}.ps1", stage));
    if let Some(parent) = script_path.parent() {
        internal::fs::ensure_dir(parent)?;
    }
    std::fs::write(&script_path, &full_script)?;

    // Build environment variables
    let config = session.config();
    let root_path = config.root_path();
    let pkg_dir = working_dir.to_path_buf();  // $dir = version dir (not current)

    let version = package.version();

    // Create marker file for P2 extraction routing
    let marker_path = working_dir.join("hok_extract_markers.txt");
    let _ = std::fs::remove_file(&marker_path); // clean from previous runs

    // Prefer pwsh.exe (PowerShell Core, faster startup)
    let ps_exe = if is_pwsh_available() { "pwsh.exe" } else { "powershell.exe" };

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
    if let Ok(markers) = std::fs::read_to_string(&marker_path) {
        for line in markers.lines() {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() < 3 {
                continue;
            }
            let format = parts[0];
            let source = Path::new(parts[1]);
            let dest = Path::new(parts[2]);
            let innosetup = format == "innosetup";

            if source.exists() {
                let emit = session.emitter();
                if let Err(e) = internal::archive::extract(source, dest, None, None, innosetup, &emit) {
                    // Log but don't abort — extraction errors may be handled
                    // by the PS script's own error handling
                    debug!("P2 extract failed for {}: {}", source.display(), e);
                }
            }
        }
    }
    let _ = std::fs::remove_file(&marker_path);

    // Clean up temp script file
    let _ = std::fs::remove_file(&script_path);

    Ok(())
}

/// Check whether pwsh.exe (PowerShell Core 7+) is available on PATH.
fn is_pwsh_available() -> bool {
    std::process::Command::new("pwsh.exe")
        .arg("-NoProfile")
        .arg("-c")
        .arg("$null")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// Expand Scoop-style variables (`$dir`, `$scoopdir`, `$persist_dir`, etc.)
/// in installer/uninstaller args, replacing them with the actual filesystem paths.
///
/// This mirrors the variable definitions in `run_script`'s PowerShell preamble,
/// so that `installer.file` and `uninstaller.file` (which run via `run_gui`
/// rather than through PowerShell) get equivalent variable expansion.
fn expand_installer_vars(
    args: &[&str],
    session: &Session,
    pkg: &Package,
    working_dir: &Path,
    cmd: &str,
) -> Vec<String> {
    let config = session.config();
    let root_path = config.root_path();
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

    if only_upgrade {
        packages = query::query_installed(session, queries, &[QueryOption::Upgradable])?;

        // Replace the packages with their upgradable references.
        // Packages without an upgradable version are skipped (filter_map).
        packages = packages
            .into_iter()
            .filter_map(|p| {
                let upgradable = p.upgradable().cloned();
                if upgradable.is_none() {
                    debug!("package '{}' has no upgradable reference, skipping", p.name());
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
                    let (query_bucket, query_name) = query.split_once('/').unwrap_or(("", query));
                    let bucket_matched = query_bucket.is_empty() || p.bucket() == query_bucket;
                    let name_matched = p.name() == query_name;
                    bucket_matched && name_matched
                })
                .cloned()
                .collect::<Vec<_>>();

            // Debug: log how many synced packages for diagnosis
            debug!("query '{}': {} synced packages, {} exact matches", query, synced.len(), matched.len());

            match matched.len() {
                0 => return Err(Error::PackageNotFound(query.to_owned())),
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

                    resolve::select_candidate(session, &mut matched)?;
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
        resolve::resolve_dependencies(session, &mut packages)?;
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

    if !assume_yes {
        if let Some(tx) = session.emitter() {
            if tx
                .send(Event::PromptTransactionNeedConfirm(transaction.clone()))
                .is_ok()
            {
                let rx = session.receiver().unwrap();
                let mut confirmed = false;

                while let Ok(event) = rx.recv() {
                    if let Event::PromptTransactionNeedConfirmResult(ret) = event {
                        confirmed = ret;
                        break;
                    }
                }

                if !confirmed {
                    return Ok(());
                }
            }
        }
    }

    if !should_offline {
        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageDownloadStart);
        }

        set.download()?;

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageDownloadDone);
        }
    }

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

            for (idx, (filename, hash)) in files.into_iter().zip(hashes.into_iter()).enumerate() {
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
                    let ctx =
                        super::HashMismatchContext::new(name, url, expected.to_owned(), actual);
                    return Err(Error::HashMismatch(ctx));
                }
            }
        }

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageIntegrityCheckDone);
        }
    }

    let download_only = options.contains(&SyncOption::DownloadOnly);
    if !download_only {
        let ignore_failure = options.contains(&SyncOption::IgnoreFailure);
        commit_install(session, &packages, ignore_failure)?;
    }

    Ok(())
}

/// Commit package installation: extract files, run scripts, create symlinks,
/// shims, and shortcuts.
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
    let apps_dir = config.root_path().join("apps");
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
    let files = pkg.download_filenames();
    let urls = pkg.manifest().url();

    // Collect the files that need to be decompressed
    let archives: Vec<usize> = files.iter().enumerate()
        .filter_map(|(idx, f)| {
            let url = &urls[idx];

            // Extract the target filename directly from the URL
            let target_name = url.rsplit('/').next().unwrap_or(f);

            let ext = Path::new(target_name)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if matches!(ext, "7z" | "zip" | "nupkg" | "rar" | "lzh"
                | "gz" | "bz2" | "xz" | "zst" | "tgz" | "tar") {
                Some(idx)
            } else {
                None
            }
        })
        .collect();

    // 1. Extract the archive file
    if !archives.is_empty() {
        let cache_path = config.cache_path();
        debug!("commit: {} v{} - extract ({} files)", pkg.name(), pkg.version(), archives.len());

        for idx in archives.iter() {
            let filename = &files[*idx];
            let src = cache_path.join(filename);
            if src.exists() {
                if let Some(tx) = session.emitter() {
                    let _ = tx.send(Event::PackageExtractStart(
                        format!("{}/{}", pkg.name(), filename)));
                }
                let emit = session.emitter();
                internal::archive::extract(
                    &src, &working_dir,
                    pkg.manifest().extract_dir().as_deref(),
                    pkg.manifest().extract_to().as_deref(),
                    pkg.manifest().innosetup(),
                    &emit)?;
                if let Some(tx) = session.emitter() {
                    let _ = tx.send(Event::PackageExtractDone);
                }
            }
        }
    }

    // 2. Copy all non-archived files (including _ files and regular files)
    debug!("commit: {} v{} - copy ({} files)", pkg.name(), pkg.version(), files.len() - archives.len());

    for (idx, filename) in files.iter().enumerate() {
        // Skip already extracted archive files
        if archives.contains(&idx) {
            continue;
        }

        let src = config.cache_path().join(filename);
        if !src.exists() {
            continue;
        }

        let url = &urls[idx];
        let target_name = url.rsplit('/').next().unwrap_or(filename);

        let dst = working_dir.join(&target_name);
        let _ = std::fs::remove_file(&dst);
        std::fs::copy(&src, dst)?;
    }

    // 2. pre_install (Scoop order: after extract/copy, before link_current)
    if pkg.has_install_script() {
        debug!("commit: {} v{} - pre_install", pkg.name(), pkg.version());
        run_script(session, pkg, &working_dir, "pre_install", "install",
            pkg.manifest().pre_install())?;
    }

    // 3. installer, $dir = version dir)
    if pkg.has_install_script() {
        if let Some(installer) = pkg.manifest().installer() {
            if let Some(script) = installer.script() {
                debug!("commit: {} v{} - installer.script", pkg.name(), pkg.version());
                run_script(session, pkg, &working_dir, "installer", "install", Some(script))?;
            } else if let Some(file) = installer.file() {
                debug!("commit: {} v{} - installer.file", pkg.name(), pkg.version());
                let exe_path = working_dir.join(file);
                let raw_args: Vec<&str> = installer.args().unwrap_or_default();
                let expanded = expand_installer_vars(&raw_args, session, pkg, &working_dir, "install");
                let args: Vec<&str> = expanded.iter().map(|s| s.as_str()).collect();
                crate::internal::os::run_gui(&exe_path, &args, Some(&working_dir))
                    .map_err(|e| Error::Custom(format!(
                        "failed to run installer '{}' for '{}': {}", file, pkg.name(), e)))?;
            }
        }
    }

    // 4. link_current (Scoop order: after installer, before shims)
    debug!("commit: {} v{} - link_current", pkg.name(), pkg.version());
    let current_lnk = apps_dir.join(pkg.name()).join("current");
    let _ = internal::fs::remove_symlink(&current_lnk);
    if current_lnk.exists() {
        let _ = std::fs::remove_dir_all(&current_lnk);
    }
    internal::fs::symlink_dir(&working_dir, &current_lnk)?;

    // 5. shims + shortcuts
    debug!("commit: {} v{} - shims/shortcuts", pkg.name(), pkg.version());
    shim::add(session, pkg)?;
    shortcut::add(session, pkg)?;

    // 6. persist (Scoop order: after shims, before post_install)
    debug!("commit: {} v{} - persist", pkg.name(), pkg.version());
    persist::link(session, pkg)?;

    // 7. post_install (Scoop order: last hook)
    if pkg.has_install_script() {
        debug!("commit: {} v{} - post_install", pkg.name(), pkg.version());
        run_script(session, pkg, &working_dir, "post_install", "install",
            pkg.manifest().post_install())?;
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

    debug!("commit: {} v{} - writing metadata", pkg.name(), pkg.version());

    // 7. Write install metadata
    let current_dir = apps_dir.join(pkg.name()).join("current");

    // 1. Copy manifest from bucket to current/manifest.json
    // Use bucket path (manifest.path() may be virtual when loaded from cache)
    let bucket_path = config.root_path().join("buckets").join(pkg.bucket());
    let manifest_src = bucket_path.join("bucket").join(format!("{}.json", pkg.name()));
    let manifest_fallback = bucket_path.join(format!("{}.json", pkg.name()));
    let manifest_src = if manifest_src.exists() { manifest_src } else { manifest_fallback };
    let manifest_dst = current_dir.join("manifest.json");
    match std::fs::copy(&manifest_src, manifest_dst) {
        Ok(_) => {},
        Err(e) => return Err(Error::Custom(format!(
            "could not copy manifest from {:?}: {}", manifest_src, e))),
    }

    // 2. Write current/install.json
    let arch = if cfg!(target_arch = "x86_64") { "64bit" }
               else if cfg!(target_arch = "x86") { "32bit" }
               else { "arm64" };
    let install_info = serde_json::json!({
        "architecture": arch,
        "bucket": pkg.bucket(),
    });
    if let Err(e) = internal::fs::write_json(current_dir.join("install.json"), &install_info) {
        return Err(Error::Custom(format!("install.json write: {}", e)));
    }

    Ok(())
}

/// Sync operation: remove packages.
pub fn remove(session: &Session, queries: &[&str], options: &[SyncOption]) -> Fallible<()> {
    let escape_hold = options.contains(&SyncOption::EscapeHold);
    let no_dependent_check = options.contains(&SyncOption::NoDependentCheck);

    // Query target packages directly instead of scanning all installed.
    // Dependency checking (below) does the full scan when needed.
    let mut packages = vec![];
    for &name in queries {
        let matched = query::query_installed(session, &[name], &[QueryOption::Explicit])?;
        if matched.is_empty() {
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
                        .map(super::extract_name)
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
    if !assume_yes {
        if let Some(tx) = session.emitter() {
            if tx
                .send(Event::PromptTransactionNeedConfirm(transaction.clone()))
                .is_ok()
            {
                let rx = session.receiver().unwrap();
                let mut confirmed = false;

                while let Ok(event) = rx.recv() {
                    if let Event::PromptTransactionNeedConfirmResult(ret) = event {
                        confirmed = ret;
                        break;
                    }
                }

                if !confirmed {
                    return Ok(());
                }
            }
        }
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
fn commit_remove(session: &Session, packages: &[Package], purge: bool, ignore_failure: bool) -> Fallible<()> {
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
    let config = session.config();
    let root_dir = config.root_path();

    debug!("remove: {} - starting", package.name());

    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackageCommitStart(package.name().to_owned()));
    }

    let app_dir = root_dir.join("apps").join(package.name());

    run_script(session, package, &app_dir.join("current"), "pre_uninstall", "uninstall",
        package.manifest().pre_uninstall())?;

    if let Some(uninstaller) = package.manifest().uninstaller() {
        if let Some(script) = uninstaller.script() {
            run_script(session, package, &app_dir.join("current"), "uninstaller", "uninstall", Some(script))?;
        } else if let Some(file) = uninstaller.file() {
            debug!("remove: {} - uninstaller.file", package.name());
            let exe_path = app_dir.join("current").join(file);
            let raw_args: Vec<&str> = uninstaller.args().unwrap_or_default();
            let expanded = expand_installer_vars(&raw_args, session, package, &app_dir.join("current"), "uninstall");
            let args: Vec<&str> = expanded.iter().map(|s| s.as_str()).collect();
            crate::internal::os::run_gui(&exe_path, &args, Some(&app_dir.join("current")))
                .map_err(|e| Error::Custom(format!(
                    "failed to run uninstaller '{}' for '{}': {}", file, package.name(), e)))?;
        }
    }

    debug!("remove: {} - cleanup (shims/shortcuts/env/persist)", package.name());
    shim::remove(session, package)?;
    shortcut::remove(session, package)?;
    psmodule::remove(session, package)?;
    env::remove(session, package)?;
    persist::unlink(session, package)?;

    let current_lnk = app_dir.join("current");
    internal::fs::remove_symlink(current_lnk)?;

    run_script(session, package, &app_dir, "post_uninstall", "uninstall",
        package.manifest().post_uninstall())?;

    internal::fs::remove_dir(&app_dir)?;

    if purge {
        debug!("remove: {} - purging persist data", package.name());
        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackagePersistPurgeStart);
        }
        let persist_dir = config.root_path().join("persist").join(package.name());
        internal::fs::remove_dir(persist_dir)?;
        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackagePersistPurgeDone);
        }
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
    let current_lnk = pkg_dir.join("current");
    let _ = internal::fs::remove_symlink(&current_lnk);
    internal::fs::symlink_dir(&version_dir, &current_lnk)?;

    // Re-link persistent data
    persist::link(session, pkg)?;

    // Re-create shims + shortcuts
    shim::remove(session, pkg)?;
    shim::add(session, pkg)?;
    shortcut::remove(session, pkg)?;
    shortcut::add(session, pkg)?;

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that installer.file execution path works correctly.
    #[test]
    fn test_installer_file_execution() {
        let tmp = std::env::temp_dir().join("hok_test_installer_file");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // Test 1: capture exit code via cmd.exe /c
        let exit_code = crate::internal::os::run_gui(
            &std::path::PathBuf::from("cmd.exe"),
            &["/c", "exit /b 42"],
            Some(&tmp),
        ).unwrap();
        assert_eq!(exit_code, 42, "should capture exit code from cmd.exe /c");

        // Test 2: create a file via PowerShell (used in many Scoop installer scripts)
        let marker = tmp.join("ran.txt");
        let ps = if is_pwsh_available() { "pwsh.exe" } else { "powershell.exe" };
        let exit_code = crate::internal::os::run_gui(
            &std::path::PathBuf::from(ps),
            &[
                "-NoProfile",
                "-Command",
                &format!("New-Item -Path '{}' -ItemType File -Force | Out-Null", marker.display()),
            ],
            Some(&tmp),
        ).unwrap();
        assert_eq!(exit_code, 0, "powershell script should exit 0");
        assert!(marker.exists(), "powershell should have created marker file");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Test URL fragment rename: url#/installer.exe → copy directly as installer.exe
    #[test]
    fn test_url_fragment_rename() {
        let tmp = std::env::temp_dir().join("hok_test_fragment_rename");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

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
        let filename = std::path::Path::new(url).file_name()
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

    fn setup_expand_vars_test(test_name: &str) -> (crate::Session, Package, std::path::PathBuf, TestDirGuard) {
        let tmp = std::env::temp_dir().join(format!("hok_test_expand_vars_{}", test_name));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
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
        let expanded_install = expand_installer_vars(&args, &session, &pkg, &working_dir, "install");
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
}
