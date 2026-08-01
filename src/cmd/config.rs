use clap::{Parser, Subcommand};
use libscoop::{config, Session};

use crate::{output, util, Result};

/// Configuration management
#[derive(Debug, Parser)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Edit the config file [default: system default editor]
    Edit,
    /// List all settings in key-value
    #[clap(alias = "ls")]
    List,
    /// Add a new setting to the config file
    #[clap(arg_required_else_help = true)]
    Set {
        /// The key of the config
        key: String,
        /// The value of the setting
        value: String,
    },
    /// Remove a setting from config file
    #[clap(arg_required_else_help = true)]
    Unset {
        /// The key of the setting
        key: String,
    },
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    match args.command {
        Command::Edit => {
            let path = &session.config().path;
            if let Ok(editor) = std::env::var("EDITOR") {
                let mut child = std::process::Command::new(editor.as_str())
                    .arg(path)
                    .spawn()?;
                child.wait()?;
            } else {
                util::open_file(path)?;
            }
            Ok(())
        }
        Command::List => {
            let config_json = config::list(session)?;
            output::info(session.config().path.display().to_string());
            println!("{}", config_json);
            Ok(())
        }
        Command::Set { key, value } => {
            config::set(session, key.as_str(), value.as_str())?;
            output::info(rust_i18n::t!("cmd.config_set", key = key, value = value));
            Ok(())
        }
        Command::Unset { key } => {
            config::set(session, key.as_str(), "")?;
            output::info(rust_i18n::t!("cmd.config_unset", key = key));
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
