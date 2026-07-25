use clap::Parser;
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
    /// Command: list (default), add, rm
    command: Option<String>,
    /// Alias name (for add/rm)
    name: Option<String>,
    /// Alias command (for add)
    value: Option<String>,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    // Determine command from positional args or use default
    let cmd = args.command.as_deref().unwrap_or("list");

    match cmd {
        "list" => {
            let config = session.config();
            let aliases = config.aliases();
            match aliases {
                Some(map) if !map.is_empty() => {
                    output::header(rust_i18n::t!("cmd.header_aliases"));
                    let mut sorted: Vec<_> = map.iter().collect();
                    sorted.sort_by_key(|(k, _)| *k);
                    for (name, cmd) in sorted {
                        let cmd_short = if cmd.len() > 60 {
                            format!("{}...", &cmd[..57])
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
        "add" => {
            if let (Some(name), Some(value)) = (&args.name, &args.value) {
                match operation::alias_add(session, name, value) {
                    Ok(_) => output::done(format!("added: {name} -> {value}")),
                    Err(e) => output::err(format!("{e}")),
                }
            } else {
                output::err(rust_i18n::t!("cmd.alias_add_usage"));
            }
        }
        "rm" | "remove" | "delete" => {
            if let Some(name) = &args.name {
                match operation::alias_remove(session, name) {
                    Ok(_) => output::done(format!("removed: {name}")),
                    Err(e) => output::err(format!("{e}")),
                }
            } else {
                output::err(rust_i18n::t!("cmd.alias_rm_usage"));
            }
        }
        _ => {
            output::err(rust_i18n::t!("cmd.alias_unknown", cmd = cmd));
        }
    }

    Ok(())
}
