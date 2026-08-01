//! List installed versions of a package.
//!
//! Scans the version directories under `<root>/apps/<name>/` and reports them
//! in descending version order, flagging the one the `current` link points to
//! — the business logic behind the `hok list --versions` flag.

use std::cmp::Ordering;

use crate::{error::Fallible, Session};

/// A single installed version directory of a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledVersion {
    /// The version directory name (e.g. `24.09`).
    pub version: String,
    /// Whether the `current` link points to this version.
    pub is_current: bool,
}

/// Compare two version strings in descending order (newest first).
///
/// Numeric dot-separated segments are compared segment by segment; the
/// leading `v`/`V` prefix is ignored, matching Scoop's convention.
pub fn compare_versions_desc(a: &str, b: &str) -> Ordering {
    let a_ver = a.trim_start_matches(['v', 'V']);
    let b_ver = b.trim_start_matches(['v', 'V']);
    let a_parts: Vec<u64> = a_ver.split('.').filter_map(|s| s.parse().ok()).collect();
    let b_parts: Vec<u64> = b_ver.split('.').filter_map(|s| s.parse().ok()).collect();
    for (a_n, b_n) in a_parts.iter().zip(b_parts.iter()) {
        match a_n.cmp(b_n) {
            Ordering::Equal => continue,
            other => return other.reverse(),
        }
    }
    a_parts.len().cmp(&b_parts.len()).reverse()
}

/// List all installed version directories of a package, newest first.
///
/// Returns an empty vector if the package is not installed or its directory
/// is not readable. I/O failures are treated as "no versions" so that a
/// broken install is reported gracefully by the caller.
pub fn list_installed_versions(session: &Session, name: &str) -> Fallible<Vec<InstalledVersion>> {
    let apps_dir = session.effective_root_path().join("apps").join(name);
    if !apps_dir.exists() {
        return Ok(Vec::new());
    }

    let current_target = std::fs::read_link(apps_dir.join("current"))
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()));

    let mut versions: Vec<String> = std::fs::read_dir(&apps_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n != "current")
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    versions.sort_by(|a, b| compare_versions_desc(a, b));

    Ok(versions
        .into_iter()
        .map(|version| InstalledVersion {
            is_current: current_target.as_deref() == Some(version.as_str()),
            version,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_desc_numeric() {
        assert_eq!(compare_versions_desc("2.0", "1.0"), Ordering::Less);
        assert_eq!(compare_versions_desc("1.0", "2.0"), Ordering::Greater);
        assert_eq!(compare_versions_desc("1.0", "1.0"), Ordering::Equal);
    }

    #[test]
    fn compare_desc_ignores_v_prefix() {
        assert_eq!(compare_versions_desc("v2.0", "1.9"), Ordering::Less);
        assert_eq!(compare_versions_desc("V1.0", "2.0"), Ordering::Greater);
    }

    #[test]
    fn compare_desc_more_segments_newer() {
        assert_eq!(compare_versions_desc("1.0.1", "1.0"), Ordering::Less);
        assert_eq!(compare_versions_desc("1.0", "1.0.1"), Ordering::Greater);
    }

    #[test]
    fn compare_desc_same_prefix() {
        assert_eq!(compare_versions_desc("1.0", "1.1"), Ordering::Greater);
        assert_eq!(compare_versions_desc("1.1", "1.0"), Ordering::Less);
    }

    #[test]
    fn compare_desc_non_numeric_falls_back_to_len() {
        // "beta" parses to no segments; both empty → equal by length
        assert_eq!(compare_versions_desc("beta", "alpha"), Ordering::Equal);
    }
}
