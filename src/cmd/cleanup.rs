use clap::{ArgAction, Parser};
use libscoop::{operation, Session};

use crate::{output, Result};

/// Cleanup apps by removing old versions
#[derive(Debug, Parser)]
#[clap()]
pub struct Args {
    /// Given named app(s) to be cleaned up (use '*' or --all to cleanup all apps)
    #[arg(action = ArgAction::Append)]
    app: Vec<String>,
    /// Cleanup all apps (alternative to '*')
    #[arg(short = 'a', long, action = ArgAction::SetTrue)]
    all: bool,
    /// Remove download cache simultaneously
    #[arg(short = 'k', long, action = ArgAction::SetTrue)]
    cache: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    // --all or '*' overrides any specific app names
    let apps: Vec<String> = if args.all || args.app.iter().any(|a| a == "*") {
        Vec::new() // empty means "all apps" in package_cleanup
    } else {
        args.app
    };
    // Cleanup is a maintenance operation; individual failures should not abort it
    let results = operation::package_cleanup(session, &apps, true)?;

    for (name, count) in &results {
        output::named(name.as_str(), format!("{count} old version(s) removed"));
    }

    if results.is_empty() {
        output::info(rust_i18n::t!("cmd.cleanup_no_old"));
    } else {
        output::info(rust_i18n::t!("cmd.checkup_ok"));
    }

    if args.cache {
        operation::cache_remove(session, "*")?;
        output::info(rust_i18n::t!("cmd.cache_cleaned"));
    }

    Ok(())
}
