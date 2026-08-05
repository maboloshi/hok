//! Check for potential problems with installed packages.

use clap::Parser;
use libscoop::{package::checkup, Session};

use crate::{output, Result};

/// Check for potential problems with installed packages
#[derive(Debug, Parser)]
pub struct Args {}

pub fn execute(_: Args, session: &Session) -> Result<()> {
    let issues = checkup::check_installed(session);

    if issues.is_empty() {
        output::info(rust_i18n::t!("cmd.no_issues"));
    } else {
        let config = session.config();
        let apps_dir = config.root_path().join("apps");
        if !apps_dir.exists() {
            output::warn(rust_i18n::t!("cmd.no_apps_found"));
            return Ok(());
        }
        for issue in &issues {
            output::named(issue.name.as_str(), &issue.message);
        }
        output::warn(format!("{} issue(s) found.", issues.len()));
        output::status(rust_i18n::t!("cmd.run_reset"));
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
