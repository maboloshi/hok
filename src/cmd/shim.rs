use clap::Parser;
use libscoop::Session;

use crate::{output, Result};

/// List or inspect shims
#[derive(Debug, Parser)]
pub struct Args {
    /// Command: info (default), list
    #[arg(default_value = "list")]
    command: String,
    /// Shim name (for info command)
    name: Option<String>,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let config = session.config();
    let shims_dir = config.root_path().join("shims");

    match args.command.as_str() {
        "list" => {
            if !shims_dir.exists() {
                output::warn("No shims directory found.");
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
        "info" => {
            if let Some(shim_name) = &args.name {
                for ext in &["", ".cmd", ".ps1", ".exe"] {
                    let path = shims_dir.join(format!("{}{}", shim_name, ext));
                    if path.exists() {
                        output::change(shim_name.as_str(), "->", path.display().to_string());
                    }
                }
            } else {
                output::err("Usage: hok shim info <name>");
            }
        }
        _ => {
            output::err(format!("Unknown command: '{}'. Use: list, info", args.command));
        }
    }

    Ok(())
}
