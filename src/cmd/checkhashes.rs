use clap::Parser;
use libscoop::package::checkhashes::{self, CheckHashesOptions, CheckHashesStatus};
use libscoop::Session;
use std::path::PathBuf;

use crate::{output, Result};

/// Verify and update manifest hashes
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = ".")]
    dir: PathBuf,

    /// Specific app(s) to check (supports wildcards, default: all)
    #[arg(default_value = "*")]
    app: Vec<String>,

    /// Update manifest with correct hash when mismatch found
    #[arg(short = 'u', long, action = clap::ArgAction::SetTrue)]
    update: bool,

    /// Force update manifest even when hash matches (re-hash)
    #[arg(short = 'f', long, action = clap::ArgAction::SetTrue)]
    force: bool,

    /// Suppress output for manifests with correct hashes
    #[arg(short = 's', long = "skip-correct", action = clap::ArgAction::SetTrue)]
    skip_correct: bool,

    /// Keep downloaded files in cache after check completes
    #[arg(short = 'k', long = "keep-cache", action = clap::ArgAction::SetTrue)]
    keep_cache: bool,

    /// Use cache directory for downloaded files
    #[arg(short = 'c', long)]
    cache: Option<PathBuf>,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let dir = &args.dir;
    if !dir.is_dir() {
        output::err(rust_i18n::t!("cmd.checkhashes_err_dir", path = dir.display()));
        return Ok(());
    }

    let opts = CheckHashesOptions {
        dir: args.dir,
        app: args.app,
        update: args.update,
        force: args.force,
        skip_correct: args.skip_correct,
        keep_cache: args.keep_cache,
        cache: args.cache,
    };

    let report = checkhashes::check_hashes(session, &opts)?;

    for item in &report.items {
        match item.status {
            CheckHashesStatus::Passed => {
                if !opts.skip_correct {
                    output::done(&item.name);
                }
            }
            CheckHashesStatus::Updated => {
                output::change(rust_i18n::t!("cmd.checkhashes_mismatch_upd"), "->", &item.name);
                for msg in &item.messages {
                    output::warn(msg);
                }
            }
            CheckHashesStatus::Failed => {
                output::err(&item.name);
                for msg in &item.messages {
                    output::err(format!("  {msg}"));
                }
            }
        }
    }

    output::info(rust_i18n::t!(
        "cmd.checkhashes_summary",
        total = report.total,
        passed = report.passed,
        failed = report.failed,
        updated = report.updated
    ));

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
