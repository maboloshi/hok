//! Reset a package to its current version.

use clap::Parser;
use libscoop::{package, Session};

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
    package::sync::reset(session, &name, version)?;
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
