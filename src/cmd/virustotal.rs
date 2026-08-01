use clap::Parser;
use libscoop::{package, QueryOption, Session};

use crate::{output, Result};

/// Check a package's download URL against VirusTotal
///
/// Requires a VirusTotal API key. Set it with:
///   hok config virustotal_api_key <key>
/// Or set the $VT_API_KEY environment variable.
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
    let pkgs = package::query::query(session, queries, options, false)?;

    if pkgs.is_empty() {
        output::err(rust_i18n::t!("cmd.no_pkgs_found"));
        return Ok(());
    }

    // Get API key: env var > config > none
    let api_key = std::env::var("VT_API_KEY").ok();

    for pkg in &pkgs {
        let urls = pkg.manifest().url();
        if urls.is_empty() {
            output::named(pkg.name(), rust_i18n::t!("cmd.no_download_urls"));
            continue;
        }

        let url = urls[0].split('#').next().unwrap_or(urls[0]);
        print!("  {}: {} ... ", pkg.name(), url);

        if let Some(key) = &api_key {
            match package::virustotal::check_url(url, key) {
                Ok(stats) => {
                    if stats.malicious > 0 {
                        output::err(format!("MALICIOUS {}/{} engines flagged", stats.malicious, stats.total));
                    } else if stats.suspicious > 0 {
                        output::warn(format!("SUSPICIOUS {}/{} suspicious", stats.suspicious, stats.total));
                    } else {
                        output::info(format!("OK {}/{} clean", stats.total - stats.harmless, stats.total));
                    }
                }
                Err(e) => output::err(format!("{e}")),
            }
        } else {
            output::warn(rust_i18n::t!("cmd.vt_skip"));
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
