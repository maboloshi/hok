//! Look for malware using VirusTotal.

use clap::Parser;
use libscoop::{QueryOption, Session, package};

use crate::output::style;
use crate::{ExitCode, Result, output};

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
        return Err(anyhow::anyhow!(hok_i18n::t!("cmd.no_pkgs_found")));
    }

    // Get API key: env var > config > none
    let api_key = std::env::var("VT_API_KEY")
        .ok()
        .or_else(|| session.config().virustotal_api_key.clone());

    // Scoop aborts with exit 16 when no API key is configured (it cannot do
    // any work); mirror that instead of silently skipping every package.
    if api_key.is_none() {
        output::err(hok_i18n::t!("cmd.vt_no_key"));
        return Err(ExitCode(16).into());
    }

    // Scoop combines result bits: 2 = at least one unsafe (malicious or
    // suspicious) finding, 4 = at least one request exception.
    let mut exit_code: u32 = 0;

    for pkg in &pkgs {
        let urls = pkg.manifest().url();
        if urls.is_empty() {
            output::named(pkg.name(), hok_i18n::t!("cmd.no_download_urls"));
            continue;
        }

        let Some(key) = &api_key else {
            // Unreachable: the no-key guard above already returned; a package
            // without URLs was skipped. Kept to satisfy the Option binding.
            continue;
        };
        let url = urls[0].split('#').next().unwrap_or(urls[0]);
        match package::virustotal::check_url(url, key).await {
            // A finding is result *data*: print it to stdout (colour-coded by
            // severity) so redirecting stdout keeps the full report, and set the
            // Scoop "unsafe" exit bit (2) for automation/CI. The name is
            // emphasized, the verdict carries the severity role.
            Ok(stats) if stats.malicious > 0 => {
                exit_code |= 2;
                println!(
                    "  {}: {}",
                    style::emphasis(pkg.name()),
                    style::danger(hok_i18n::t!(
                        "cmd.vt_malicious",
                        n = stats.malicious,
                        total = stats.total
                    ))
                );
            }
            Ok(stats) if stats.suspicious > 0 => {
                exit_code |= 2;
                println!(
                    "  {}: {}",
                    style::emphasis(pkg.name()),
                    style::warning(hok_i18n::t!(
                        "cmd.vt_suspicious",
                        n = stats.suspicious,
                        total = stats.total
                    ))
                );
            }
            Ok(stats) => println!(
                "  {}: {}",
                style::emphasis(pkg.name()),
                style::success(hok_i18n::t!("cmd.vt_clean", total = stats.total))
            ),
            // A request exception is a real failure (Scoop bit 4).
            Err(e) => {
                exit_code |= 4;
                output::err(format!("{}: {}", pkg.name(), e));
            }
        }
    }

    if exit_code != 0 {
        return Err(ExitCode(exit_code as i32).into());
    }

    Ok(())
}
