//! Download package(s) command.
//!
//! Thin CLI wrapper around [`package::download::download_apps`]: downloads
//! the package files into the cache directory without installing them
//! (Scoop's `scoop download`).

use clap::{ArgAction, Parser};
use libscoop::{package, Session};

use crate::Result;

/// Download package(s) into the cache folder without installing
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// The package(s) to download
    #[arg(required = true, action = ArgAction::Append)]
    package: Vec<String>,

    /// Force download (overwrite cache)
    #[arg(short = 'f', long, action = ArgAction::SetTrue)]
    force: bool,

    /// Skip package integrity check (Scoop: --skip-hash-check)
    #[arg(short = 's', long, visible_alias = "skip-hash-check", action = ArgAction::SetTrue)]
    no_hash_check: bool,

    /// Use the specified architecture (32bit/64bit/arm64), overriding the
    /// runtime-detected and configured default (Scoop's -a/--arch)
    #[arg(short = 'a', long = "arch")]
    arch: Option<String>,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    // `-a/--arch` overrides the effective architecture (same as install).
    if let Some(arch) = args.arch.as_deref() {
        session.set_default_architecture(arch)?;
    }

    let queries = args.package.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    let opts = package::download::DownloadOptions {
        force: args.force,
        check_hash: !args.no_hash_check,
    };

    let handle = crate::eventloop::run_event_loop_default(session);
    let result = package::download::download_apps(session, &queries, &opts);
    handle.join().unwrap();
    result?;

    Ok(())
}
