//! Unhold a package to allow upgrades.
//!
//! Thin shell over [`crate::cmd::hold::hold_packages`] with `flag = false` —
//! the batch loop is shared with the `hold` command (see `hold.rs`).

use clap::{ArgAction, Parser};
use libscoop::Session;

use crate::cmd::hold::hold_packages;
use crate::Result;

/// Unhold package(s) to enable changes
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// The package(s) to be unheld
    #[arg(required = true, action = ArgAction::Append)]
    package: Vec<String>,
    /// Unhold globally installed app (from $SCOOP_GLOBAL)
    #[arg(short = 'g', long, action = ArgAction::SetTrue)]
    global: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    ensure_global(session, args.global, "unhold")?;
    hold_packages(session, &args.package, false, rust_i18n::t!("cmd.unholding"))
}

use crate::cmd::shared_args::{ensure_global, Cmd};

impl Cmd for Args {
    type Args = Self;

    #[inline]
    fn execute(args: Self::Args, session: &Session) -> Result<()> {
        execute(args, session)
    }
}
