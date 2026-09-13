//! Check for hash problems in package manifests.
//!
//! Walks the manifests under `--dir`, downloads every URL they declare and
//! compares the computed digest with the `hash` field, mirroring Scoop's
//! `bin/checkhashes.ps1`. Manifests that declare a top-level `url` are checked
//! against it alone; the rest are checked per architecture (64bit, 32bit,
//! arm64).
//!
//! With `--update` a manifest whose hashes do not match is rewritten in place
//! (CRLF, trailing newline preserved), while `--force` also rewrites manifests
//! whose hashes are already correct.

use clap::Parser;
use libscoop::package::checkhashes::{self, CheckHashesStatus};
use libscoop::Session;
use std::path::PathBuf;

use crate::cmd::shared_args::validate_dir;
use crate::{output, Result};

/// Verify and update manifest hashes
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = ".")]
    dir: PathBuf,

    /// Specific app(s) to check (supports wildcards, default: all)
    ///
    /// Matching is case-insensitive, like Scoop's `-App` filter; multiple
    /// patterns are accepted and OR-ed together.
    #[arg(default_value = "*")]
    app: Vec<String>,

    /// Update manifest with correct hash when mismatch found
    #[arg(short = 'u', long, action = clap::ArgAction::SetTrue)]
    update: bool,

    /// Force update manifest even when hash matches
    ///
    /// Downloads are always re-fetched with this flag, even together with
    /// `--keep-cache`.
    #[arg(short = 'f', long, action = clap::ArgAction::SetTrue)]
    force: bool,

    /// Suppress output for manifests with correct hashes
    #[arg(short = 's', long = "skip-correct", action = clap::ArgAction::SetTrue)]
    skip_correct: bool,

    /// Keep downloaded files after the check completes
    ///
    /// Equivalent to Scoop's `-UseCache`: the next run reuses the cached files
    /// instead of downloading them again, so it may check an already
    /// out-of-date file. The cache is keyed by manifest version.
    #[arg(short = 'k', long = "keep-cache", action = clap::ArgAction::SetTrue)]
    keep_cache: bool,

    /// Use cache directory for downloaded files
    ///
    /// Defaults to a temporary directory that is removed at the end of the run
    /// unless `--keep-cache` is given.
    #[arg(short = 'c', long)]
    cache: Option<PathBuf>,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    validate_dir(&args.dir)?;

    let options = checkhashes::CheckhashesOptions {
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
    let report = checkhashes::execute(options, session, |item| match item.status {
        CheckHashesStatus::Passed => {
            if !args.skip_correct {
                output::done(&item.name);
            }
        }
        CheckHashesStatus::Updated => {
            output::done(format!(
                "{}: {}",
                item.name,
                hok_i18n::t!("cmd.checkhashes_mismatch_upd")
            ));
            for msg in &item.messages {
                output::warn(msg);
            }
        }
        CheckHashesStatus::Failed => {
            output::err(&item.name);
            for msg in &item.messages {
                output::err(format!("    {msg}"));
            }
        }
    })
    .await?;

    output::info(hok_i18n::t!(
        "cmd.checkhashes_summary",
        total = report.total,
        passed = report.passed,
        failed = report.failed,
        updated = report.updated
    ));

    Ok(())
}
