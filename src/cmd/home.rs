use clap::Parser;
use libscoop::{operation, QueryOption, Session};

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
        0 => output::err(rust_i18n::t!("cmd.home_not_found", query = query)),
        1 => {
            let package = &result[0];
            let url = package.homepage();
            util::open_url(url)?;
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
            output::prompt(rust_i18n::t!("output.select_prompt"));
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
            output::err(rust_i18n::t!("cmd.home_invalid"));
        }
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
