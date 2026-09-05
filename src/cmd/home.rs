//! Open the homepage of a package.

use clap::Parser;
use libscoop::{QueryOption, Session, package};

use crate::{Result, output};

/// Browse the homepage of a package
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// The package name
    package: String,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let query = args.package;

    let queries = vec![query.as_str()];
    let options = vec![QueryOption::Explicit];
    let mut result = package::query::query(session, queries, options, false)?;

    match result.len() {
        0 => {
            return Err(anyhow::anyhow!(rust_i18n::t!(
                "cmd.home_not_found",
                query = query
            )));
        }
        1 => {
            let package = &result[0];
            let url = package.homepage();
            libscoop::os::open_url(url)?;
        }
        _ => {
            result.sort_by_key(|p| p.ident());

            output::info(rust_i18n::t!("cmd.home_multiple", query = query));
            for (idx, pkg) in result.iter().enumerate() {
                println!(
                    "  {idx}. {}/{} ({})",
                    pkg.bucket(),
                    pkg.name(),
                    pkg.homepage()
                );
            }
            let index = output::prompt_select(result.len())
                .ok_or_else(|| anyhow::anyhow!(rust_i18n::t!("cmd.home_invalid")))?;
            let package = &result[index];
            let url = package.homepage();
            libscoop::os::open_url(url)?;
            return Ok(());
        }
    }
    Ok(())
}
