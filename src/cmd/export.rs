//! Export installed apps to a JSON file.

use clap::Parser;
use libscoop::{package::export, Session};

use crate::Result;

/// Export installed packages list
#[derive(Debug, Parser)]
pub struct Args {
    /// Include non-bucket packages (URL/path installs)
    #[arg(short, long, action = clap::ArgAction::SetTrue)]
    all: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let output = export::build_export(session, args.all)?;
    println!("{}", serde_json::to_string_pretty(&output).unwrap());
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
