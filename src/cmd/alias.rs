use clap::{Parser, Subcommand};
use libscoop::{operation, Session};

use crate::{output, Result};

/// List, add, or remove Scoop aliases
///
/// Examples:
///   hok alias                        list all aliases
///   hok alias list                   list all aliases
///   hok alias add <name> <command>   add an alias
///   hok alias rm  <name>            remove an alias
#[derive(Debug, Parser)]
pub struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List all aliases (default)
    #[clap(alias = "ls")]
    List,

    /// Add a new alias
    Add {
        /// Alias name
        name: String,
        /// Command to execute
        value: String,
    },

    /// Remove an alias
    #[clap(alias = "rm", alias = "delete")]
    Remove {
        /// Alias name to remove
        name: String,
    },
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    match args.command {
        // If no subcommand is provided, default to List
        Some(Command::List) | None => {
            let config = session.config();
            let aliases = config.aliases();
            match aliases {
                Some(map) if !map.is_empty() => {
                    output::header(rust_i18n::t!("cmd.header_aliases"));
                    let mut sorted: Vec<_> = map.iter().collect();
                    sorted.sort_by_key(|(k, _)| *k);
                    for (name, cmd) in sorted {
                        let cmd_short = if cmd.len() > 60 {
                            let truncated: String = cmd.chars().take(57).collect();
                            format!("{}...", truncated)
                        } else {
                            cmd.clone()
                        };
                        output::field(name.as_str(), &cmd_short);
                    }
                }
                _ => {
                    output::warn(rust_i18n::t!("cmd.no_aliases"));
                }
            }
        }
        Some(Command::Add { name, value }) => {
            match operation::alias_add(session, &name, &value) {
                Ok(_) => output::info(rust_i18n::t!("cmd.alias_added", name = name, value = value)),
                Err(e) => output::err(format!("{}: {e}", rust_i18n::t!("output.error"))),
            }
        }
        Some(Command::Remove { name }) => {
            match operation::alias_remove(session, &name) {
                Ok(_) => output::info(rust_i18n::t!("cmd.alias_removed", name = name)),
                Err(e) => output::err(format!("{}: {e}", rust_i18n::t!("output.error"))),
            }
        }
    }

    Ok(())
}
