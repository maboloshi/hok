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
use std::path::Path;

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

    /// Ignore failures to ensure a complete transaction
    #[arg(short = 'f', long, action = clap::ArgAction::SetTrue)]
    pub ignore_failure: bool,

    /// Leverage cache and suppress network access
    #[arg(short = 'o', long, action = clap::ArgAction::SetTrue)]
    pub offline: bool,

    /// Skip package integrity check
    #[arg(short = 's', long, action = clap::ArgAction::SetTrue)]
    pub no_hash_check: bool,

    /// Do not install dependencies (may break packages)
    #[arg(short = 'I', long, action = clap::ArgAction::SetTrue)]
    pub independent: bool,

    /// Do not replace package(s)
    #[arg(short = 'R', long, action = clap::ArgAction::SetTrue)]
    pub no_replace: bool,

    /// Escape hold to allow changes on held package(s)
    #[arg(short = 'S', long, action = clap::ArgAction::SetTrue)]
    pub escape_hold: bool,

    /// Do not upgrade package(s)
    #[arg(short = 'U', long, action = clap::ArgAction::SetTrue)]
    pub no_upgrade: bool,

    /// Ignore cache and force download
    #[arg(short = 'D', long, action = clap::ArgAction::SetTrue)]
    pub ignore_cache: bool,

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
        if self.escape_hold {
            options.push(SyncOption::EscapeHold);
        }
        if self.ignore_failure || session.config().ignore_failures() {
            options.push(SyncOption::IgnoreFailure);
        }
        if self.ignore_cache {
            options.push(SyncOption::IgnoreCache);
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
        if self.no_hash_check {
            options.push(SyncOption::NoHashCheck);
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
        return Err(anyhow::anyhow!(rust_i18n::t!(
            "cmd.admin_rights_required",
            verb = verb
        )));
    }
    Ok(())
}

/// Validate that `dir` is an existing directory, printing a localized error
/// when it is not.
///
/// Shared by the bucket-scan commands (checkhashes / checkurls / checkver /
/// formatjson / missing_checkver / auto-pr): each mirrors Scoop's
/// `if (-not (Test-Path $dir -PathType Container))` guard and returns
/// normally after printing the error. Returns `true` when the directory is
/// valid; callers should `return Ok(())` on `false`.
pub(crate) fn validate_dir(dir: &Path) -> bool {
    if dir.is_dir() {
        true
    } else {
        crate::output::err(rust_i18n::t!("cmd.dir_not_found", path = dir.display()));
        false
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
    #[arg(short = 'D', long, action = clap::ArgAction::SetTrue)]
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

/// Standard interface for all hok commands.
///
/// Each command module defines its own `Args` struct
/// (via `#[derive(clap::Parser)]`) and implements this trait.
/// The trait enables uniform dispatch and future auto-registration.
///
/// Note: dispatch currently calls each module's `execute()` function
/// directly (see `cmd/mod.rs`); the trait is reserved for a future
/// uniform dispatch / auto-registration, hence the allow.
#[allow(dead_code)]
pub trait Cmd {
    /// The clap argument type for this command.
    type Args: clap::Args;

    /// Execute the command with parsed arguments and session.
    fn execute(args: Self::Args, session: &libscoop::Session) -> crate::Result<()>;
}
