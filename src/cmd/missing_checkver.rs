use clap::Parser;
use crossterm::style::Stylize;
use std::path::PathBuf;

use crate::Result;

/// Check bucket manifests missing checkver and autoupdate
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = ".")]
    dir: PathBuf,

    /// Only show manifests that have checkver/autoupdate (inverse)
    #[arg(short = 's', long, action = clap::ArgAction::SetTrue)]
    supported: bool,
}

pub fn execute(args: Args) -> Result<()> {
    let dir = &args.dir;
    if !dir.is_dir() {
        eprintln!("error: '{}' is not a directory", dir.display());
        return Ok(());
    }

    let mut total = 0u32;
    let mut missing_checkver = 0u32;
    let mut missing_autoupdate = 0u32;

    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let value: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let has_checkver = value.get("checkver").is_some();
        let has_autoupdate = value.get("autoupdate").is_some();
        total += 1;

        if args.supported {
            // Only show manifests that DO have checkver/autoupdate
            if has_checkver || has_autoupdate {
                println!("{} {}", "✓".green(), name);
            }
        } else {
            // Default: show what's missing
            let mut issues = Vec::new();
            if !has_checkver {
                issues.push("checkver".to_string());
                missing_checkver += 1;
            }
            if !has_autoupdate {
                issues.push("autoupdate".to_string());
                missing_autoupdate += 1;
            }
            if !issues.is_empty() {
                println!("  {} {} ({})", "✗".red(), name, issues.join(", "));
            }
        }
    }

    if !args.supported {
        println!(
            "\n{}",
            format!(
                "Scanned {} manifests: {} missing checkver, {} missing autoupdate.",
                total, missing_checkver, missing_autoupdate
            )
            .yellow()
        );
        if missing_checkver == 0 && missing_autoupdate == 0 {
            println!("{}", "All manifests have checkver and autoupdate.".green());
        }
    }

    Ok(())
}
