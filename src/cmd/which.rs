use clap::Parser;
use libscoop::{operation, QueryOption, Session};

use crate::{output, Result};

/// Show the shim location(s) of a command
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// Command name to search for
    command: String,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let config = session.config();
    let shims_dir = config.root_path().join("shims");
    let command = args.command;

    // Check for .cmd, .ps1, .exe shims
    let exts = ["", ".cmd", ".ps1", ".exe"];
    let mut found = false;

    for ext in &exts {
        let path = shims_dir.join(format!("{}{}", command, ext));
        if path.exists() {
            println!("{}", path.display());
            found = true;
        }
    }

    if !found {
        // Search installed packages for the binary
        let queries = vec!["*"];
        let options = vec![QueryOption::Binary];
        let pkgs = operation::package_query(session, queries, options, true)?;

        for pkg in &pkgs {
            if let Some(shims) = pkg.shims() {
                if shims.iter().any(|s| s == &command) {
                    let path = config.root_path()
                        .join("apps")
                        .join(pkg.name())
                        .join("current");
                    println!("{}", path.display());
                    found = true;
                }
            }
        }
    }

    if !found {
        output::err(format!("Could not find '{}'.", command));
    }

    Ok(())
}
