//! Import installed packages from an exported JSON file.
//!
//! Parses the export format produced by [`crate::package::export`] and turns
//! it into a list of `bucket/name` queries that can be fed to a sync
//! operation — the business logic behind the `hok import` command.
//!
//! # Format
//!
//! The canonical export format nests applications under the `"apps"` key:
//!
//! ```json
//! {
//!   "buckets": { "main": "https://github.com/ScoopInstaller/Main" },
//!   "apps": { "main": { "7zip": "24.09" } }
//! }
//! ```
//!
//! For compatibility, files written by older versions that placed the apps
//! under the `"buckets"` key are still accepted as a fallback.

use serde_json::Value;

use crate::error::Fallible;

/// Parse an exported JSON document into a list of `bucket/name` queries.
///
/// # Errors
///
/// Returns an error if the content is not valid JSON.
pub fn parse_import_json(content: &str) -> Fallible<Vec<String>> {
    let root: Value = serde_json::from_str(content)?;

    let mut packages = Vec::new();
    for key in ["apps", "buckets"] {
        if let Some(buckets) = root.get(key).and_then(Value::as_object) {
            for (bucket, apps) in buckets {
                if let Some(apps) = apps.as_object() {
                    for name in apps.keys() {
                        packages.push(format!("{bucket}/{name}"));
                    }
                }
            }
        }
        if !packages.is_empty() {
            break;
        }
    }
    Ok(packages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_object_yields_no_packages() {
        assert_eq!(parse_import_json("{}").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn parse_canonical_apps_format() {
        let content = r#"{
            "buckets": { "main": "https://github.com/ScoopInstaller/Main" },
            "apps": { "main": { "7zip": "24.09", "aria2": "1.37.0" } }
        }"#;
        let packages = parse_import_json(content).unwrap();
        assert_eq!(packages, vec!["main/7zip", "main/aria2"]);
    }

    #[test]
    fn parse_legacy_buckets_format() {
        let content = r#"{ "buckets": { "extras": { "everything": "1.4.1" } } }"#;
        let packages = parse_import_json(content).unwrap();
        assert_eq!(packages, vec!["extras/everything"]);
    }

    #[test]
    fn parse_apps_takes_precedence_over_buckets() {
        let content = r#"{
            "buckets": { "main": { "old": "1.0" } },
            "apps": { "main": { "new": "2.0" } }
        }"#;
        let packages = parse_import_json(content).unwrap();
        assert_eq!(packages, vec!["main/new"]);
    }

    #[test]
    fn parse_non_object_apps_is_skipped() {
        let content = r#"{ "apps": { "main": "not-an-object" } }"#;
        assert_eq!(parse_import_json(content).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn parse_invalid_json_errors() {
        assert!(parse_import_json("{ not json").is_err());
    }
}
