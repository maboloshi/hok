use clap::Parser;
use libscoop::{operation, SyncOption, Session};

use crate::{output, Result};

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
            output::err(format!("Error reading '{}': {}", args.file, e));
            return Ok(());
        }
    };

    let root: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            output::err(format!("Error parsing JSON: {}", e));
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
        output::warn("No packages found in import file.");
        return Ok(());
    }

    output::info(format!("Found {} packages to install.", packages.len()));

    let queries: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
    let options = vec![SyncOption::AssumeYes];

    match operation::package_sync(session, queries, options) {
        Ok(_) => output::info("Import complete."),
        Err(e) => output::err(format!("Import error: {}", e)),
    }

    Ok(())
}
