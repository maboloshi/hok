use clap::{ArgAction, Parser, Subcommand};
use crossterm::style::Stylize;
use libscoop::{operation, Session};

use crate::{output, Result};

/// Manage manifest buckets
#[derive(Debug, Parser)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Add a bucket
    #[clap(arg_required_else_help = true)]
    Add {
        /// The bucket name
        name: String,
        /// The bucket repository url (optional for known buckets)
        repo: Option<String>,
    },
    /// List buckets
    #[clap(alias = "ls")]
    List {
        /// List known buckets
        #[arg(short = 'k', long, action = ArgAction::SetTrue)]
        known: bool,
    },
    /// Remove bucket(s)
    #[clap(alias = "rm")]
    #[clap(arg_required_else_help = true)]
    Remove {
        /// The bucket name(s)
        #[arg(required = true, action = ArgAction::Append)]
        name: Vec<String>,
    },
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    match args.command {
        Command::Add { name, repo } => {
            output::progress(rust_i18n::t!("cmd.adding_bucket"), &name);
            let repo = repo.as_deref().unwrap_or_default();
            match operation::bucket_add(session, name.as_str(), repo) {
                Ok(..) => output::ok(),
                Err(err) => {
                    output::err(rust_i18n::t!("cmd.bucket_err"));
                    return Err(err.into());
                }
            }
            Ok(())
        }
        Command::List { known } => {
            if known {
                let known_buckets = operation::bucket_list_known();
                let max_name = known_buckets.iter().map(|&(n, _)| n.len()).max().unwrap_or(4);
                output::header(rust_i18n::t!("cmd.header_buckets_list"));
                for (name, repo) in &known_buckets {
                    println!("  {}  {}", format!("{:<1$}", name, max_name).dark_cyan(), repo);
                }
                Ok(())
            } else {
                match operation::bucket_list(session) {
                    Err(e) => Err(e.into()),
                    Ok(buckets) => {
                        for bucket in buckets {
                            output::named(bucket.name(), "");
                            output::field(" ├─manifests", bucket.manifest_count().to_string());
                            if let Some(updated) = bucket.updated_at() {
                                output::field(" ├─updated", updated);
                            }
                            output::field(" └─source", bucket.source());
                        }
                        Ok(())
                    }
                }
            }
        }
        Command::Remove { name } => {
            for name in name {
                output::progress(rust_i18n::t!("cmd.removing_bucket"), &name);
                match operation::bucket_remove(session, name.as_str()) {
                    Ok(..) => output::ok(),
                    Err(err) => {
                        output::err(rust_i18n::t!("cmd.bucket_err"));
                        return Err(err.into());
                    }
                }
            }
            Ok(())
        }
    }
}
use crate::cmd::shared_args::Cmd;

impl Cmd for Args {
    type Args = Self;

    #[inline]
    fn execute(args: Self::Args, session: &Session) -> Result<()> {
        execute(args, session)
    }
}
