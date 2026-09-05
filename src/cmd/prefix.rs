//! Return the install path of a package.

use clap::Parser;
use libscoop::Session;

use crate::Result;

/// Show the directory where a package is installed
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// Name of the package
    package: String,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let app_dir = session.current_dir(&args.package);

    if !app_dir.exists() {
        // Check if the app directory exists at all (without /current)
        let base_dir = session.app_dir(&args.package);
        let msg = if base_dir.exists() {
            rust_i18n::t!("cmd.prefix_installed", pkg = args.package)
        } else {
            rust_i18n::t!("cmd.prefix_no_pkg", pkg = args.package)
        };
        return Err(anyhow::anyhow!(msg));
    }

    println!("{}", app_dir.display());
    Ok(())
}
