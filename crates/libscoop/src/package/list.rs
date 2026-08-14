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
/// Delegates to the unified [`crate::internal::compare_versions`] comparator
/// (numeric dot-separated segments compared segment by segment, leading
/// `v`/`V` prefix ignored, matching Scoop's convention) and reverses the
/// result so that newer versions sort first.
pub fn compare_versions_desc(a: &str, b: &str) -> Ordering {
    crate::internal::compare_versions(a, b).reverse()
}

/// List all installed version directories of a package, newest first.
///
/// Returns an empty vector if the package is not installed or its directory
/// is not readable. I/O failures are treated as "no versions" so that a
/// broken install is reported gracefully by the caller.
pub fn list_installed_versions(session: &Session, name: &str) -> Fallible<Vec<InstalledVersion>> {
    let apps_dir = session.app_dir(name);
    if !apps_dir.exists() {
        return Ok(Vec::new());
    }

    let current_target = if session.config().no_junction() {
        // No `current` junction under NO_JUNCTION: resolve via
        // Select-CurrentVersion fallback (mtime of version dirs).
        super::install_state::select_current_version(&apps_dir)
    } else {
        std::fs::read_link(apps_dir.join("current"))
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
    };

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
    fn compare_desc_non_numeric_text_segments() {
        // Unified internal comparator falls back to alphabetical text compare:
        // "beta" > "alpha" ascending, so descending puts "beta" first.
        assert_eq!(compare_versions_desc("beta", "alpha"), Ordering::Less);
        assert_eq!(compare_versions_desc("alpha", "beta"), Ordering::Greater);
        assert_eq!(compare_versions_desc("beta", "beta"), Ordering::Equal);
    }
}
