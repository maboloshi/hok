//! List dependencies of a package.

use clap::Parser;
use libscoop::{package, QueryOption, Session};
use std::io::Write;

use crate::{output, Result};

/// Show dependencies of a package
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// Name of the package
    package: String,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let query = args.package;
    let queries = vec![query.as_str()];
    let options = vec![QueryOption::Explicit];
    let mut result = package::query::query(session, queries, options, false)?;

    if result.is_empty() {
        output::err(rust_i18n::t!("cmd.depends_not_found", query = query));
        return Ok(());
    }

    // Pick the first match (or let user choose if multiple)
    let pkg = if result.len() == 1 {
        result.remove(0)
    } else {
        result.sort_by_key(|p| p.ident());
        output::info(format!("Found multiple packages named '{query}':\n"));
        for (idx, pkg) in result.iter().enumerate() {
            output::named(format!("{idx}."), format!("{}/{}", pkg.bucket(), pkg.name()));
        }
        output::prompt(rust_i18n::t!("output.select_one", max = result.len() - 1));
        std::io::stdout().flush().ok();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).ok();
        let idx = input.trim().parse::<usize>().unwrap_or(0);
        if idx >= result.len() {
            output::err(rust_i18n::t!("cmd.depends_invalid"));
            return Ok(());
        }
        result.remove(idx)
    };

    // Display the dependency tree
    let tree = package::depends::dependencies_tree(session, pkg.name(), pkg.bucket())?;
    for node in &tree {
        if node.depth == 0 {
            output::named(format!("{}/{}", node.bucket, node.name), "");
        } else if node.already_listed {
            println!(
                "{:indent$} {} (already listed)",
                "",
                node.name,
                indent = node.depth * 2
            );
        } else {
            println!(
                "{:indent$} {} [{}]",
                "",
                node.name,
                node.bucket,
                indent = node.depth * 2
            );
        }
    }

    Ok(())
}
