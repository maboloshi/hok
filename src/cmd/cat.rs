use clap::Parser;
use libscoop::{operation, QueryOption, Session};
use std::process::Command;

use crate::{output, Result};

/// Inspect the manifest of a package
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// Name of the package to be inspected
    package: String,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let query = args.package;

    let queries = vec![query.as_str()];
    let options = vec![QueryOption::Explicit];
    let mut result = operation::package_query(session, queries, options, false)?;

    if result.is_empty() {
        output::err(rust_i18n::t!("cmd.cat_not_found", query = query))
    } else {
        let length = result.len();
        let package = if length == 1 {
            &result[0]
        } else {
            result.sort_by_key(|p| p.ident());

            println!("Found multiple packages named '{query}':\n");
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
            let num = match input.trim().parse::<usize>() {
                Ok(n) if n < length => n,
                _ => {
                    output::err(rust_i18n::t!("cmd.cat_invalid"));
                    return Ok(());
                }
            };
            &result[num]
        };

        let path = package.manifest().path();
        output::info(format!("{}", path.display()));

        // Use bat.exe for syntax-highlighted output if available
        // (install via: hok install bat)
        if libscoop::internal::os::is_program_available("bat.exe") {
            let config = session.config();
            let mut args = vec!["--no-paging"];
            let cat_style = config.cat_style();
            if !cat_style.is_empty() {
                args.push("--style");
                args.push(cat_style);
            }
            args.push("--language");
            args.push("json");
            let mut child = Command::new("bat.exe").arg(path).args(args).spawn()?;
            child.wait()?;
        } else {
            let content = std::fs::read_to_string(path)?;
            println!("{}", content.trim());
        }
    }
    Ok(())
}
