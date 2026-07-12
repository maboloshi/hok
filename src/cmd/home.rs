use clap::Parser;
use libscoop::{operation, QueryOption, Session};
use std::io::Write;

use crate::{output, util, Result};

/// Browse the homepage of a package
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// The package name
    package: String,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let query = args.package;

    let queries = vec![query.as_str()];
    let options = vec![QueryOption::Explicit];
    let mut result = operation::package_query(session, queries, options, false)?;

    match result.len() {
        0 => output::err(format!("Could not find package named '{query}'.")),
        1 => {
            let package = &result[0];
            let url = package.homepage();
            util::open_url(url)?;
        }
        _ => {
            result.sort_by_key(|p| p.ident());

            output::info(format!("Found multiple packages named '{query}':\n"));
            for (idx, pkg) in result.iter().enumerate() {
                println!(
                    "  {idx}. {}/{} ({})",
                    pkg.bucket(),
                    pkg.name(),
                    pkg.homepage()
                );
            }
            print!("\nPlease select one, enter the number to continue: ");
            std::io::stdout().flush().unwrap();
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).unwrap();
            let parsed = input.trim().parse::<usize>();
            if let Ok(num) = parsed {
                if num < result.len() {
                    let package = &result[num];
                    let url = package.homepage();
                    util::open_url(url)?;
                    return Ok(());
                }
            }
            output::err("Invalid input.");
        }
    }
    Ok(())
}
