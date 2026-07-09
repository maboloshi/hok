use clap::Parser;
use libscoop::{operation, QueryOption, Session};

use crate::Result;

/// Export installed packages list
#[derive(Debug, Parser)]
pub struct Args {
    /// Include non-bucket packages (URL/path installs)
    #[arg(short, long, action = clap::ArgAction::SetTrue)]
    all: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let queries = vec!["*"];
    let options = vec![QueryOption::Upgradable];
    let pkgs = operation::package_query(session, queries, options, true)?;

    let mut output = serde_json::Map::new();
    let mut buckets = serde_json::Map::new();

    for pkg in &pkgs {
        let bucket = pkg.installed_bucket().unwrap_or("isolated");
        let version = pkg.installed_version().unwrap_or("unknown");

        let entry = buckets.entry(bucket.to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let serde_json::Value::Object(map) = entry {
            map.insert(pkg.name().to_string(), serde_json::Value::String(version.to_string()));
        }
    }

    output.insert("buckets".to_string(), serde_json::Value::Object(buckets));

    if args.all {
        // Also include URL/path installs
        let mut isolated = serde_json::Map::new();
        for pkg in &pkgs {
            if pkg.installed_bucket().is_none() {
                isolated.insert(pkg.name().to_string(), serde_json::Value::String(
                    pkg.installed_version().unwrap_or("unknown").to_string()
                ));
            }
        }
        if !isolated.is_empty() {
            output.insert("isolated".to_string(), serde_json::Value::Object(isolated));
        }
    }

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
    Ok(())
}
