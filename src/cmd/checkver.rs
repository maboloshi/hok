//! Check for outdated package manifests.

use clap::Parser;
use libscoop::Session;
use libscoop::package::checkver::{self, ReportSeverity};
use std::path::PathBuf;

use crate::cmd::shared_args::{scan_dir, validate_dir, write_dir};
use crate::{Result, output};

/// Check manifest for a newer version
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    ///
    /// Defaults to the current directory, but required with `--update`/
    /// `--force-update` (which rewrite manifests) or hok refuses to run.
    #[arg(short = 'd', long)]
    dir: Option<PathBuf>,

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

    /// Throw on the first error instead of reporting it and continuing
    #[arg(long = "throw", action = clap::ArgAction::SetTrue)]
    throw_error: bool,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    // `--update` and `--force-update` both rewrite manifests (`--force-update`
    // implies `--update`, like upstream's `if ($ForceUpdate) { $Update = $true }`
    // in `bin/checkver.ps1:404`), so either one demands an explicit target.
    // A plain check only reads, and keeps the current-directory default.
    let dir = if args.update || args.force_update {
        write_dir(args.dir, "checkver")?
    } else {
        scan_dir(args.dir)
    };
    validate_dir(&dir)?;

    let options = checkver::CheckverOptions {
        dir,
        app: args.app,
        update: args.update,
        force_update: args.force_update,
        skip_updated: args.skip_updated,
        version: args.version,
        timeout: args.timeout,
        throw_error: args.throw_error,
    };

    // Stream results: each manifest is reported as soon as its check completes,
    // matching Scoop's event-driven output in checkver.ps1.
    checkver::execute(options, session, |r| {
        if let Some(msg) = &r.message {
            match r.severity {
                ReportSeverity::Warn => output::warn(format!("{}: {}", r.stem, msg)),
                _ => output::err(format!("{}: {}", r.stem, msg)),
            }
            return;
        }

        let Some(ver) = &r.new_version else {
            return;
        };

        if ver == &r.current {
            output::status(format!("  {} ({})", r.stem, ver));
            if r.updated {
                output::done(hok_i18n::t!("cmd.checkver_updated_to", ver = ver));
            }
        } else {
            output::status(format!("  {} ({} -> {})", r.stem, r.current, ver));
            if r.autoupdate_available {
                output::status(format!("    {}", hok_i18n::t!("cmd.checkver_autoupdate")));
            }
            if r.updated {
                output::done(hok_i18n::t!("cmd.checkver_updated_to", ver = ver));
            }
        }
    })
    .await?;

    Ok(())
}
