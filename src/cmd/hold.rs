use clap::{ArgAction, Parser};
use libscoop::{operation, Session};

use crate::{output, Result};

/// Hold package(s) to disable changes
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// The package(s) to be held
    #[arg(required= true, action = ArgAction::Append)]
    package: Vec<String>,
    /// Hold globally installed app (from $SCOOP_GLOBAL)
    #[arg(short = 'g', long, action = ArgAction::SetTrue)]
    global: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    session.set_global(args.global);
    for name in &args.package {
        output::progress(rust_i18n::t!("cmd.holding"), name);
        match operation::package_hold(session, name, true) {
            Ok(..) => output::ok(),
            Err(err) => {
                output::err(rust_i18n::t!("cmd.hold_err"));
                return Err(err.into());
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
