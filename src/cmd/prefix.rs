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
    // Upstream `scoop prefix` prints `currentdir $app` (libexec/scoop-prefix.ps1),
    // which under NO_JUNCTION is the resolved version dir — there is no
    // `current` junction to print.
    let app_dir = session.app_current_dir(&args.package);

    let Some(app_dir) = app_dir.filter(|dir| dir.exists()) else {
        // Check if the app directory exists at all (without /current)
        let base_dir = session.app_dir(&args.package);
        let msg = if base_dir.exists() {
            hok_i18n::t!("cmd.prefix_installed", pkg = args.package)
        } else {
            hok_i18n::t!("cmd.prefix_no_pkg", pkg = args.package)
        };
        return Err(anyhow::anyhow!(msg));
    };

    println!("{}", app_dir.display());
    Ok(())
}
