use clap::Parser;
use crossterm::style::Stylize;
use libscoop::{operation, SyncOption, Session};

use crate::Result;

/// Import installed packages from a file
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// JSON file to import from
    file: String,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let content = match std::fs::read_to_string(&args.file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading '{}': {}", args.file, e);
            return Ok(());
        }
    };

    let root: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error parsing JSON: {}", e);
            return Ok(());
        }
    };

    let mut packages = Vec::new();

    if let Some(buckets) = root.get("buckets").and_then(|v| v.as_object()) {
        for (bucket, apps) in buckets {
            if let Some(apps) = apps.as_object() {
                for (name, _version) in apps {
                    packages.push(format!("{}/{}", bucket, name));
                }
            }
        }
    }

    if packages.is_empty() {
        println!("{}", "No packages found in import file.".yellow());
        return Ok(());
    }

    println!("{}", format!("Found {} packages to install.", packages.len()).green());

    let queries: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
    let options = vec![SyncOption::AssumeYes];

    match operation::package_sync(session, queries, options) {
        Ok(_) => println!("{}", "Import complete.".green()),
        Err(e) => eprintln!("Import error: {}", e),
    }

    Ok(())
}
