//! Upgrade installed package(s).

use clap::{ArgAction, Parser};
use libscoop::Session;

use crate::cmd::shared_args::{SyncArgs, SyncFlags};
use crate::Result;

/// Upgrade installed package(s)
#[derive(Debug, Parser)]
pub struct Args {
    /// The package(s) to be upgraded (default: all except held)
    #[arg(action = ArgAction::Append)]
    package: Vec<String>,
    /// Ignore failures to ensure a complete transaction
    #[arg(short = 'f', long, action = ArgAction::SetTrue)]
    ignore_failure: bool,
    /// Leverage cache and suppress network access
    #[arg(short = 'o', long, action = ArgAction::SetTrue)]
    offline: bool,
    /// Assume yes to all prompts and run non-interactively
    #[arg(short = 'y', long, action = ArgAction::SetTrue)]
    assume_yes: bool,
    /// Escape hold to allow to upgrade held package(s)
    #[arg(short = 'S', long, action = ArgAction::SetTrue)]
    escape_hold: bool,
    /// Skip package integrity check (Scoop: --skip-hash-check)
    #[arg(short = 's', long, visible_alias = "skip-hash-check", action = ArgAction::SetTrue)]
    no_hash_check: bool,
    /// Install globally (to $SCOOP_GLOBAL)
    #[arg(short = 'g', long, action = ArgAction::SetTrue)]
    global: bool,
    /// Update all installed packages (alternative to '*')
    #[arg(short = 'a', long, action = clap::ArgAction::SetTrue)]
    all: bool,
}

impl SyncFlags for Args {
    fn sync_args(&self) -> SyncArgs {
        SyncArgs {
            global: self.global,
            assume_yes: self.assume_yes,
            ignore_failure: self.ignore_failure,
            offline: self.offline,
            no_hash_check: self.no_hash_check,
            independent: false,
            no_replace: false,
            escape_hold: self.escape_hold,
            no_upgrade: false,
            ignore_cache: false,
            download_only: false,
        }
    }
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    // --all is a shorthand for upgrading all packages (like passing '*'),
    // matching `update --all` semantics.
    if args.all && args.package.is_empty() {
        return super::update::execute_upgrade(
            session,
            &[String::from("*")],
            &args.sync_args(),
            false,
        );
    }

    super::update::execute_upgrade(session, &args.package, &args.sync_args(), false)
}
