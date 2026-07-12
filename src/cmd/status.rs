use clap::Parser;
use libscoop::{operation, QueryOption, Session};

use crate::{output, Result};

/// Show the status of all installed apps
#[derive(Debug, Parser)]
pub struct Args {
    /// Also show up-to-date apps (default: only outdated)
    #[arg(short = 'a', long, action = clap::ArgAction::SetTrue)]
    all: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let queries = vec!["*"];
    let options = vec![QueryOption::Upgradable];
    let packages = operation::package_query(session, queries, options, false)?;

    let mut outdated = 0u32;
    let mut up_to_date = 0u32;

    for pkg in &packages {
        let upgradable = pkg.upgradable_version();

        if !args.all && upgradable.is_none() {
            up_to_date += 1;
            continue;
        }

        let mut line = format!("{}/{} {}", pkg.name(), pkg.bucket(), pkg.version());

        if let Some(new_ver) = upgradable {
            line.push_str(&format!(" -> {new_ver}"));
            outdated += 1;
        } else {
            up_to_date += 1;
        }

        if pkg.is_held() {
            line.push_str(" [held]");
        }

        output::status(&line);
    }

    if packages.is_empty() {
        output::status("No apps installed.");
    } else if args.all {
        if outdated == 0 {
            output::info("All apps are up to date.");
        } else {
            output::warn(format!("Status: {outdated} outdated / {up_to_date} up to date."));
        }
    } else if outdated == 0 {
        output::info("All apps are up to date.");
    }

    Ok(())
}
