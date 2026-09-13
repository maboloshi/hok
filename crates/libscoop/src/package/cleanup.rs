//! Clean up old versions of installed packages.
//!
//! Removes all version directories except the current one for each package,
//! mirroring Scoop's `cleanup` command.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::{Error, Session, cache, error::Fallible, internal, package::install_state};

/// Behaviour knobs for [`cleanup`].
#[derive(Clone, Copy, Debug, Default)]
pub struct CleanupOptions {
    /// Continue after a failed version-dir removal instead of aborting the
    /// rest of the batch.
    pub ignore_failure: bool,
    /// Also drop each app's outdated download cache, keeping its current
    /// version — upstream `cleanup -k` (`scoop-cleanup.ps1:32-33`).
    pub cache: bool,
}

/// The app directory in an explicit scope: upstream `appdir $app $global`
/// (`lib/core.ps1`), which takes the scope as a parameter rather than reading
/// the session's current one.
fn scope_app_dir(session: &Session, name: &str, global: bool) -> PathBuf {
    let config = session.config();
    let root = if global {
        config.global_path()
    } else {
        config.root_path()
    };
    root.join("apps").join(name)
}

/// Whether an existing app directory holds a usable install — upstream `failed`
/// (`lib/core.ps1:404-409`).
///
/// A version must resolve, and in junction mode the `current` junction must be
/// present: a directory whose junction was lost mid-upgrade is "not installed
/// correctly" even though a version directory survives. A missing
/// `install.json` is *not* a failure here — upstream's `Select-CurrentVersion`
/// reads the version from `current/manifest.json`.
fn install_failed(pkg_dir: &Path, no_junction: bool) -> bool {
    let installed = install_state::resolve_current_version(pkg_dir, no_junction).is_some();
    let has_current = no_junction || pkg_dir.join("current").exists();
    !(has_current && installed)
}

/// Resolve the named apps a `cleanup` invocation may act on, reporting the
/// ones it cannot — upstream `Confirm-InstallationStatus`
/// (`lib/core.ps1:1084-1121`).
///
/// `global` is the requested scope. An app installed only in the *other* scope
/// is reported with upstream's "try again with/without `--global`" hints
/// instead of being cleaned; an app directory that exists but has no usable
/// install is reported as not installed correctly. The return value holds the
/// cleanable names in `global`, deduplicated and in input order.
pub fn confirm_installation_status(
    session: &Session,
    names: &[String],
    global: bool,
) -> Vec<String> {
    let no_junction = session.config().no_junction();
    let mut installed = Vec::new();
    let mut seen = HashSet::new();

    for name in names {
        // Upstream skips `scoop` itself and deduplicates the app list.
        if name == "scoop" || !seen.insert(name.as_str()) {
            continue;
        }

        let here = scope_app_dir(session, name, global);
        if here.exists() {
            if install_failed(&here, no_junction) {
                session
                    .output()
                    .error(hok_i18n::t!("cmd.not_installed_correctly", name = name));
            } else {
                installed.push(name.clone());
            }
        } else if scope_app_dir(session, name, !global).exists() {
            if global {
                session.output().error(hok_i18n::t!(
                    "cmd.cleanup_not_installed_globally",
                    name = name
                ));
                session.output().warn(hok_i18n::t!("cmd.cleanup_try_local"));
            } else {
                session.output().error(hok_i18n::t!(
                    "cmd.cleanup_not_installed_locally",
                    name = name
                ));
                session
                    .output()
                    .warn(hok_i18n::t!("cmd.cleanup_try_global"));
            }
        } else {
            session
                .output()
                .error(hok_i18n::t!("cmd.not_installed", name = name));
        }
    }

    installed
}

/// Every installed app name in the session's current scope, excluding `scoop`
/// itself — upstream `installed_apps` (`lib/core.ps1:396-399`).
///
/// The sweep-all forms enumerate the apps through this and pass the explicit
/// list to [`cleanup`]: an empty list then means "nothing to do", never
/// "everything".
pub fn installed_app_names(session: &Session) -> Vec<String> {
    let apps_dir = session.apps_dir();
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&apps_dir) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_dir())
                && let Some(name) = entry.file_name().to_str()
                && name != "scoop"
            {
                names.push(name.to_owned());
            }
        }
    }
    names.sort();
    names
}

/// Cleanup old versions of packages.
///
/// Removes all version directories except the current one for each named
/// package, and returns a list of (package_name, removed_count, failed_count).
/// `names` is always an explicit target list (upstream's `$apps`): an empty
/// list cleans nothing, so callers sweeping every app must enumerate it
/// through [`installed_app_names`]. A name that does not exist yields
/// [`PackageNotFound`][1] unless `options.ignore_failure` is set.
///
/// # Errors
///
/// A [`PackageNotFound`][1] error will be returned if a named package does not
/// exist and `options.ignore_failure` is false.
///
/// [1]: crate::Error::PackageNotFound
pub fn cleanup(
    session: &Session,
    names: &[String],
    options: &CleanupOptions,
) -> Fallible<Vec<(String, usize, usize)>> {
    let ignore_failure = options.ignore_failure;
    let apps_dir = session.apps_dir();
    let mut results = Vec::new();

    for name in names {
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
            install_state::resolve_current_version(&pkg_dir, session.config().no_junction())
                .map(|resolved| resolved.version);

        let Some(ref current_ver) = current_version else {
            // No install info — skip broken package
            continue;
        };

        // Upstream drops the app's outdated cache before touching its version
        // dirs, and it does so for every app in the batch — including one with
        // no old versions. The current version is excluded so a reinstall does
        // not re-download it.
        if options.cache {
            cache::remove_outdated(session, name, current_ver)?;
        }

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

    fn opts(ignore_failure: bool) -> CleanupOptions {
        CleanupOptions {
            ignore_failure,
            cache: false,
        }
    }

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

        let results = cleanup(&session, &["app".to_owned()], &opts(false)).unwrap();

        assert_eq!(results, vec![("app".to_owned(), 2, 0)]);
        assert!(root.0.join("apps/app/1.0.0").exists(), "current kept");
        assert!(!root.0.join("apps/app/0.9.0").exists(), "old removed");
        assert!(!root.0.join("apps/app/0.8.0").exists(), "old removed");
    }

    /// `cleanup -k` drops the app's outdated cached downloads but keeps the
    /// current version's (upstream excludes `$app#$current_version#*`) and
    /// leaves other apps' cache alone.
    #[test]
    fn cleanup_with_cache_keeps_current_version_download() {
        let (session, root) = setup("keep_current_cache");
        install_versions(&root.0, "app", &["2.0.0", "1.0.0"], "2.0.0");
        let cache = root.0.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        for name in ["app#1.0.0#a.zip", "app#2.0.0#b.zip", "other#1.0#c.zip"] {
            std::fs::write(cache.join(name), b"x").unwrap();
        }

        let results = cleanup(
            &session,
            &["app".to_owned()],
            &CleanupOptions {
                ignore_failure: false,
                cache: true,
            },
        )
        .unwrap();

        assert_eq!(results, vec![("app".to_owned(), 1, 0)]);
        assert!(
            !cache.join("app#1.0.0#a.zip").exists(),
            "outdated cache dropped"
        );
        assert!(
            cache.join("app#2.0.0#b.zip").exists(),
            "current version cache kept"
        );
        assert!(
            cache.join("other#1.0#c.zip").exists(),
            "other apps' cache untouched"
        );
    }

    /// `Confirm-InstallationStatus`: only apps installed in the requested scope
    /// are returned; duplicates and `scoop` are skipped, and a missing app is
    /// reported (and dropped) instead of aborting the batch.
    #[test]
    fn confirm_installation_status_filters_named_apps() {
        let (session, root) = setup("confirm_status");
        crate::test_utils::mark_installed(&root.0, "app", "main", r#"{"version":"1.0.0"}"#, false);

        let picked = confirm_installation_status(
            &session,
            &[
                "app".to_owned(),
                "app".to_owned(),
                "ghost".to_owned(),
                "scoop".to_owned(),
            ],
            false,
        );

        assert_eq!(picked, vec!["app".to_owned()]);
    }

    /// An app directory whose `current` junction is gone (an interrupted
    /// upgrade) is not cleanable — upstream `failed` requires the junction —
    /// so it is reported and dropped instead of being cleaned.
    #[test]
    fn confirm_installation_status_rejects_a_damaged_install() {
        let (session, root) = setup("confirm_damaged");
        let pkg = root.0.join("apps/app");
        std::fs::create_dir_all(pkg.join("1.0.0")).unwrap();
        std::fs::write(
            pkg.join("1.0.0/install.json"),
            r#"{"architecture":"64bit"}"#,
        )
        .unwrap();

        let picked = confirm_installation_status(&session, &["app".to_owned()], false);

        assert!(picked.is_empty(), "a damaged install must not be cleaned");
    }

    /// A resolvable `current` manifest is enough, even without `install.json`:
    /// upstream reads the version from `current/manifest.json`, not from the
    /// install metadata.
    #[test]
    fn confirm_installation_status_accepts_manifest_without_install_info() {
        let (session, root) = setup("confirm_no_install_json");
        let current = root.0.join("apps/app/current");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("manifest.json"), r#"{"version":"1.0.0"}"#).unwrap();

        let picked = confirm_installation_status(&session, &["app".to_owned()], false);

        assert_eq!(picked, vec!["app".to_owned()]);
    }

    #[test]
    fn cleanup_no_old_versions_returns_empty() {
        let (session, root) = setup("only_current");
        install_versions(&root.0, "app", &["1.0.0"], "1.0.0");

        let results = cleanup(&session, &["app".to_owned()], &opts(false)).unwrap();

        assert!(results.is_empty(), "nothing to clean");
    }

    /// The sweep-all form enumerates every installed app and cleans each —
    /// and `scoop` itself is never a target (upstream `installed_apps`).
    #[test]
    fn cleanup_all_apps_via_explicit_names() {
        let (session, root) = setup("scan_all");
        install_versions(&root.0, "app1", &["2.0.0", "1.0.0"], "2.0.0");
        install_versions(&root.0, "app2", &["1.5.0", "1.0.0"], "1.5.0");
        std::fs::create_dir_all(root.0.join("apps/scoop/1.0.0")).unwrap();

        let names = installed_app_names(&session);
        assert_eq!(names, ["app1", "app2"], "scoop excluded, order stable");
        let results = cleanup(&session, &names, &opts(false)).unwrap();

        let mut cleaned = results
            .iter()
            .map(|(n, r, f)| (n.as_str(), *r, *f))
            .collect::<Vec<_>>();
        cleaned.sort();
        assert_eq!(cleaned, vec![("app1", 1, 0), ("app2", 1, 0)]);
    }

    /// An empty target list cleans nothing — it must never be re-read as
    /// "every app", or a `cleanup <not-installed>` would sweep the install.
    #[test]
    fn cleanup_with_no_targets_is_a_noop() {
        let (session, root) = setup("no_targets");
        install_versions(&root.0, "app", &["2.0.0", "1.0.0"], "2.0.0");

        let results = cleanup(&session, &[], &opts(false)).unwrap();

        assert!(results.is_empty());
        assert!(
            root.0.join("apps/app/1.0.0").exists(),
            "an empty list cleaned an app"
        );
    }

    #[test]
    fn cleanup_missing_package_errors_without_ignore() {
        let (session, _root) = setup("missing");

        let err = cleanup(&session, &["nope".to_owned()], &opts(false)).unwrap_err();

        assert!(matches!(err, Error::PackageNotFound(name) if name == "nope"));
    }

    #[test]
    fn cleanup_missing_package_skipped_with_ignore() {
        let (session, root) = setup("missing_ignored");
        install_versions(&root.0, "app", &["1.0.0"], "1.0.0");

        let results = cleanup(
            &session,
            &["nope".to_owned(), "app".to_owned()],
            &opts(true),
        )
        .unwrap();

        assert!(results.is_empty(), "missing skipped, nothing to clean");
    }

    #[test]
    fn cleanup_broken_package_without_current_is_skipped() {
        let (session, root) = setup("broken");
        // apps/app exists but has no `current` symlink
        std::fs::create_dir_all(root.0.join("apps/app/1.0.0")).unwrap();

        let results = cleanup(&session, &["app".to_owned()], &opts(false)).unwrap();

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

        let results = cleanup(&session, &["app".to_owned()], &opts(false)).unwrap();

        assert_eq!(results, vec![("app".to_owned(), 2, 0)]);
        assert!(pkg_dir.join("2.0.0").exists(), "newest version kept");
        assert!(!pkg_dir.join("1.0.0").exists(), "old version removed");
        assert!(!pkg_dir.join("_1.0.0.old").exists(), "old dir removed");
        let _ = std::fs::remove_dir_all(&root);
    }
}
