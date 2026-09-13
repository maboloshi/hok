//! List installed versions of a package.
//!
//! Scans the version directories under `<root>/apps/<name>/` and reports them
//! in descending version order, flagging the one the `current` link points to
//! — the business logic behind the `hok list --versions` flag. Also resolves
//! the app's `Updated` timestamp for the plain listing.

use std::cmp::Ordering;

use crate::{Session, error::Fallible};

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

    // Which version is current comes from the one shared resolver — never
    // from a local `read_link("current")`, which disagrees with upstream
    // `Select-CurrentVersion` (and with the other flows) on broken/nightly
    // installs.
    let current_target =
        super::install_state::resolve_current_version(&apps_dir, session.config().no_junction())
            .map(|resolved| resolved.version);

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

/// The `Updated` timestamp of an installed app, formatted as local
/// `yyyy-MM-dd HH:mm:ss`.
///
/// Upstream starts from the app directory's `LastWriteTime` and overrides it
/// with the current version's `install.json` `LastWriteTime` when that file
/// exists (`scoop-list.ps1:34-40`), rendered through the format string in
/// `supporting/formats/ScoopTypes.Format.ps1xml:22-25`. Returns `None` when
/// neither path can be stat'ed (an app whose directory vanished), so the
/// caller renders an empty cell instead of a bogus date.
pub fn update_time(session: &Session, name: &str, version: &str) -> Option<String> {
    let install_json = session.versioned_dir(name, version).join("install.json");
    let stamp = std::fs::metadata(&install_json)
        .and_then(|m| m.modified())
        .or_else(|_| std::fs::metadata(session.app_dir(name)).and_then(|m| m.modified()))
        .ok()?;
    crate::internal::time::format_updated_local(stamp)
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

    /// `Updated` uses `install.json`'s mtime when present (even an old one),
    /// and falls back to the app directory's when the file is missing;
    /// a vanished app yields `None`.
    #[test]
    fn update_time_prefers_install_json_and_falls_back_to_the_app_dir() {
        let root = crate::test_utils::tmpdir("list_update_time");
        let session = crate::test_utils::test_session(&root);
        let version_dir = root.join("apps/app/1.0.0");
        std::fs::create_dir_all(&version_dir).unwrap();

        // No install.json yet: the app directory is the fallback source.
        let fallback = update_time(&session, "app", "1.0.0").expect("app dir exists");
        assert_eq!(fallback.len(), 19, "yyyy-MM-dd HH:mm:ss: {fallback}");

        // With an install.json pinned to a known instant, that mtime wins —
        // even though it is far older than the app directory.
        let install_json = version_dir.join("install.json");
        std::fs::write(&install_json, b"{}").unwrap();
        let pinned = std::time::UNIX_EPOCH + std::time::Duration::from_secs(2_000_000_000);
        std::fs::File::options()
            .write(true)
            .open(&install_json)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(pinned))
            .unwrap();
        assert_eq!(
            update_time(&session, "app", "1.0.0").unwrap(),
            crate::internal::time::format_updated_local(pinned).unwrap()
        );

        // Neither the version dir nor the app dir exists.
        std::fs::remove_dir_all(root.join("apps/app")).unwrap();
        assert!(update_time(&session, "app", "1.0.0").is_none());
    }
}
