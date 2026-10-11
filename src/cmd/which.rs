//! Locate the shim or executable of a command.

use clap::Parser;
use libscoop::{Session, package};

use crate::{ExitCode, Result, output};

/// Show the shim location(s) of a command
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// Command name to search for
    command: String,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let command = args.command;

    // Upstream `scoop which` prints the path that would actually run
    // (libexec/scoop-which.ps1:12-19 via `Get-CommandPath`): the target a
    // Scoop shim points at, a Scoop alias script, or the program itself when
    // it is found on `PATH`. `command_path` is the one implementation of that
    // resolution (it classifies both the user and the global shims folder).
    match package::shim::command_path(session, &command) {
        Some(path) => {
            println!("{}", path.display());
            Ok(())
        }
        None => {
            // Not on `PATH`, not a Scoop shim, or a shim whose target is gone
            // — upstream warns and exits 2 for all three.
            output::warn(hok_i18n::t!("cmd.which_not_found", command = command));
            Err(ExitCode(2).into())
        }
    }
}
