//! Open the homepage of a package.

use clap::Parser;
use libscoop::Session;

use crate::Result;

/// Browse the homepage of a package
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// The package name
    package: String,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let query = args.package;

    // Source resolution (URL / UNC / local file / installed record / bucket)
    // is `Get-Manifest`, shared with `cat`.
    let resolved = super::resolve_manifest_query(
        session,
        &query,
        |_| hok_i18n::t!("cmd.home_multiple", query = query).to_string(),
        hok_i18n::t!("cmd.home_not_found", query = query).to_string(),
        hok_i18n::t!("cmd.home_invalid").to_string(),
    )
    .await?;

    // Upstream aborts when the manifest carries no homepage
    // (libexec/scoop-home.ps1:12-16). This must be checked *before* the shell
    // open: `ShellExecuteW` reports success for an empty string, so an empty
    // homepage used to exit 0 without opening anything.
    let homepage = resolved.manifest.homepage();
    if homepage.is_empty() {
        return Err(anyhow::anyhow!(hok_i18n::t!(
            "cmd.home_no_homepage",
            pkg = query
        )));
    }

    libscoop::os::open_url(homepage)?;
    Ok(())
}
