//! Shared argument groups for use across multiple commands.
//!
//! [`SyncArgs`] centralizes the flag→option conversion shared by the sync
//! commands (install / update / upgrade / reinstall). Each command declares
//! its own `Args` fields privately — matching Scoop's per-command flag sets —
//! and implements [`SyncFlags`] to project them into a [`SyncArgs`]:
//!
//! ```ignore
//! impl SyncFlags for Args {
//!     fn sync_args(&self) -> SyncArgs {
//!         SyncArgs { global: self.global, /* ... */ }
//!     }
//! }
//! ```
//!
//! [`QueryArgs`] is the same idea for query operations (search etc.).

use libscoop::{QueryOption, Session, SyncOption};
use std::path::{Path, PathBuf};

/// Shared arguments for sync operations (install, update, upgrade, reinstall, etc.).
///
/// Provides `.to_sync_options()` to convert flags to a `Vec<SyncOption>`.
#[derive(Clone, Debug, clap::Args)]
pub struct SyncArgs {
    /// Install/upgrade globally (to $SCOOP_GLOBAL)
    #[arg(short = 'g', long, action = clap::ArgAction::SetTrue)]
    pub global: bool,

    /// Assume yes to all prompts and run non-interactively
    #[arg(short = 'y', long, action = clap::ArgAction::SetTrue)]
    pub assume_yes: bool,

    /// Ignore failures to ensure a complete transaction (persistent setting:
    /// 'hok config ignore_failures true')
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub ignore_failures: bool,

    /// Leverage cache and suppress network access
    #[arg(short = 'o', long, action = clap::ArgAction::SetTrue)]
    pub offline: bool,

    /// Skip package integrity check
    #[arg(short = 's', long, action = clap::ArgAction::SetTrue)]
    pub skip_hash_check: bool,

    /// Don't install dependencies automatically
    #[arg(short = 'i', long, action = clap::ArgAction::SetTrue)]
    pub independent: bool,

    /// Do not replace package(s)
    #[arg(short = 'R', long, action = clap::ArgAction::SetTrue)]
    pub no_replace: bool,

    /// Do not upgrade package(s)
    #[arg(short = 'U', long, action = clap::ArgAction::SetTrue)]
    pub no_upgrade: bool,

    /// Don't use the download cache
    #[arg(short = 'k', long, action = clap::ArgAction::SetTrue)]
    pub no_cache: bool,

    /// Download package(s) without performing installation
    #[arg(short = 'd', long, action = clap::ArgAction::SetTrue)]
    pub download_only: bool,
}

impl SyncArgs {
    /// Convert shared sync flags to a `Vec<SyncOption>`.
    pub fn to_sync_options(&self, session: &Session) -> Vec<SyncOption> {
        let mut options = vec![];
        if self.assume_yes {
            options.push(SyncOption::AssumeYes);
        }
        if self.download_only {
            options.push(SyncOption::DownloadOnly);
        }
        if self.ignore_failures || session.config().ignore_failures() {
            options.push(SyncOption::IgnoreFailures);
        }
        if self.no_cache {
            options.push(SyncOption::NoCache);
        }
        if self.no_upgrade {
            options.push(SyncOption::NoUpgrade);
        }
        if self.no_replace {
            options.push(SyncOption::NoReplace);
        }
        if self.offline {
            options.push(SyncOption::Offline);
        }
        if self.independent {
            options.push(SyncOption::NoDependencies);
        }
        if self.skip_hash_check {
            options.push(SyncOption::SkipHashCheck);
        }
        options
    }
}

/// Apply the `--global` scope and verify admin rights when requested.
///
/// Mirrors Scoop's per-command `if ($global -and !(is_admin))` guard (see
/// scoop-install.ps1 / scoop-uninstall.ps1 / scoop-hold.ps1): sets the
/// session's global scope, then bails with a per-command message when a
/// global operation runs without elevation. `verb` fills the message
/// (e.g. "install", "uninstall", "hold").
pub(crate) fn ensure_global(session: &Session, global: bool, verb: &str) -> crate::Result<()> {
    session.set_global(global);
    if global && !session.is_admin() {
        return Err(anyhow::anyhow!(hok_i18n::t!(
            "cmd.admin_rights_required",
            verb = verb
        )));
    }
    Ok(())
}

/// Resolve the bucket-scan directory for a run that writes to disk.
///
/// `-d/--dir` defaults to `.` only for read-only runs. As soon as a run
/// rewrites manifests the target must be explicit, because the scan is a
/// recursive `*.json` walk (`libscoop::package::manifest_walker`) that
/// excludes only `package.json` and never checks whether a file looks like a
/// Scoop manifest: a bare `hok formatjson` in a source checkout rewrites
/// `tsconfig.json`, `package-lock.json`, everything under `target/`, and the
/// user's own `config.json`. Upstream enforces the same thing by declaring
/// `-Dir` `Mandatory = $true` (`bin/formatjson.ps1`, `bin/checkhashes.ps1`,
/// `bin/checkurls.ps1`, `bin/missing-checkver.ps1`) or by throwing
/// (`bin/checkver.ps1:85`); hok states the reason instead of only reporting a
/// missing parameter. `cmd` names the command in that message.
pub(crate) fn write_dir(dir: Option<PathBuf>, cmd: &str) -> crate::Result<PathBuf> {
    dir.ok_or_else(|| {
        anyhow::anyhow!(hok_i18n::t!("cmd.dir_required_for_write", cmd = cmd).to_string())
    })
}

/// Resolve the bucket-scan directory for a read-only run: `-d/--dir` defaults
/// to the current directory, exactly as before the write guard existed.
pub(crate) fn scan_dir(dir: Option<PathBuf>) -> PathBuf {
    dir.unwrap_or_else(|| PathBuf::from("."))
}

/// Validate that `dir` is an existing directory, printing a localized error
/// when it is not.
///
/// Shared by the bucket-scan commands (checkhashes / checkurls / checkver /
/// formatjson / missing_checkver / auto-pr): each mirrors Scoop's
/// `if (-not (Test-Path $dir -PathType Container))` guard and returns
/// normally after printing the error. Returns `true` when the directory is
/// valid; callers should `return Ok(())` on `false`.
pub(crate) fn validate_dir(dir: &Path) -> crate::Result<()> {
    if dir.is_dir() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(hok_i18n::t!(
            "cmd.dir_not_found",
            path = dir.display()
        )))
    }
}

/// Standard interface for commands that share the sync flag set.
///
/// Each sync command (install / update / upgrade / reinstall) declares its
/// own `Args` fields privately and implements this trait to project them into
/// a [`SyncArgs`]. The default `to_sync_options()` reuses the shared
/// conversion (including the config `ignore_failures` merge), so execute
/// bodies stay free of flag plumbing.
///
/// Commands may bake in command-specific defaults here (e.g. `reinstall`
/// always runs with `assume_yes`).
pub trait SyncFlags {
    /// Project this command's sync flags into a [`SyncArgs`].
    fn sync_args(&self) -> SyncArgs;

    /// Convert sync flags to a `Vec<SyncOption>`.
    fn to_sync_options(&self, session: &Session) -> Vec<SyncOption> {
        self.sync_args().to_sync_options(session)
    }
}

/// Shared arguments for query/search operations (list, search, info, depends, etc.).
///
/// Provides `.to_query_options()` to convert flags to a `Vec<QueryOption>`.
#[derive(Clone, Debug, clap::Args)]
pub struct QueryArgs {
    /// Turn regex off and use explicit matching
    #[arg(short = 'e', long, action = clap::ArgAction::SetTrue)]
    pub explicit: bool,

    /// Search through package binaries as well
    #[arg(short = 'B', long, action = clap::ArgAction::SetTrue)]
    pub with_binary: bool,

    /// Search through package descriptions as well
    #[arg(short = 'k', long, action = clap::ArgAction::SetTrue)]
    pub with_description: bool,
}

impl QueryArgs {
    /// Convert shared query flags to a `Vec<QueryOption>`.
    pub fn to_query_options(&self) -> Vec<QueryOption> {
        let mut options = vec![];
        if self.with_binary {
            options.push(QueryOption::Binary);
        }
        if self.with_description {
            options.push(QueryOption::Description);
        }
        if self.explicit {
            options.push(QueryOption::Explicit);
        }
        options
    }
}
