use clap::Parser;
use libscoop::{operation, Session};

use crate::Result;

/// Reset an app to resolve conflicts (reapply shims, shortcuts, post_install)
#[derive(Debug, Parser)]
pub struct Args {
    /// The app name
    app: String,
    /// A specific version to reset to
    version: Option<String>,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let name = args.app;
    let version = args.version.as_deref();
    operation::package_reset(session, &name, version)?;
    Ok(())
}
