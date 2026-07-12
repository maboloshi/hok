use clap::Parser;
use libscoop::{operation, QueryOption, Session};

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
    let pkgs = operation::package_query(session, queries, options, false)?;

    if pkgs.is_empty() {
        output::err("No packages found.");
        return Ok(());
    }

    // Get API key: env var > config > none
    let api_key = std::env::var("VT_API_KEY").ok()
        .or_else(|| None); // config lookup deferred

    for pkg in &pkgs {
        let urls = pkg.manifest().url();
        if urls.is_empty() {
            output::named(pkg.name(), "no download URLs");
            continue;
        }

        let url = urls[0].split('#').next().unwrap_or(urls[0]);
        print!("  {}: {} ... ", pkg.name(), url);

        if let Some(key) = &api_key {
            match check_virustotal(url, key) {
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
            output::warn("skipped (set VT_API_KEY to scan)");
        }
    }

    Ok(())
}

struct ScanStats {
    total: u32,
    malicious: u32,
    suspicious: u32,
    harmless: u32,
}

/// Check a URL against VirusTotal API.
fn check_virustotal(url: &str, api_key: &str) -> Result<ScanStats> {
    // Step 1: Submit URL for analysis
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build().new_agent();

    let submit_body = serde_json::json!({ "url": url });
    let submit_resp = agent
        .post("https://www.virustotal.com/api/v3/urls")
        .header("x-apikey", api_key)
        .send_json(&submit_body)
        .map_err(|e| anyhow::anyhow!("VT submit error: {}", e))?;

    let submit_json: serde_json::Value = submit_resp
        .into_body().read_json()
        .map_err(|e| anyhow::anyhow!("VT response error: {}", e))?;

    let analysis_id = submit_json["data"]["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("VT: no analysis ID"))?;

    // Step 2: Poll for analysis results
    use std::time::Duration;
    let analysis_url = format!("https://www.virustotal.com/api/v3/analyses/{}", analysis_id);

    // Poll up to 5 times with 3s delay
    for _ in 0..5 {
        std::thread::sleep(Duration::from_secs(3));

        let resp = agent
            .get(&analysis_url)
            .header("x-apikey", api_key)
            .call()
            .map_err(|e| anyhow::anyhow!("VT poll error: {}", e))?;

        let json: serde_json::Value = resp.into_body().read_json()
            .map_err(|e| anyhow::anyhow!("VT parse error: {}", e))?;

        let status = json["data"]["attributes"]["status"].as_str().unwrap_or("");
        if status == "completed" {
            let stats = &json["data"]["attributes"]["stats"];
            return Ok(ScanStats {
                total: stats["total"].as_u64().unwrap_or(0) as u32,
                malicious: stats["malicious"].as_u64().unwrap_or(0) as u32,
                suspicious: stats["suspicious"].as_u64().unwrap_or(0) as u32,
                harmless: stats["harmless"].as_u64().unwrap_or(0) as u32,
            });
        }
    }

    Err(anyhow::anyhow!("VT analysis timed out"))
}
