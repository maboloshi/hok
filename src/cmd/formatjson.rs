use clap::Parser;
use crossterm::style::Stylize;
use std::path::PathBuf;

use crate::Result;

/// Format manifest JSON files in a bucket directory
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = ".")]
    dir: PathBuf,

    /// Specific app(s) to format (supports wildcards, default: all)
    #[arg(default_value = "*")]
    app: Vec<String>,
}

pub fn execute(args: Args) -> Result<()> {
    let dir = &args.dir;
    if !dir.is_dir() {
        eprintln!("error: '{}' is not a directory", dir.display());
        return Ok(());
    }

    let pattern = if args.app.is_empty() || args.app[0] == "*" {
        None
    } else {
        Some(args.app.iter().map(|s| s.as_str()).collect::<Vec<_>>())
    };

    let entries = std::fs::read_dir(dir)?;
    let mut count = 0u32;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }

        // Apply app filter
        if let Some(ref patterns) = pattern {
            let name = path.file_stem().unwrap().to_string_lossy();
            if !patterns.iter().any(|p| name.contains(*p)) {
                continue;
            }
        }

        // Read, validate, and reformat
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let value: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("  {}: {}", path.display(), format!("parse error: {}", e).red());
                continue;
            }
        };

        let formatted = serde_json::to_string_pretty(&value)
            .map_err(|e| anyhow::anyhow!("serialize error: {}", e))?;

        // Only write if the content changed
        if formatted != content {
            std::fs::write(&path, formatted.as_bytes())?;
            println!("  {} {}", format!("✓").green(), path.display());
            count += 1;
        }
    }

    if count == 0 {
        println!("{}", "No manifests needed formatting.".green());
    } else {
        println!("{} {} {}", "Formatted".green(), count, "manifest(s).");
    }

    Ok(())
}
