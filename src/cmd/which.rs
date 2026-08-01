use clap::Parser;
use libscoop::{package, QueryOption, Session};

use crate::{output, Result};

/// Show the shim location(s) of a command
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// Command name to search for
    command: String,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let command = args.command;
    let mut found = false;

    // Check for .cmd, .ps1, .exe shims
    let paths = package::shim::shim_paths(session, &command)?;
    for (_, path) in &paths {
        println!("{}", path.display());
        found = true;
    }

    if !found {
        // Search installed packages for the binary
        let queries = vec!["*"];
        let options = vec![QueryOption::Binary];
        let pkgs = package::query::query(session, queries, options, true)?;

        for pkg in &pkgs {
            if let Some(shims) = pkg.shims() {
                if shims.iter().any(|s| s == &command) {
                    let path = session
                        .effective_root_path()
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
use crate::cmd::shared_args::Cmd;

impl Cmd for Args {
    type Args = Self;

    #[inline]
    fn execute(args: Self::Args, session: &Session) -> Result<()> {
        execute(args, session)
    }
}
