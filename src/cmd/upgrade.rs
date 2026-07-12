use clap::{ArgAction, Parser};
use libscoop::Session;

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
    /// Skip package integrity check
    #[arg(long, action = ArgAction::SetTrue)]
    no_hash_check: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let update_args = super::update::Args {
        package: args.package,
        ignore_failure: args.ignore_failure,
        offline: args.offline,
        assume_yes: args.assume_yes,
        escape_hold: args.escape_hold,
        no_hash_check: args.no_hash_check,
        force: false,
    };
    super::update::execute_upgrade(session, &update_args.package, &update_args)
}
