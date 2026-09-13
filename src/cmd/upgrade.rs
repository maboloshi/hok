//! Upgrade installed package(s).

use clap::{ArgAction, Parser};
use libscoop::Session;

use crate::Result;
use crate::cmd::shared_args::{SyncArgs, SyncFlags};

/// Upgrade installed package(s)
#[derive(Debug, Parser)]
pub struct Args {
    /// The package(s) to be upgraded (default: all except held)
    #[arg(action = ArgAction::Append)]
    package: Vec<String>,
    /// Ignore failures to ensure a complete transaction (persistent setting:
    /// 'hok config ignore_failures true')
    #[arg(long, action = ArgAction::SetTrue)]
    ignore_failures: bool,
    /// Leverage cache and suppress network access
    #[arg(short = 'o', long, action = ArgAction::SetTrue)]
    offline: bool,
    /// Assume yes to all prompts and run non-interactively
    #[arg(short = 'y', long, action = ArgAction::SetTrue)]
    assume_yes: bool,
    /// Escape hold to allow to upgrade held package(s)
    #[arg(short = 'S', long, action = ArgAction::SetTrue)]
    escape_hold: bool,
    /// Skip package integrity check
    #[arg(short = 's', long, action = ArgAction::SetTrue)]
    skip_hash_check: bool,
    /// Force upgrade: reinstall apps that are already at the requested version
    /// (Scoop's `-f/--force`)
    #[arg(short = 'f', long, action = ArgAction::SetTrue)]
    force: bool,
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
            ignore_failures: self.ignore_failures,
            offline: self.offline,
            skip_hash_check: self.skip_hash_check,
            independent: false,
            no_replace: false,
            escape_hold: self.escape_hold,
            no_upgrade: false,
            no_cache: false,
            download_only: false,
        }
    }
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    // --all is a shorthand for upgrading all packages (like passing '*'),
    // matching `update --all` semantics.
    if args.all && args.package.is_empty() {
        return super::update::execute_upgrade(
            session,
            &[String::from("*")],
            &args.sync_args(),
            args.force,
        )
        .await;
    }

    super::update::execute_upgrade(session, &args.package, &args.sync_args(), args.force).await
}
