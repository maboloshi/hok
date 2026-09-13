//! Format package manifests in-place.

use clap::Parser;
use std::path::PathBuf;

use crate::cmd::shared_args::{validate_dir, write_dir};
use crate::{output, Result};
use libscoop::package::formatjson;

/// Format manifest JSON files in a bucket directory
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    ///
    /// Required: this command rewrites every manifest it finds in place, and
    /// the scan is a recursive `*.json` walk that cannot tell a manifest from
    /// any other JSON file.
    #[arg(short = 'd', long)]
    dir: Option<PathBuf>,

    /// Specific app(s) to format (supports wildcards, default: all)
    #[arg(default_value = "*")]
    app: Vec<String>,
}

pub async fn execute(args: Args) -> Result<()> {
    // Always a write: every manifest found under `dir` is rewritten in place,
    // so the target may never default to the current directory.
    let dir = write_dir(args.dir, "formatjson")?;
    validate_dir(&dir)?;

    let report = formatjson::format_manifests(&dir, &args.app)?;

    for msg in &report.errors {
        output::err(msg);
    }

    for msg in &report.warnings {
        output::warn(msg);
    }

    if report.formatted == 0 && report.errors.is_empty() {
        output::info(hok_i18n::t!("cmd.formatjson_none"));
    } else {
        output::info(hok_i18n::t!(
            "cmd.formatjson_count",
            count = report.formatted
        ));
    }

    Ok(())
}
