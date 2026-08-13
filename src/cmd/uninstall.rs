//! Uninstall package(s).

use clap::Parser;
use libscoop::{package, Session, SyncOption};

use crate::cmd::shared_args::ensure_global;
use crate::Result;

/// Uninstall package(s)
#[derive(Debug, Parser)]
pub struct Args {
    /// The package(s) to uninstall
    #[arg(required = true)]
    package: Vec<String>,
    /// Ignore failures to ensure a complete transaction
    #[arg(short = 'f', long, action = clap::ArgAction::SetTrue)]
    ignore_failure: bool,
    /// Escape hold to allow changes on held package(s)
    #[arg(short = 'S', long, action = clap::ArgAction::SetTrue)]
    escape_hold: bool,
    /// Assume yes to all prompts and run non-interactively
    #[arg(short = 'y', long, action = clap::ArgAction::SetTrue)]
    assume_yes: bool,
    /// Skip package integrity check (Scoop: --skip-hash-check)
    #[arg(short = 's', long, visible_alias = "skip-hash-check", action = clap::ArgAction::SetTrue)]
    no_hash_check: bool,
    /// Uninstall a globally installed app (from $SCOOP_GLOBAL)
    #[arg(short = 'g', long, action = clap::ArgAction::SetTrue)]
    global: bool,
    /// Remove all persistent data
    #[arg(short = 'p', long, action = clap::ArgAction::SetTrue)]
    purge: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    ensure_global(session, args.global, "uninstall")?;

    let mut options = vec![SyncOption::Remove];

    if args.purge {
        options.push(SyncOption::Purge);
    }

    if args.escape_hold {
        options.push(SyncOption::EscapeHold);
    }

    if args.assume_yes {
        options.push(SyncOption::AssumeYes);
    }

    if args.ignore_failure || session.config().ignore_failures() {
        options.push(SyncOption::IgnoreFailure);
    }

    if args.no_hash_check {
        options.push(SyncOption::NoHashCheck);
    }

    let queries = args.package.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    let handle = crate::eventloop::run_event_loop_default(session);

    package::sync::sync(session, queries, options)?;
    handle.join().unwrap();

    Ok(())
}
