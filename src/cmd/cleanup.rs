//! Clean up apps by removing old versions.

use clap::{ArgAction, Parser};
use libscoop::{cache, package, Session};

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
    /// Cleanup a globally installed app
    #[arg(short = 'g', long, action = ArgAction::SetTrue)]
    global: bool,
    /// Remove download cache simultaneously
    #[arg(short = 'k', long, action = ArgAction::SetTrue)]
    cache: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    session.set_global(args.global);
    // --all or '*' overrides any specific app names
    let apps: Vec<String> = if args.all || args.app.iter().any(|a| a == "*") {
        Vec::new() // empty means "all apps" in package_cleanup
    } else {
        args.app
    };
    // Cleanup is a maintenance operation; individual failures should not abort it
    let results = package::cleanup::cleanup(session, &apps, true)?;

    for (name, count, failed) in &results {
        let msg = if *failed > 0 {
            rust_i18n::t!("cmd.cleanup_removed_failed", count = *count, failed = *failed)
        } else {
            rust_i18n::t!("cmd.cleanup_removed", count = *count)
        };
        output::named(name.as_str(), msg);
    }

    if results.is_empty() {
        output::info(rust_i18n::t!("cmd.cleanup_no_old"));
    } else {
        output::info(rust_i18n::t!("cmd.checkup_ok"));
    }

    if args.cache {
        cache::remove(session, "*")?;
        output::info(rust_i18n::t!("cmd.cache_cleaned"));
    }

    Ok(())
}

use crate::cmd::shared_args::Cmd;

impl Cmd for Args {
    type Args = Self;

    #[inline]
    fn execute(args: Self::Args, session: &Session) -> Result<()> {
        execute(args, session)
    }
}
