use clap::Parser;
use libscoop::{operation, Session};

use crate::Result;

/// Export installed packages list
#[derive(Debug, Parser)]
pub struct Args {
    /// Include non-bucket packages (URL/path installs)
    #[arg(short, long, action = clap::ArgAction::SetTrue)]
    all: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let mut output = serde_json::Map::new();

    // 1. Buckets: name → remote_url
    let buckets = operation::bucket_list(session)?;
    let mut bucket_map = serde_json::Map::new();
    for b in &buckets {
        if let Some(url) = b.remote_url() {
            bucket_map.insert(b.name().to_string(), serde_json::Value::String(url.to_string()));
        }
    }
    output.insert("buckets".to_string(), serde_json::Value::Object(bucket_map));

    // 2. Apps: bucket → { name → version }
    let queries = vec!["*"];
    let pkgs = operation::package_query(session, queries, vec![], true)?;

    let mut bucket_apps = serde_json::Map::new();
    for pkg in &pkgs {
        let bucket = pkg.installed_bucket().unwrap_or("__isolated__");
        if bucket == "__isolated__" && !args.all {
            continue;
        }
        let version = pkg.installed_version().unwrap_or("unknown");

        let entry = bucket_apps.entry(bucket.to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let serde_json::Value::Object(map) = entry {
            map.insert(pkg.name().to_string(), serde_json::Value::String(version.to_string()));
        }
    }
    output.insert("apps".to_string(), serde_json::Value::Object(bucket_apps));

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
    Ok(())
}
