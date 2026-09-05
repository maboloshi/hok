//! Import installed apps from a JSON file.

use clap::Parser;
use libscoop::{Session, SyncOption, fs, package};

use crate::{Result, output};

/// Import installed packages from a file
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// JSON file to import from
    file: String,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let content = match fs::read_to_string(&args.file) {
        Ok(c) => c,
        Err(e) => {
            return Err(anyhow::anyhow!(rust_i18n::t!(
                "cmd.import_read_err",
                file = args.file,
                e = e
            )));
        }
    };

    let packages = match package::import::parse_import_json(&content) {
        Ok(v) => v,
        Err(e) => {
            return Err(anyhow::anyhow!(rust_i18n::t!(
                "cmd.import_parse_err",
                e = e
            )));
        }
    };

    if packages.is_empty() {
        output::warn(rust_i18n::t!("cmd.import_no_pkgs"));
        return Ok(());
    }

    output::info(rust_i18n::t!("cmd.import_found", count = packages.len()));

    let queries: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
    // Non-interactive restore: always assume yes, and always tolerate
    // per-package failures (a broken app must not abort the whole import).
    let options = vec![SyncOption::AssumeYes, SyncOption::IgnoreFailure];

    match package::sync::sync(session, queries, options).await {
        Ok(_) => output::info(rust_i18n::t!("cmd.import_complete")),
        Err(e) => return Err(anyhow::anyhow!(rust_i18n::t!("cmd.import_err", e = e))),
    }

    Ok(())
}
