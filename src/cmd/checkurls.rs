use clap::Parser;
use libscoop::{operation, Manifest, Session};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::{output, util, Result};

/// Check manifest URLs for validity
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = ".")]
    dir: PathBuf,

    /// Specific app(s) to check (supports wildcards, default: all)
    #[arg(default_value = "*")]
    app: Vec<String>,

    /// Request timeout in seconds (Scoop default: 5)
    #[arg(short = 't', long, default_value = "5")]
    timeout: u64,

    /// Only show invalid URLs (suppress valid ones)
    #[arg(short = 's', long, action = clap::ArgAction::SetTrue)]
    skip_valid: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let dir = &args.dir;
    if !dir.is_dir() {
        output::err(rust_i18n::t!("cmd.checkurls_err_dir", path = dir.display()));
        return Ok(());
    }

    // Scoop-style header: [U]RLs [O]kay [F]ailed
    // The ps1 prints: '[' + 'U' (cyan) + ']RLs | [' + 'O' (green) + ']kay |  | [' + 'F' (red) + ']ailed |  |  |'
    use crossterm::style::Stylize;
    eprintln!(
        "{} {} {}",
        format!("[{}]", "U".cyan()).to_string(),
        format!("RLs   [{}]", "O".green()).to_string(),
        format!("[{}]", "F".red()).to_string(),
    );
    eprintln!();

    let mut total_manifests = 0u32;
    let mut total_urls = 0u32;
    let mut total_valid = 0u32;
    let mut total_invalid = 0u32;

    // Recursively collect all .json files
    let manifest_paths: Vec<PathBuf> = util::walkdir_files(dir);

    for path in &manifest_paths {
        if path.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }

        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        if args.app[0] != "*" && !args.app.iter().any(|p| name.contains(p.as_str())) {
            continue;
        }

        let manifest = match Manifest::parse(path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        // Collect ALL URLs (noarch + all architectures), matching Scoop's behavior:
        // - If manifest.url exists → use those; otherwise collect from architecture specs
        let raw_urls: Vec<String> = if manifest.url().is_empty() {
            // Scoop: script:url $manifest '64bit', '32bit', 'arm64'
            manifest.all_urls().into_iter().map(|s| s.to_string()).collect()
        } else {
            manifest.url().into_iter().map(|s| s.to_string()).collect()
        };

        // Scoop: Trim renaming suffix (#/filename) to prevent 40x responses
        let urls: Vec<String> = raw_urls
            .iter()
            .map(|u| u.split('#').next().unwrap_or(u).to_string())
            .collect();

        if urls.is_empty() {
            continue;
        }

        total_manifests += 1;

        let manifest_cookies: Option<HashMap<String, String>> =
            manifest.cookie().map(|c| c.clone());

        let mut ok_count = 0u32;
        let mut failed_count = 0u32;
        let mut errors: Vec<String> = Vec::new();

        for url in &urls {
            total_urls += 1;
            let result = operation::head_url_ext(session, url, args.timeout,
                manifest_cookies.as_ref());

            match result.error {
                None => {
                    // Scoop check: $status -eq 'OK' -or $status -eq 'OpeningData'
                    // HTTP 2xx/3xx = OK. 3xx redirects are also OK (OpeningData for FTP)
                    ok_count += 1;
                    total_valid += 1;
                }
                Some(ref msg) => {
                    failed_count += 1;
                    total_invalid += 1;
                    errors.push(format!("{} ({})", msg, url));
                }
            }
        }

        if ok_count == urls.len() as u32 && args.skip_valid {
            continue;
        }

        // Scoop-style output: [urls][ok][failed] name
        eprint!("[{}]", urls.len().to_string().cyan());
        eprint!("[{}]",
            if ok_count == urls.len() as u32 {
                ok_count.to_string().green()
            } else if ok_count == 0 {
                ok_count.to_string().red()
            } else {
                ok_count.to_string().yellow()
            }
        );
        eprint!("[{}]",
            if failed_count == 0 {
                failed_count.to_string().green()
            } else {
                failed_count.to_string().red()
            }
        );
        eprintln!(" {}", name);

        // Print detailed errors (Scoop: dark red indented lines)
        for err in &errors {
            eprintln!("{} > {}", "       ".to_string(), err.clone().dark_red());
        }
    }

    if total_manifests == 0 {
        output::info(rust_i18n::t!("cmd.checkurls_no_manifests"));
    } else {
        output::info(rust_i18n::t!(
            "cmd.checkurls_summary",
            total = total_urls,
            valid = total_valid,
            invalid = total_invalid
        ));
    }

    Ok(())
}
