//! Look for malware using VirusTotal.

use clap::Parser;
use libscoop::{QueryOption, Session, package};

use crate::{Result, output};

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

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let queries: Vec<&str> = args.app.iter().map(|s| s.as_str()).collect();
    let options = vec![QueryOption::Explicit];
    let pkgs = package::query::query(session, queries, options, false)?;

    if pkgs.is_empty() {
        return Err(anyhow::anyhow!(rust_i18n::t!("cmd.no_pkgs_found")));
    }

    // Get API key: env var > config > none
    let api_key = std::env::var("VT_API_KEY")
        .ok()
        .or_else(|| session.config().virustotal_api_key.clone());

    for pkg in &pkgs {
        let urls = pkg.manifest().url();
        if urls.is_empty() {
            output::named(pkg.name(), rust_i18n::t!("cmd.no_download_urls"));
            continue;
        }

        if let Some(key) = &api_key {
            let url = urls[0].split('#').next().unwrap_or(urls[0]);
            match package::virustotal::check_url(url, key).await {
                Ok(stats) if stats.malicious > 0 => output::err(format!(
                    "{}: {}",
                    pkg.name(),
                    rust_i18n::t!("cmd.vt_malicious", n = stats.malicious, total = stats.total)
                )),
                Ok(stats) if stats.suspicious > 0 => output::warn(format!(
                    "{}: {}",
                    pkg.name(),
                    rust_i18n::t!(
                        "cmd.vt_suspicious",
                        n = stats.suspicious,
                        total = stats.total
                    )
                )),
                Ok(stats) => output::info(format!(
                    "{}: {}",
                    pkg.name(),
                    rust_i18n::t!("cmd.vt_clean", total = stats.total)
                )),
                Err(e) => output::err(format!("{}: {}", pkg.name(), e)),
            }
        } else {
            output::warn(rust_i18n::t!("cmd.vt_skip"));
        }
    }

    Ok(())
}
