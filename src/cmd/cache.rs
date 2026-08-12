//! Manage the download cache.

use clap::{ArgAction, Parser, Subcommand};
use libscoop::{cache, Session};

use crate::{format, output, Result};

/// Package cache management
#[derive(Debug, Parser)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List download caches
    #[clap(alias = "ls")]
    List {
        /// List caches matching the query
        query: Option<String>,
    },
    /// Remove download caches
    #[clap(alias = "rm")]
    #[clap(arg_required_else_help = true)]
    Remove {
        /// Remove caches matching the query
        query: Option<String>,
        /// Remove all caches
        #[arg(short, long, action = ArgAction::SetTrue, conflicts_with = "query")]
        all: bool,
    },
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    match args.command {
        Command::List { query } => {
            let query = query.unwrap_or("*".to_string());
            let files = cache::list(session, query.as_str())?;
            let mut total_size: u64 = 0;
            let mut total_count = 0u32;

            for f in files.into_iter() {
                // Skip entries deleted concurrently (NotFound) instead of
                // aborting the whole listing; other errors still surface.
                let metadata = match f.path().metadata() {
                    Ok(m) => m,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e.into()),
                };
                let size = metadata.len();
                total_size += size;
                total_count += 1;

                println!(
                    "{:>8} {} ({}) {:>}",
                    format::humansize(size, true),
                    f.package_name(),
                    f.version(),
                    f.file_name()
                );
            }

            println!(
                "{:>8} {} files, {}",
                "Total:",
                total_count,
                format::humansize(total_size, true)
            );

            Ok(())
        }
        Command::Remove { query, all } => {
            if all {
                match cache::remove(session, "*") {
                    Ok(_) => {
                        output::info(rust_i18n::t!("cmd.cache_all_removed"));
                        return Ok(());
                    }
                    Err(e) => return Err(e.into()),
                }
            }

            if let Some(query) = query {
                match cache::remove(session, query.as_str()) {
                    Ok(_) => {
                        if query == "*" {
                            output::info(rust_i18n::t!("cmd.cache_all_removed"));
                        } else {
                            output::info(rust_i18n::t!("cmd.cache_matching_removed", query = query));
                        }
                    }
                    Err(e) => return Err(e.into()),
                }
            }

            Ok(())
        }
    }
}
