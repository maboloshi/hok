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
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    for name in &args.package {
        print!("Holding {}...", name);
        match operation::package_hold(session, name, true) {
            Ok(..) => output::ok(),
            Err(err) => {
                output::err("Err");
                return Err(err.into());
            }
        }
    }
    Ok(())
}
