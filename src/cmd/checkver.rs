use clap::Parser;
use crossterm::style::Stylize;
use libscoop::{operation, Manifest, Session};
use regex::Regex;
use std::path::PathBuf;

use crate::Result;

/// Check manifest for a newer version
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = ".")]
    dir: PathBuf,

    /// Specific app(s) to check (supports wildcards, default: all)
    #[arg(default_value = "*")]
    app: Vec<String>,

    /// Update manifest with new version and trigger autoupdate
    #[arg(short = 'u', long, action = clap::ArgAction::SetTrue)]
    update: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let dir = &args.dir;
    if !dir.is_dir() {
        eprintln!("error: '{}' is not a directory", dir.display());
        return Ok(());
    }

    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }

        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        if args.app[0] != "*" && !args.app.iter().any(|p| stem.contains(p.as_str())) {
            continue;
        }

        let manifest = match Manifest::parse(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let cv = match manifest.checkver() {
            Some(c) => c,
            None => continue,
        };

        print!("{} ... ", stem);

        // Determine URL and regex to use
        let url = match &cv.url {
            Some(u) => u.clone(),
            None if cv.sourceforge.is_some() => {
                println!("{}", "sourceforge checkver not supported".yellow());
                continue;
            }
            None => {
                println!("{}", "no checkver url".yellow());
                continue;
            }
        };

        // Fetch page content
        let raw = match operation::download_page(session, &url) {
            Ok(t) => t,
            Err(e) => {
                println!("{}: {}", "fetch error".red(), e);
                continue;
            }
        };

        // Extract version
        let current = manifest.version().to_string();
        let latest = extract_version(&raw, cv);

        match latest {
            Some(ver) if ver == current => {
                println!("{} ({})", "up to date".green(), ver);
            }
            Some(ref ver) => {
                println!("{} {} -> {}", "update available".yellow(), current, ver.as_str().blue());
                if args.update {
                    update_manifest_version(&path, ver)?;
                    println!("  {} updated to {}", "✓".green(), ver);
                }
            }
            None => {
                println!("{}", "could not extract version".red());
            }
        }
    }

    Ok(())
}

/// Extract version string from page content using checkver rules.
fn extract_version(content: &str, cv: &libscoop::Checkver) -> Option<String> {
    if let Some(regex_str) = &cv.regex {
        let re = Regex::new(regex_str).ok()?;
        let caps = re.captures(content)?;
        let ver = caps.get(1).or_else(|| caps.get(0))?.as_str().to_string();
        return Some(ver);
    }

    // No regex: treat content itself as version string
    let trimmed = content.trim();
    if !trimmed.is_empty() {
        Some(trimmed.to_string())
    } else {
        None
    }
}

/// Update the `version` field in a manifest JSON file.
fn update_manifest_version(path: &PathBuf, new_version: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let mut root: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| anyhow::anyhow!("parse: {}", e))?;

    if let Some(v) = root.get_mut("version") {
        if let Some(s) = v.as_str() {
            // Only update if version actually changed
            if s != new_version {
                *v = serde_json::Value::String(new_version.to_string());
                let formatted = serde_json::to_string_pretty(&root)
                    .map_err(|e| anyhow::anyhow!("serialize: {}", e))?;
                std::fs::write(path, formatted.as_bytes())?;
            }
        }
    }
    Ok(())
}
