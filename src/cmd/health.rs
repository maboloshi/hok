//! Check for potential problems with installed packages.

use clap::Parser;
use libscoop::{Session, package::health};

use crate::{Result, output};

/// Check for potential problems with installed packages
#[derive(Debug, Parser)]
pub struct Args {}

pub async fn execute(_: Args, session: &Session) -> Result<()> {
    // Check the apps directory before scanning: `check_installed` returns an
    // empty list when it is missing, so this branch has to come first to
    // actually be reachable.
    if !session.apps_dir().exists() {
        output::warn(hok_i18n::t!("cmd.no_apps_found"));
        return Ok(());
    }

    let issues = health::check_installed(session);
    if issues.is_empty() {
        output::info(hok_i18n::t!("cmd.no_issues"));
    } else {
        for issue in &issues {
            output::named(issue.name.as_str(), &issue.message);
        }
        output::info(hok_i18n::t!("cmd.health_issue_count", count = issues.len()));
        output::info(hok_i18n::t!("cmd.run_reset"));
    }

    Ok(())
}
