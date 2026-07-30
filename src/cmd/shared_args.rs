//! Shared argument groups for use across multiple commands.
//!
//! These `#[derive(clap::Args)]` structs standardize commonly-repeated flag
//! definitions. Use them via `#[clap(flatten)]` in any command's `Args`:
//!
//! ```ignore
//! #[derive(Parser)]
//! pub struct Args {
//!     #[clap(flatten)]
//!     pub sync: SyncArgs,
//!     // command-specific flags follow
//! }
//! ```

use libscoop::{QueryOption, Session, SyncOption};

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
pub trait Cmd {
    /// The clap argument type for this command.
    type Args: clap::Args;

    /// Execute the command with parsed arguments and session.
    fn execute(args: Self::Args, session: &libscoop::Session) -> crate::Result<()>;
}
