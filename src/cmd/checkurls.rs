use clap::Parser;
use crossterm::style::Stylize;
use std::path::PathBuf;

use crate::Result;

/// Check manifest URLs for validity
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = ".")]
    dir: PathBuf,

    /// Specific app(s) to check (supports wildcards, default: all)
    #[arg(default_value = "*")]
    app: Vec<String>,

    /// Request timeout in seconds
    #[arg(short = 't', long, default_value = "30")]
    timeout: u64,

    /// Only show invalid URLs (suppress valid ones)
    #[arg(short = 's', long, action = clap::ArgAction::SetTrue)]
    skip_valid: bool,
}

pub fn execute(args: Args) -> Result<()> {
    let dir = &args.dir;
    if !dir.is_dir() {
        eprintln!("error: '{}' is not a directory", dir.display());
        return Ok(());
    }

    let mut total_urls = 0u32;
    let mut valid = 0u32;
    let mut invalid = 0u32;

    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }

        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        if args.app[0] != "*" && !args.app.iter().any(|p| name.contains(p.as_str())) {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let manifest: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let urls = extract_urls(&manifest);
        if urls.is_empty() {
            continue;
        }

        if !args.skip_valid {
            print!("{} ... ", name);
        }

        let mut all_valid = true;
        for url in &urls {
            total_urls += 1;
            match check_url(url, args.timeout) {
                Ok(true) => {
                    valid += 1;
                }
                Ok(false) => {
                    invalid += 1;
                    all_valid = false;
                    if !args.skip_valid {
                        println!("\n  {} {} ({})", "✗".red(), "not found".yellow(), url);
                    } else {
                        println!("  {} {} ({})", "✗".red(), name, url);
                    }
                }
                Err(e) => {
                    invalid += 1;
                    all_valid = false;
                    if !args.skip_valid {
                        println!("\n  {} {} ({})", "✗".red(), e, url);
                    } else {
                        println!("  {} {} {} ({})", "✗".red(), name, e, url);
                    }
                }
            }
        }

        if all_valid && !args.skip_valid {
            println!("{} ({} urls)", "ok".green(), urls.len());
        }
    }

    println!(
        "\n{}",
        format!("Checked {} URLs: {} valid, {} invalid.", total_urls, valid, invalid).yellow()
    );

    Ok(())
}

/// Extract all download URLs from a manifest (arch-specific + top-level).
fn extract_urls(manifest: &serde_json::Value) -> Vec<String> {
    let mut result = Vec::new();

    if let Some(arch) = manifest.get("architecture") {
        for key in &["64bit", "32bit", "arm64"] {
            if let Some(cfg) = arch.get(*key) {
                result.extend(get_urls_from_node(cfg));
            }
        }
    }

    if result.is_empty() {
        result.extend(get_urls_from_node(manifest));
    }

    result
}

fn get_urls_from_node(node: &serde_json::Value) -> Vec<String> {
    let mut urls = Vec::new();
    if let Some(url_field) = node.get("url") {
        match url_field {
            serde_json::Value::String(s) => urls.push(strip_fragment(s)),
            serde_json::Value::Array(arr) => {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        urls.push(strip_fragment(s));
                    }
                }
            }
            _ => {}
        }
    }
    urls
}

/// Strip `#/dl.7z` fragment used for URL renaming.
fn strip_fragment(url: &str) -> String {
    url.split('#').next().unwrap_or(url).to_string()
}

/// Check if a URL is accessible via HTTP HEAD.
fn check_url(url: &str, timeout_secs: u64) -> Result<bool> {
    let output = std::process::Command::new("curl.exe")
        .args([
            "-sI",
            "-o", "NUL",
            "-w", "%{http_code}",
            "--connect-timeout", &timeout_secs.to_string(),
            "--max-time", &timeout_secs.to_string(),
            url,
        ])
        .output()
        .map_err(|e| anyhow::anyhow!("curl error: {}", e))?;

    let code_str = String::from_utf8_lossy(&output.stdout);
    let code: u16 = code_str.trim().parse().unwrap_or(0);

    match code {
        200..=399 => Ok(true),
        0 => Err(anyhow::anyhow!("connection failed")),
        _c => Ok(false), // 404, 403, etc. are "not valid" but not errors
    }
}
