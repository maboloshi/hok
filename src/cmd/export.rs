//! Export installed apps to a JSON file.

use clap::Parser;
use libscoop::{Session, package::export};

use crate::Result;

/// Export installed packages list
#[derive(Debug, Parser)]
pub struct Args {
    /// Export the configuration too; upstream `scoop export -c` reads it back
    #[arg(short = 'c', long, action = clap::ArgAction::SetTrue)]
    config: bool,
    /// Append hok's own extension (config and per-app state); `scoop import`
    /// ignores it, `hok import` restores it exactly
    #[arg(long, action = clap::ArgAction::SetTrue)]
    hok: bool,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let document = export::build_export(
        session,
        export::ExportSections {
            config: args.config,
            hok: args.hok,
        },
    )?;
    println!("{}", serde_json::to_string_pretty(&document)?);
    Ok(())
}
