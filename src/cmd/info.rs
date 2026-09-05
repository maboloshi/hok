//! Display information about a package.

use clap::Parser;
use libscoop::{Session, package};

use crate::{Result, output};

/// Show package(s) basic information
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// The query string (regex supported)
    query: String,
}

/// Print a package's detail block (the info body).
fn print_package(pkg: &package::Package) {
    output::field(rust_i18n::t!("output.field_identity"), pkg.ident());
    output::field(rust_i18n::t!("output.field_name"), pkg.name());
    output::field(rust_i18n::t!("output.field_bucket"), pkg.bucket());
    output::field(
        rust_i18n::t!("output.field_description"),
        pkg.description().unwrap_or("<no description>"),
    );
    output::field(rust_i18n::t!("output.field_version"), pkg.version());
    output::field(rust_i18n::t!("output.field_homepage"), pkg.homepage());
    output::field(
        rust_i18n::t!("output.field_license"),
        pkg.license().to_string(),
    );
    output::field(
        rust_i18n::t!("output.field_shims"),
        pkg.shims()
            .map(|v| v.join(","))
            .unwrap_or("<no shims>".to_owned()),
    );
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let query = args.query;

    let queries = vec![query.as_str()];
    let options = vec![];
    let packages = package::query::query(session, queries, options, false)?;
    let length = packages.len();

    // A bare name usually matches the same app in several buckets. Scoop's
    // info command describes the installed app, so prefer an installed match
    // for the full detail. When nothing is installed, list the candidates
    // compactly (capped) instead of dumping a field block per match.
    match length {
        0 => {
            return Err(anyhow::anyhow!(rust_i18n::t!(
                "cmd.info_not_found",
                query = query
            )));
        }
        1 => {
            print_package(&packages[0]);
            Ok(())
        }
        _ => {
            let installed: Vec<&package::Package> = {
                let all = package::query::query_installed(session, &["*"], &[])?;
                let names: std::collections::HashSet<String> =
                    all.iter().map(|p| p.name().to_lowercase()).collect();
                packages
                    .iter()
                    .filter(|p| names.contains(&p.name().to_lowercase()))
                    .collect()
            };

            if !installed.is_empty() {
                let n = installed.len();
                for (idx, &pkg) in installed.iter().enumerate() {
                    print_package(pkg);
                    if idx != n - 1 {
                        println!();
                    }
                }
                Ok(())
            } else {
                output::info(rust_i18n::t!("cmd.found", count = length, query = query));
                const CAP: usize = 30;
                for pkg in packages.iter().take(CAP) {
                    println!("  {}  {}", pkg.ident(), pkg.version());
                }
                if length > CAP {
                    output::info(rust_i18n::t!("cmd.info_more", n = length - CAP));
                }
                Ok(())
            }
        }
    }
}
