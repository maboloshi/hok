use clap::{ArgAction, Parser};
use crossterm::style::Stylize;
use libscoop::{operation, Session};

use crate::Result;

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
    let ignore_failure = true; // cleanup is best-effort
    let results = operation::package_cleanup(session, &args.app, ignore_failure)?;

    for (name, count) in &results {
        println!("  {}: {} {} removed", name.as_str().blue(), count, "old version(s)".yellow());
    }

    if results.is_empty() {
        println!("{}", "No old versions to clean up.".green());
    } else {
        println!("{}", "Everything is shiny now!".green());
    }

    if args.cache {
        operation::cache_remove(session, "*")?;
        println!("{}", "Cache cleaned.".green());
    }

    Ok(())
}
