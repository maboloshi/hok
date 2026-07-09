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

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let config = session.config();
    let app_dir = config.root_path().join("apps").join(&args.package).join("current");

    if !app_dir.exists() {
        // Check if the app directory exists at all (without /current)
        let base_dir = config.root_path().join("apps").join(&args.package);
        if base_dir.exists() {
            eprintln!("Package '{}' is installed but has no 'current' symlink.", args.package);
        } else {
            eprintln!("Package '{}' is not installed.", args.package);
        }
        return Ok(());
    }

    println!("{}", app_dir.display());
    Ok(())
}
