use clap::Parser;
use libscoop::{operation, Session};

use crate::{output, Result};

/// Show package(s) basic information
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// The query string (regex supported)
    query: String,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let query = args.query;

    let queries = vec![query.as_str()];
    let options = vec![];
    let packages = operation::package_query(session, queries, options, false)?;
    let length = packages.len();
    match length {
        0 => output::err(format!("Could not find package for query '{query}'.")),
        _ => {
            if length == 1 {
                output::info(format!("Found 1 package for query '{}'", query));
            } else {
                output::info(format!("Found {length} package(s) for query '{query}'"));
            }

            for (idx, pkg) in packages.iter().enumerate() {
                output::field("Identity:", pkg.ident());
                output::field("Name:", pkg.name());
                output::field("Bucket:", pkg.bucket());
                output::field("Description:", pkg.description().unwrap_or("<no description>"));
                output::field("Version:", pkg.version());
                output::field("Homepage:", pkg.homepage());
                output::field("License:", pkg.license().to_string());
                output::field("Shims:", pkg.shims()
                    .map(|v| v.join(","))
                    .unwrap_or("<no shims>".to_owned()));

                if idx != (length - 1) {
                    println!();
                }
            }
        }
    }
    Ok(())
}
