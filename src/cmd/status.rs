use clap::Parser;
use crossterm::style::Stylize;
use libscoop::{operation, QueryOption, Session};

use crate::Result;

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

        let mut line = format!("{}/{} {}", pkg.name(), pkg.bucket().green(), pkg.version());

        if let Some(new_ver) = upgradable {
            line.push_str(&format!(" -> {}", new_ver.blue()));
            outdated += 1;
        } else {
            up_to_date += 1;
        }

        if pkg.is_held() {
            line.push_str(&format!(" [{}]", "held".magenta()));
        }

        println!("{}", line);
    }

    if packages.is_empty() {
        println!("No apps installed.");
    } else if args.all {
        if outdated == 0 {
            println!("\n{}", "All apps are up to date.".green());
        } else {
            println!(
                "\n{}",
                format!("Status: {} outdated / {} up to date.", outdated, up_to_date).yellow()
            );
        }
    } else if outdated == 0 {
        println!("\n{}", "All apps are up to date.".green());
    }

    Ok(())
}
