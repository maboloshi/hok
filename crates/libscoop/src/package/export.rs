//! Export installed packages to a portable JSON document.
//!
//! Builds the export structure consumed by [`crate::package::import`] — the
//! business logic behind the `hok export` command.
//!
//! # Format
//!
//! ```json
//! {
//!   "buckets": { "main": "https://github.com/ScoopInstaller/Main" },
//!   "apps": { "main": { "7zip": "24.09" } }
//! }
//! ```
//!
//! Isolated packages (installed via URL or path, not managed by any bucket)
//! are grouped under the [`crate::constant::ISOLATED_PACKAGE_BUCKET`] key and
//! are omitted unless `include_all` is set.

use serde_json::{Map, Value};

use crate::{bucket, constant::ISOLATED_PACKAGE_BUCKET, error::Fallible, Session};

/// Build the export JSON document for all installed packages.
///
/// # Errors
///
/// Returns an error if listing buckets or querying installed packages fails.
pub fn build_export(session: &Session, include_all: bool) -> Fallible<Value> {
    let mut output = Map::new();

    // 1. Buckets: name → remote_url
    let buckets = bucket::list(session)?;
    let mut bucket_map = Map::new();
    for bucket in &buckets {
        if let Some(url) = bucket.remote_url() {
            bucket_map.insert(bucket.name().to_string(), Value::String(url.to_string()));
        }
    }
    output.insert("buckets".to_string(), Value::Object(bucket_map));

    // 2. Apps: bucket → { name → version }
    let pkgs = super::query::query(session, vec!["*"], vec![], true)?;

    let mut bucket_apps = Map::new();
    for pkg in &pkgs {
        let bucket = pkg.installed_bucket().unwrap_or(ISOLATED_PACKAGE_BUCKET);
        if bucket == ISOLATED_PACKAGE_BUCKET && !include_all {
            continue;
        }
        let version = pkg.installed_version().unwrap_or("unknown");

        let entry = bucket_apps
            .entry(bucket.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(map) = entry {
            map.insert(pkg.name().to_string(), Value::String(version.to_string()));
        }
    }
    output.insert("apps".to_string(), Value::Object(bucket_apps));

    Ok(Value::Object(output))
}
