//! Get or set Scoop configuration values.

use clap::{Parser, Subcommand};
use libscoop::{Session, config};

use crate::{Result, output};

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
    List {
        /// Also show the effective default value of every setting
        #[arg(long)]
        all: bool,
    },
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

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    match args.command {
        Command::Edit => {
            let path = &session.config().path;
            // `config::edit` launches `$EDITOR` (if set); fall back to the
            // system default handler otherwise.
            if !config::edit(session)? {
                libscoop::os::open_file(path)?;
            }
            Ok(())
        }
        Command::List { all } => {
            if all {
                // Column headers are localized in the CLI layer (i18n lives
                // here, not in the lib); alignment accounts for their width.
                let key = hok_i18n::t!("cmd.config_col_key");
                let value = hok_i18n::t!("cmd.config_col_value");
                let default = hok_i18n::t!("cmd.config_col_default");
                let table = config::list_all_with_headers(session, &key, &value, &default)?;
                println!("{table}");
            } else {
                let config_json = config::list(session)?;
                output::info(session.config().path.display().to_string());
                println!("{}", config_json);
            }
            Ok(())
        }
        Command::Set { key, value } => {
            config::set(session, key.as_str(), value.as_str())?;
            output::info(hok_i18n::t!("cmd.config_set", key = key, value = value));
            Ok(())
        }
        Command::Unset { key } => {
            config::set(session, key.as_str(), "")?;
            output::info(hok_i18n::t!("cmd.config_unset", key = key));
            Ok(())
        }
    }
}
