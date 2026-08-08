//! Check for hash problems in package manifests.

use clap::Parser;
use libscoop::package::checkhashes::{self, CheckHashesStatus};
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
    if !validate_dir(&args.dir) {
        return Ok(());
    }

    let inner = checkhashes::Args {
        dir: args.dir,
        app: args.app,
        update: args.update,
        force: args.force,
        skip_correct: args.skip_correct,
        keep_cache: args.keep_cache,
        cache: args.cache,
    };

    // Stream results: each manifest is reported as soon as its hash check
    // completes, matching Scoop's per-manifest output while checking.
    let report = checkhashes::execute(inner, session, |item| match item.status {
        CheckHashesStatus::Passed => {
            if !args.skip_correct {
                output::done(&item.name);
            }
        }
        CheckHashesStatus::Updated => {
            output::change(
                rust_i18n::t!("cmd.checkhashes_mismatch_upd"),
                "->",
                &item.name,
            );
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
    })?;

    output::info(rust_i18n::t!(
        "cmd.checkhashes_summary",
        total = report.total,
        passed = report.passed,
        failed = report.failed,
        updated = report.updated
    ));

    Ok(())
}

use crate::cmd::shared_args::{validate_dir, Cmd};

impl Cmd for Args {
    type Args = Self;

    #[inline]
    fn execute(args: Self::Args, session: &Session) -> Result<()> {
        execute(args, session)
    }
}
