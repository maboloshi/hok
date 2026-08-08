//! Report package manifests that are missing checkver.

use clap::Parser;
use libscoop::package::missing_checkver;
use libscoop::Session;
use std::path::PathBuf;

use crate::{output, Result};

/// Check bucket manifests missing checkver and autoupdate
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = ".")]
    dir: PathBuf,

    /// Only show manifests that have checkver/autoupdate (inverse)
    #[arg(short = 's', long, action = clap::ArgAction::SetTrue)]
    supported: bool,
}

pub fn execute(args: Args) -> Result<()> {
    if !validate_dir(&args.dir) {
        return Ok(());
    }
    let dir = &args.dir;

    let report = missing_checkver::scan(dir, args.supported)?;

    if args.supported {
        for name in &report.supported_items {
            output::done(name);
        }
    } else {
        for item in &report.missing_items {
            output::named(&item.name, format!("({})", item.issues.join(", ")));
        }

        output::info(rust_i18n::t!(
            "cmd.missing_checkver_scan",
            total = report.total,
            missing = report.missing_checkver,
            noauto = report.missing_autoupdate
        ));
        if report.missing_checkver == 0 && report.missing_autoupdate == 0 {
            output::info(rust_i18n::t!("cmd.missing_checkver_all"));
        }
    }

    Ok(())
}

use crate::cmd::shared_args::{validate_dir, Cmd};

impl Cmd for Args {
    type Args = Self;

    #[inline]
    fn execute(args: Self::Args, _session: &Session) -> Result<()> {
        execute(args)
    }
}
