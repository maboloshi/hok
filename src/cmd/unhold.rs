use clap::{ArgAction, Parser};
use libscoop::{package, Session};

use crate::{output, Result};

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
    session.set_global(args.global);
    let packages = args.package.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    for name in packages {
        output::progress(rust_i18n::t!("cmd.unholding"), name);
        match package::hold::hold(session, name, false) {
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
