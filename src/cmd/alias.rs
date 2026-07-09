use clap::Parser;
use crossterm::style::Stylize;
use libscoop::{operation, Session};

use crate::Result;

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
                    println!("{}", "Aliases:".green());
                    let mut sorted: Vec<_> = map.iter().collect();
                    sorted.sort_by_key(|(k, _)| *k);
                    for (name, cmd) in sorted {
                        let cmd_short = if cmd.len() > 60 {
                            format!("{}...", &cmd[..57])
                        } else {
                            cmd.clone()
                        };
                        println!("  {} -> {}", name.as_str().blue(), cmd_short);
                    }
                }
                _ => {
                    println!("{}", "No aliases configured.".yellow());
                }
            }
        }
        "add" => {
            if let (Some(name), Some(value)) = (&args.name, &args.value) {
                match operation::alias_add(session, name, value) {
                    Ok(_) => println!("  {} added: {} -> {}", "✓".green(), name.as_str().blue(), value),
                    Err(e) => eprintln!("Error: {}", e),
                }
            } else {
                eprintln!("Usage: hok alias add <name> <command>");
            }
        }
        "rm" | "remove" | "delete" => {
            if let Some(name) = &args.name {
                match operation::alias_remove(session, name) {
                    Ok(_) => println!("  {} removed: {}", "✓".green(), name.as_str().blue()),
                    Err(e) => eprintln!("Error: {}", e),
                }
            } else {
                eprintln!("Usage: hok alias rm <name>");
            }
        }
        _ => {
            eprintln!("Unknown command: '{}'. Use: list, add, rm", cmd);
        }
    }

    Ok(())
}
