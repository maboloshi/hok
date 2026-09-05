//! Locate the shim or executable of a package.

use clap::Parser;
use libscoop::{Session, package};

use crate::Result;

/// Show the shim location(s) of a command
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// Command name to search for
    command: String,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let command = args.command;
    let mut found = false;

    // Check for .cmd, .ps1, .exe shims
    let paths = package::shim::shim_paths(session, &command)?;
    for (_, path) in &paths {
        println!("{}", path.display());
        found = true;
    }

    if !found {
        // Scoop's `which` resolves only a real shim/executable file — never an
        // app directory. `shim_paths` above already enumerated every candidate
        // (.exe/.cmd/.ps1); reaching here means the command is not a shim, is a
        // broken shim, or is not an installed app's command.
        return Err(anyhow::anyhow!(rust_i18n::t!(
            "cmd.which_not_found",
            command = command
        )));
    }

    Ok(())
}
