use clap::Parser;
use libscoop::{operation, Manifest, Session};
use std::path::PathBuf;

use crate::{output, Result};

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

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let dir = &args.dir;
    if !dir.is_dir() {
        output::err(format!("error: '{}' is not a directory", dir.display()));
        return Ok(());
    }

    let _proxy = session.config().proxy().map(|s| s.to_owned());

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

        let manifest = match Manifest::parse(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let urls: Vec<String> = manifest
            .url()
            .into_iter()
            .map(|u| u.split('#').next().unwrap_or(u).to_string())
            .collect();

        if urls.is_empty() {
            continue;
        }

        if !args.skip_valid {
            print!("{} ... ", name);
        }

        let mut all_valid = true;
        for url in &urls {
            total_urls += 1;
            match operation::head_url(session, url, args.timeout) {
                Ok(true) => valid += 1,
                Ok(false) => {
                    invalid += 1;
                    all_valid = false;
                    if !args.skip_valid {
                        output::err(format!("not found: {url}"));
                    } else {
                        output::named(&name, url);
                    }
                }
                Err(e) => {
                    invalid += 1;
                    all_valid = false;
                    if !args.skip_valid {
                        output::err(format!("{e}: {url}"));
                    } else {
                        output::err(format!("{name} {e} ({url})"));
                    }
                }
            }
        }

        if all_valid && !args.skip_valid {
            println!("ok ({} urls)", urls.len());
        }
    }

    output::info(format!("Checked {total_urls} URLs: {valid} valid, {invalid} invalid."));

    Ok(())
}
