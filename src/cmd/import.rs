use clap::Parser;
use libscoop::{package, SyncOption, Session};

use crate::{output, Result};

/// Import installed packages from a file
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// JSON file to import from
    file: String,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let content = match std::fs::read_to_string(&args.file) {
        Ok(c) => c,
        Err(e) => {
            output::err(format!("Error reading '{}': {}", args.file, e));
            return Ok(());
        }
    };

    let packages = match package::import::parse_import_json(&content) {
        Ok(v) => v,
        Err(e) => {
            output::err(format!("Error parsing JSON: {}", e));
            return Ok(());
        }
    };

    if packages.is_empty() {
        output::warn(rust_i18n::t!("cmd.import_no_pkgs"));
        return Ok(());
    }

    output::info(format!("Found {} packages to install.", packages.len()));

    let queries: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
    let options = vec![SyncOption::AssumeYes];

    match package::sync::sync(session, queries, options) {
        Ok(_) => output::info(rust_i18n::t!("cmd.import_complete")),
        Err(e) => output::err(format!("Import error: {}", e)),
    }

    Ok(())
}
use crate::cmd::shared_args::Cmd;

impl Cmd for Args {
    type Args = Self;

    #[inline]
    fn execute(args: Self::Args, session: &Session) -> Result<()> {
        execute(args, session)
    }
}
