//! Clean up old versions of installed packages.
//!
//! Removes all version directories except the current one for each package,
//! mirroring Scoop's `cleanup` command.

use crate::{Error, Session, error::Fallible, internal};

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
                if entry.file_type().is_ok_and(|t| t.is_dir())
                    && let Some(name) = entry.file_name().to_str()
                {
                    all.push(name.to_owned());
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

        // Determine the current version through the one shared resolver
        // (upstream `Select-CurrentVersion`) — a local `read_link("current")`
        // diverges from it on broken/nightly installs, and deleting the wrong
        // dir here is destructive.
        let current_version =
            super::install_state::resolve_current_version(&pkg_dir, session.config().no_junction())
                .map(|resolved| resolved.version);

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
                let msg = hok_i18n::t!("cmd.cleanup_remove_failed", name = name, ver = ver, e = e)
                    .to_string();
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
    /// at `<version>` via a directory symlink, writing the junction's
    /// `manifest.json` the way a real install does — upstream
    /// `Select-CurrentVersion` (and this crate's resolver) reads the current
    /// version from there, not from the link target.
    fn install_versions(root: &std::path::Path, name: &str, versions: &[&str], current: &str) {
        let pkg_dir = root.join("apps").join(name);
        for v in versions {
            std::fs::create_dir_all(pkg_dir.join(v)).unwrap();
        }
        internal::fs::symlink_dir(pkg_dir.join(current), pkg_dir.join("current")).unwrap();
        std::fs::write(
            pkg_dir.join("current").join("manifest.json"),
            format!(r#"{{"version": "{}"}}"#, current),
        )
        .unwrap();
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

    fn session_with_no_junction(root: &std::path::Path) -> Session {
        let config_path = root.join("hok.json");
        let root_escaped = root.to_string_lossy().replace('\\', "\\\\");
        let cache_escaped = root.join("cache").to_string_lossy().replace('\\', "\\\\");
        std::fs::write(
            &config_path,
            format!(
                r#"{{"root_path": "{}", "cache_path": "{}", "no_junction": true}}"#,
                root_escaped, cache_escaped
            ),
        )
        .unwrap();
        Session::new_with(&config_path).unwrap()
    }

    /// Under NO_JUNCTION a leftover `current` directory is stale — upstream
    /// `Select-CurrentVersion` never reads it there. Resolving through it (the
    /// pre-unification behaviour) made cleanup delete **both** live version
    /// dirs while keeping the stale one; `_*.old*` dirs are never kept either.
    #[test]
    fn cleanup_no_junction_ignores_stale_current() {
        let root = crate::test_utils::tmpdir("cleanup_no_junction_stale_current");
        let session = session_with_no_junction(&root);
        let pkg_dir = root.join("apps").join("app");
        // Real installs carry `install.json` in every version dir — that file
        // is the resolver's ordering key under NO_JUNCTION.
        for v in ["1.0.0", "2.0.0", "_1.0.0.old"] {
            std::fs::create_dir_all(pkg_dir.join(v)).unwrap();
            std::fs::write(
                pkg_dir.join(v).join("install.json"),
                r#"{"architecture": "64bit", "bucket": "test"}"#,
            )
            .unwrap();
        }
        // Pin the ordering key: 2.0.0 is the most recently touched version.
        {
            use std::fs::{File, FileTimes};
            let install_json = pkg_dir.join("2.0.0").join("install.json");
            let f = File::options().write(true).open(&install_json).unwrap();
            f.set_times(FileTimes::new().set_modified(
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(2_000_000),
            ))
            .unwrap();
            let f = File::options()
                .write(true)
                .open(pkg_dir.join("1.0.0").join("install.json"))
                .unwrap();
            f.set_times(FileTimes::new().set_modified(
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000),
            ))
            .unwrap();
        }
        // A stale `current` dir left behind by an earlier junction-mode
        // install, claiming a version that is not installed at all.
        std::fs::create_dir_all(pkg_dir.join("current")).unwrap();
        std::fs::write(
            pkg_dir.join("current").join("manifest.json"),
            r#"{"version": "0.9.9"}"#,
        )
        .unwrap();

        let results = cleanup(&session, &["app".to_owned()], false).unwrap();

        assert_eq!(results, vec![("app".to_owned(), 2, 0)]);
        assert!(pkg_dir.join("2.0.0").exists(), "newest version kept");
        assert!(!pkg_dir.join("1.0.0").exists(), "old version removed");
        assert!(!pkg_dir.join("_1.0.0.old").exists(), "old dir removed");
        let _ = std::fs::remove_dir_all(&root);
    }
}
