use clap::Parser;
use libscoop::{operation, QueryOption, Session};
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
    let mut result = operation::package_query(session, queries, options, false)?;

    if result.is_empty() {
        output::err(format!("Could not find package named '{query}'."));
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
        print!("\nSelect one (0-{}): ", result.len() - 1);
        std::io::stdout().flush().ok();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).ok();
        let idx = input.trim().parse::<usize>().unwrap_or(0);
        if idx >= result.len() {
            output::err("Invalid selection.");
            return Ok(());
        }
        result.remove(idx)
    };

    // Display dependencies
    let mut seen = std::collections::HashSet::new();
    print_deps(session, pkg.name(), pkg.bucket(), 0, &mut seen)?;

    Ok(())
}

/// Recursively print dependencies as an indented tree.
fn print_deps(session: &Session, name: &str, bucket: &str, depth: usize, seen: &mut std::collections::HashSet<String>) -> Result<()> {
    if !seen.insert(name.to_string()) {
        if depth > 0 {
            println!("{:indent$} {} {}", "", name, "(already listed)", indent = depth * 2);
        }
        return Ok(());
    }

    if depth == 0 {
        output::named(format!("{bucket}/{name}"), "");
    } else {
        println!("{:indent$} {} [{}]", "", name, bucket, indent = depth * 2);
    }

    // Query the package to get its dependencies
    let q = format!("{}/{}", bucket, name);
    let queries = vec![q.as_str()];
    let options = vec![QueryOption::Explicit];
    let pkgs = operation::package_query(session, queries, options, false)?;
    let deps = pkgs.first().map(|p| p.dependencies()).unwrap_or_default();

    for dep in &deps {
        let (dep_bucket, dep_name) = dep.split_once('/')
            .map(|(b, n)| (b.to_string(), n.to_string()))
            .unwrap_or_else(|| (bucket.to_string(), dep.clone()));

        print_deps(session, &dep_name, &dep_bucket, depth + 1, seen)?;
    }

    Ok(())
}
