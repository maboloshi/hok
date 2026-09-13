//! Search available packages.

use clap::{ArgAction, Parser};
use libscoop::{Session, package};

use crate::{Result, output};

/// Search available package(s)
///
/// Search available package(s) from synced buckets.
/// The query is performed against package names by default, use
/// --with-description or --with-binary to search through package
/// descriptions or binaries. With no query every available package is listed,
/// matching upstream's empty-regex search (`libexec/scoop-search.ps1:8-9`).
#[derive(Debug, Parser)]
pub struct Args {
    /// The query string (regex supported by default)
    #[arg(action = ArgAction::Append)]
    query: Vec<String>,
    /// Turn regex off and use explicit matching
    #[arg(short = 'e', long, action = ArgAction::SetTrue, conflicts_with_all = &["with_binary", "with_description"])]
    explicit: bool,
    /// Search through package binaries as well
    #[arg(short = 'B', long, action = ArgAction::SetTrue)]
    with_binary: bool,
    /// Search through package descriptions as well
    #[arg(short = 'D', long, action = ArgAction::SetTrue)]
    with_description: bool,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let queries = args.query.iter().map(|s| s.as_str()).collect::<Vec<_>>();

    // Build QueryArgs from individual fields, then convert to options
    let qargs = crate::cmd::shared_args::QueryArgs {
        explicit: args.explicit,
        with_binary: args.with_binary,
        with_description: args.with_description,
    };
    let options = qargs.to_query_options();

    let packages = package::query::query(session, queries, options, false)?;

    // Scoop treats "no match" as a failed search (`scoop-search.ps1:195-200`
    // warns and exits 1) so scripts can test a package's existence.
    if packages.is_empty() {
        output::warn(hok_i18n::t!("cmd.search_no_match"));
        return Err(crate::ExitCode(1).into());
    }

    for pkg in packages {
        let mut line = String::new();
        line.push_str(format!("{}/{} {}", pkg.name(), pkg.bucket(), pkg.version()).as_str());

        if pkg.is_strictly_installed() {
            let manifest_version = pkg.version();
            let installed_version = pkg.installed_version().unwrap();
            if manifest_version != installed_version {
                line.push_str(format!(" [installed: {installed_version}]").as_str());
            } else {
                line.push_str(" [installed]");
            }
        }

        if args.with_description {
            let description = pkg.description().unwrap_or("<no description>");
            line.push_str(format!("\n  {description}").as_str());
        }

        if args.with_binary {
            let shims = match pkg.shims() {
                None => "<no shims>".to_owned(),
                Some(shims) => shims.join(","),
            };
            line.push_str(format!("\n  {shims}").as_str());
        }

        output::status(&line);
    }
    Ok(())
}
