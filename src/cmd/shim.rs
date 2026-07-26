use clap::{Parser, Subcommand};
use libscoop::Session;

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
    let config = session.config();
    let shims_dir = config.root_path().join("shims");

    match args.command.unwrap_or(Command::List) {
        Command::List => {
            if !shims_dir.exists() {
                output::warn(rust_i18n::t!("cmd.shim_no_dir"));
                return Ok(());
            }
            for entry in std::fs::read_dir(&shims_dir)?.flatten() {
                let name = entry.file_name();
                if let Some(name) = name.to_str() {
                    // Skip .cmd files (show only .ps1 or no extension)
                    if name.ends_with(".ps1") {
                        let stem = &name[..name.len() - 4];
                        output::named(stem, "(shim)");
                    }
                }
            }
        }
        Command::Info { name } => {
            for ext in &["", ".cmd", ".ps1", ".exe"] {
                let path = shims_dir.join(format!("{}{}", name, ext));
                if path.exists() {
                    output::change(name.as_str(), "->", path.display().to_string());
                }
            }
        }
    }

    Ok(())
}
