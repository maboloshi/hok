//! Format package manifests in-place.

use clap::Parser;
use std::path::PathBuf;

use libscoop::package::formatjson;
use libscoop::Session;
use crate::{output, Result};

/// Format manifest JSON files in a bucket directory
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = ".")]
    dir: PathBuf,

    /// Specific app(s) to format (supports wildcards, default: all)
    #[arg(default_value = "*")]
    app: Vec<String>,
}

pub fn execute(args: Args) -> Result<()> {
    let dir = &args.dir;
    if !dir.is_dir() {
        output::err(rust_i18n::t!("cmd.checkurls_err_dir", path = dir.display()));
        return Ok(());
    }

    let report = formatjson::format_manifests(dir, &args.app)?;

    for msg in &report.errors {
        output::err(msg);
    }

    if report.formatted == 0 && report.errors.is_empty() {
        output::info(rust_i18n::t!("cmd.formatjson_none"));
    } else {
        output::info(rust_i18n::t!("cmd.formatjson_count", count = report.formatted));
    }

    Ok(())
}

use crate::cmd::shared_args::Cmd;

impl Cmd for Args {
    type Args = Self;

    #[inline]
    fn execute(args: Self::Args, _session: &Session) -> Result<()> {
        execute(args)
    }
}
