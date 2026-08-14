//! Clean up old versions of installed packages.
//!
//! Removes all version directories except the current one for each package,
//! mirroring Scoop's `cleanup` command.

use crate::{error::Fallible, internal, Error, Session};

/// Cleanup old versions of packages.
///
/// Removes all version directories except the current one for each package.
/// If `names` is empty, cleans up all installed packages.
/// Returns a list of (package_name, removed_count, failed_count).
///
/// # Errors
///
/// A [`PackageNotFound`][1] error will be returned if a named package does not
/// exist and `ignore_failure` is false.
///
/// [1]: crate::Error::PackageNotFound
pub fn cleanup(
    session: &Session,
    names: &[String],
    ignore_failure: bool,
) -> Fallible<Vec<(String, usize, usize)>> {
    let apps_dir = session.apps_dir();
    let mut results = Vec::new();

    // If no names given, scan all installed packages
    let scan_names: Vec<String> = if names.is_empty() {
        let mut all = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&apps_dir) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    if let Some(name) = entry.file_name().to_str() {
                        all.push(name.to_owned());
                    }
                }
            }
        }
        all
    } else {
        names.to_vec()
    };

    for name in &scan_names {
        let pkg_dir = apps_dir.join(name);
        if !pkg_dir.exists() {
            if !ignore_failure {
                return Err(Error::PackageNotFound(name.to_owned()));
            }
            continue;
        }

        // Determine current version: read the "current" symlink target, or
        // under NO_JUNCTION resolve via Select-CurrentVersion fallback.
        let current_version = if session.config().no_junction() {
            super::install_state::select_current_version(&pkg_dir)
        } else {
            (|| -> Option<String> {
                let current_link = pkg_dir.join("current");
                std::fs::read_link(&current_link)
                    .ok()?
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
            })()
        };

        let Some(ref current_ver) = current_version else {
            // No install info — skip broken package
            continue;
        };

        // Collect old version directories
        let mut old_dirs = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&pkg_dir) {
            for entry in entries.flatten() {
                let fname = entry.file_name();
                let name_str = match fname.to_str() {
                    Some(s) => s,
                    None => continue,
                };
                // Skip "current" symlink and the current version
                if name_str == "current" || name_str == current_ver {
                    continue;
                }
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    old_dirs.push(name_str.to_owned());
                }
            }
        }

        if old_dirs.is_empty() {
            continue;
        }

        let mut removed = 0u32;
        let mut failed = 0u32;
        for ver in &old_dirs {
            let ver_dir = pkg_dir.join(ver);
            if let Err(e) = internal::fs::remove_dir(&ver_dir) {
                let msg = format!("failed to remove {} v{}: {}", name, ver, e);
                if ignore_failure {
                    session.output().error(msg);
                    failed += 1;
                } else {
                    return Err(Error::Custom(msg));
                }
            } else {
                removed += 1;
            }
        }

        if removed > 0 || failed > 0 {
            results.push((name.clone(), removed as usize, failed as usize));
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a temp root with a session rooted at it, plus a drop guard.
    fn setup(test_name: &str) -> (Session, PathBufGuard) {
        let root = crate::test_utils::tmpdir(&format!("cleanup_{}", test_name));
        let session = crate::test_utils::test_session(&root);
        (session, PathBufGuard(root))
    }

    struct PathBufGuard(std::path::PathBuf);
    impl Drop for PathBufGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Create `apps/<name>/<version>/` dirs and point `apps/<name>/current`
    /// at `<version>` via a directory symlink.
    fn install_versions(root: &std::path::Path, name: &str, versions: &[&str], current: &str) {
        let pkg_dir = root.join("apps").join(name);
        for v in versions {
            std::fs::create_dir_all(pkg_dir.join(v)).unwrap();
        }
        internal::fs::symlink_dir(pkg_dir.join(current), pkg_dir.join("current")).unwrap();
    }

    #[test]
    fn cleanup_removes_old_versions_keeps_current() {
        let (session, root) = setup("old_versions");
        install_versions(&root.0, "app", &["1.0.0", "0.9.0", "0.8.0"], "1.0.0");

        let results = cleanup(&session, &["app".to_owned()], false).unwrap();

        assert_eq!(results, vec![("app".to_owned(), 2, 0)]);
        assert!(root.0.join("apps/app/1.0.0").exists(), "current kept");
        assert!(!root.0.join("apps/app/0.9.0").exists(), "old removed");
        assert!(!root.0.join("apps/app/0.8.0").exists(), "old removed");
    }

    #[test]
    fn cleanup_no_old_versions_returns_empty() {
        let (session, root) = setup("only_current");
        install_versions(&root.0, "app", &["1.0.0"], "1.0.0");

        let results = cleanup(&session, &["app".to_owned()], false).unwrap();

        assert!(results.is_empty(), "nothing to clean");
    }

    #[test]
    fn cleanup_empty_names_scans_all() {
        let (session, root) = setup("scan_all");
        install_versions(&root.0, "app1", &["2.0.0", "1.0.0"], "2.0.0");
        install_versions(&root.0, "app2", &["1.5.0", "1.0.0"], "1.5.0");

        let results = cleanup(&session, &[], false).unwrap();

        let mut cleaned = results
            .iter()
            .map(|(n, r, f)| (n.as_str(), *r, *f))
            .collect::<Vec<_>>();
        cleaned.sort();
        assert_eq!(cleaned, vec![("app1", 1, 0), ("app2", 1, 0)]);
    }

    #[test]
    fn cleanup_missing_package_errors_without_ignore() {
        let (session, _root) = setup("missing");

        let err = cleanup(&session, &["nope".to_owned()], false).unwrap_err();

        assert!(matches!(err, Error::PackageNotFound(name) if name == "nope"));
    }

    #[test]
    fn cleanup_missing_package_skipped_with_ignore() {
        let (session, root) = setup("missing_ignored");
        install_versions(&root.0, "app", &["1.0.0"], "1.0.0");

        let results = cleanup(&session, &["nope".to_owned(), "app".to_owned()], true).unwrap();

        assert!(results.is_empty(), "missing skipped, nothing to clean");
    }

    #[test]
    fn cleanup_broken_package_without_current_is_skipped() {
        let (session, root) = setup("broken");
        // apps/app exists but has no `current` symlink
        std::fs::create_dir_all(root.0.join("apps/app/1.0.0")).unwrap();

        let results = cleanup(&session, &["app".to_owned()], false).unwrap();

        assert!(results.is_empty(), "broken package skipped");
    }
}
