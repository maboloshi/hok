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
    // Upstream `scoop prefix` prints `currentdir $app` (libexec/scoop-prefix.ps1:12),
    // trying the local root first and then the global one — so a globally
    // installed app is found without `-g`. Under NO_JUNCTION the resolved
    // version dir is printed; there is no `current` junction to print.
    let found = [false, true].into_iter().find_map(|global| {
        session
            .app_current_dir_in(&args.package, global)
            .filter(|dir| dir.exists())
    });

    let Some(app_dir) = found else {
        // One message for "not installed", "broken install" and "installed
        // without a current link" alike (upstream's single `abort`).
        return Err(anyhow::anyhow!(hok_i18n::t!(
            "cmd.prefix_not_found",
            pkg = args.package
        )));
    };

    println!("{}", app_dir.display());
    Ok(())
}
