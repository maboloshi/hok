use clap::{ArgAction, Parser};
use libscoop::{operation, Session};

use crate::{output, Result};

/// Cleanup apps by removing old versions
#[derive(Debug, Parser)]
#[clap()]
pub struct Args {
    /// Given named app(s) to be cleaned up (all apps if empty)
    #[arg(action = ArgAction::Append)]
    app: Vec<String>,
    /// Remove download cache simultaneously
    #[arg(short = 'k', long, action = ArgAction::SetTrue)]
    cache: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let ignore_failure = session.config().ignore_failures();
    let results = operation::package_cleanup(session, &args.app, ignore_failure)?;

    for (name, count) in &results {
        output::named(name.as_str(), format!("{count} old version(s) removed"));
    }

    if results.is_empty() {
        output::info("No old versions to clean up.");
    } else {
        output::info("Everything is shiny now!");
    }

    if args.cache {
        operation::cache_remove(session, "*")?;
        output::info("Cache cleaned.");
    }

    Ok(())
}
