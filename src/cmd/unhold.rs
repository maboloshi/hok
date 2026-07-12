use clap::{ArgAction, Parser};
use libscoop::{operation, Session};

use crate::{output, Result};

/// Unhold package(s) to enable changes
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// The package(s) to be unheld
    #[arg(required = true, action = ArgAction::Append)]
    package: Vec<String>,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let packages = args.package.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    for name in packages {
        print!("Unholding {}...", name);
        match operation::package_hold(session, name, false) {
            Ok(..) => output::ok(),
            Err(err) => {
                output::err("Err");
                return Err(err.into());
            }
        }
    }
    Ok(())
}
