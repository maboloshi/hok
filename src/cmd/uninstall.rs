use clap::Parser;
use libscoop::{operation, Session, SyncOption};

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
    /// Skip package integrity check
    #[arg(long, action = clap::ArgAction::SetTrue)]
    no_hash_check: bool,
    /// Uninstall a globally installed app (from $SCOOP_GLOBAL)
    #[arg(short = 'g', long, action = clap::ArgAction::SetTrue)]
    global: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let mut options = vec![SyncOption::Remove];

    if args.escape_hold {
        options.push(SyncOption::EscapeHold);
    }

    if args.ignore_failure {
        options.push(SyncOption::IgnoreFailure);
    }

    if args.no_hash_check {
        options.push(SyncOption::NoHashCheck);
    }

    session.set_global(args.global);
    let queries = args.package.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    let handle = crate::eventloop::run_event_loop(session, Default::default());

    operation::package_sync(session, queries, options)?;
    handle.join().unwrap();

    Ok(())
}