use clap::Parser;
use libscoop::package::checkver;
use libscoop::Session;
use std::path::PathBuf;

use crate::Result;

/// Check manifest for a newer version
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = ".")]
    pub(crate) dir: PathBuf,

    /// Specific app(s) to check (supports wildcards, default: all)
    #[arg(default_value = "*")]
    pub(crate) app: Vec<String>,

    /// Update manifest with new version and trigger autoupdate
    #[arg(short = 'u', long, action = clap::ArgAction::SetTrue)]
    pub(crate) update: bool,

    /// Force update even when version is unchanged (useful for hash updates)
    #[arg(short = 'f', long, action = clap::ArgAction::SetTrue)]
    pub(crate) force_update: bool,

    /// Skip manifests that are already up-to-date
    #[arg(short = 's', long = "skip-updated", action = clap::ArgAction::SetTrue)]
    pub(crate) skip_updated: bool,

    /// Update manifest to specific version (skip version detection)
    #[arg(short = 'V', long)]
    pub(crate) version: Option<String>,

    /// Request timeout in seconds
    #[arg(short = 't', long, default_value = "30")]
    pub(crate) timeout: u64,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let inner = checkver::Args {
        dir: args.dir,
        app: args.app,
        update: args.update,
        force_update: args.force_update,
        skip_updated: args.skip_updated,
        version: args.version,
        timeout: args.timeout,
    };
    checkver::execute(inner, session)?;
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
