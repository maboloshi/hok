use clap::{Parser, Subcommand};
use libscoop::{package, Session};

use crate::{output, Result};

/// List or inspect shims
#[derive(Debug, Parser)]
pub struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List all shims (default)
    #[clap(alias = "ls")]
    List,
    /// Show shim paths for a specific app
    Info {
        /// Shim name
        name: String,
    },
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    match args.command.unwrap_or(Command::List) {
        Command::List => {
            let shims = package::shim::list_shims(session)?;
            if shims.is_empty() {
                output::warn(rust_i18n::t!("cmd.shim_no_dir"));
            } else {
                for name in &shims {
                    output::named(name.as_str(), "(shim)");
                }
            }
        }
        Command::Info { name } => {
            let paths = package::shim::shim_paths(session, &name)?;
            for (_, path) in &paths {
                output::change(name.as_str(), "->", path.display().to_string());
            }
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
