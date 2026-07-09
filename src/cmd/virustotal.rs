use clap::Parser;
use crossterm::style::Stylize;
use libscoop::{operation, QueryOption, Session};

use crate::Result;

/// Check a package's download for viruses (requires API key)
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// Package name(s) to check
    #[arg(action = clap::ArgAction::Append)]
    app: Vec<String>,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let queries: Vec<&str> = args.app.iter().map(|s| s.as_str()).collect();
    let options = vec![QueryOption::Explicit];
    let pkgs = operation::package_query(session, queries, options, false)?;

    if pkgs.is_empty() {
        eprintln!("No packages found.");
        return Ok(());
    }

    for pkg in &pkgs {
        let urls = pkg.manifest().url();
        if urls.is_empty() {
            println!("  {}: {}", pkg.name().blue(), "no download URLs".yellow());
            continue;
        }
        println!("  {}: {} URL(s)", pkg.name().blue(), urls.len());
    }

    println!("{}", "VirusTotal scanning is not yet implemented.".yellow());
    println!("Set an API key with: hok config gh_token <token>");

    Ok(())
}
