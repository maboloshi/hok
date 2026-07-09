use clap::Parser;
use libscoop::Session;

use crate::Result;

/// Create a manifest from a URL (interactive)
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// URL to create manifest from
    url: String,
}

pub fn execute(_: Args, _: &Session) -> Result<()> {
    eprintln!("Manifest creation is not yet implemented.");
    eprintln!("Use 'scoop create <url>' to create manifests interactively.");
    Ok(())
}
