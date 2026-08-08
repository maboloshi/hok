//! Check for outdated package manifests.

use clap::Parser;
use libscoop::package::checkver::{self, ReportSeverity};
use libscoop::Session;
use std::path::PathBuf;

use crate::{output, Result};

/// Check manifest for a newer version
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = ".")]
    dir: PathBuf,

    /// Specific app(s) to check (supports wildcards, default: all)
    #[arg(default_value = "*")]
    app: Vec<String>,

    /// Update manifest with new version and trigger autoupdate
    #[arg(short = 'u', long, action = clap::ArgAction::SetTrue)]
    update: bool,

    /// Force update even when version is unchanged (useful for hash updates)
    #[arg(short = 'f', long, action = clap::ArgAction::SetTrue)]
    force_update: bool,

    /// Skip manifests that are already up-to-date
    #[arg(short = 's', long = "skip-updated", action = clap::ArgAction::SetTrue)]
    skip_updated: bool,

    /// Update manifest to specific version (skip version detection)
    #[arg(short = 'V', long)]
    version: Option<String>,

    /// Request timeout in seconds
    #[arg(short = 't', long, default_value = "30")]
    timeout: u64,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let dir = &args.dir;
    if !dir.is_dir() {
        output::err(rust_i18n::t!("cmd.checkver_err_dir", path = dir.display()));
        return Ok(());
    }

    let inner = checkver::Args {
        dir: args.dir,
        app: args.app,
        update: args.update,
        force_update: args.force_update,
        skip_updated: args.skip_updated,
        version: args.version,
        timeout: args.timeout,
    };

    let reports = checkver::execute(inner, session)?;
    for r in &reports {
        if let Some(msg) = &r.message {
            match r.severity {
                ReportSeverity::Warn => output::warn(format!("{}: {}", r.stem, msg)),
                _ => output::err(format!("{}: {}", r.stem, msg)),
            }
            continue;
        }

        let Some(ver) = &r.new_version else {
            continue;
        };

        if ver == &r.current {
            output::status(format!("  {} ({})", r.stem, ver));
            if r.updated {
                output::done(rust_i18n::t!("cmd.checkver_updated_to", ver = ver));
            }
        } else {
            output::status(format!("  {} ({} -> {})", r.stem, r.current, ver));
            if r.autoupdate_available {
                output::status("    autoupdate available");
            }
            if r.updated {
                output::done(rust_i18n::t!("cmd.checkver_updated_to", ver = ver));
            }
        }
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
