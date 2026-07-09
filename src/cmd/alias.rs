use clap::Parser;
use libscoop::Session;

use crate::Result;

/// Manage Scoop aliases
#[derive(Debug, Parser)]
pub struct Args {
    /// Command: list (default), add, rm
    #[arg(default_value = "list")]
    command: String,
}

pub fn execute(_: Args, _: &Session) -> Result<()> {
    eprintln!("Alias management is not yet implemented.");
    eprintln!("Use 'scoop config alias.<name> <value>' to set aliases.");
    Ok(())
}
