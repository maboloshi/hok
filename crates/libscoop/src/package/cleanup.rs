//! Clean up old versions of installed packages.
//!
//! Removes all version directories except the current one for each package,
//! mirroring Scoop's `cleanup` command.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::cache::CacheFile;
use crate::{Error, Session, cache, error::Fallible, internal, package::install_state};

/// Behaviour knobs for [`cleanup`].
#[derive(Clone, Copy, Debug, Default)]
pub struct CleanupOptions {
    /// Continue after a failed version-dir removal instead of aborting the
    /// rest of the batch.
    pub ignore_failures: bool,
    /// Also drop each app's outdated download cache, keeping its current
    /// version — upstream `cleanup -k` (`scoop-cleanup.ps1:32-33`).
    pub cache: bool,
}

/// What [`cleanup`] did to one app.
///
/// Carries the removed version strings (not just a count) so the CLI can
/// render upstream's per-app `Removing <app>: <version> ...` line
/// (`scoop-cleanup.ps1:43-46`). An entry with an empty `removed` and no
/// `failures` is an app that was already clean.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupResult {
    /// The app name.
    pub name: String,
    /// Old versions actually removed, newest first.
    pub removed: Vec<String>,
    /// Old versions that could not be removed, newest first.
    ///
    /// Carries the removal error rather than only a count: the library never
    /// prints (see [`crate::output`]), and a bare count cannot say *why* a
    /// version survived.
    pub failures: Vec<CleanupFailure>,
}

/// One old version directory that could not be removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupFailure {
    /// The version whose directory survived (possibly only partially — a
    /// recursive removal that fails keeps whatever it had not deleted yet).
    pub version: String,
    /// The removal error as reported by the OS, untranslated; the frontend
    /// wraps it in its localized line.
    pub error: String,
}

/// Everything one [`cleanup`] sweep did.
///
/// The per-app results stay in [`CleanupResult`] (they are what the per-version
/// rows render); the cache entries the sweep removed live here because they are
/// a property of the batch: `-k` can drop an app's cache without producing a
/// result (a broken install, see [`cleanup`]), and upstream prints nothing
/// about them, so the frontend renders one summary line for all of them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CleanupOutcome {
    /// One entry per app visited, in the order it was requested.
    pub apps: Vec<CleanupResult>,
    /// Cache entries removed, in the order [`cache::list`] presents them per
    /// app and the apps in request order.
    pub cache_removed: Vec<CacheFile>,
}

/// Progress notification for one old-version directory, delivered to the
/// observer passed to [`cleanup`].
///
/// One `Started`/`Finished` pair is emitted per planned version directory, from
/// the worker thread that removes it — which lets a frontend drive a
/// per-version progress row without owning the parallelism. A directory that is
/// never started (strict mode aborted the sweep first) emits nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CleanupProgress {
    /// A worker claimed this version directory.
    Started {
        /// The app name.
        name: String,
        /// The version being removed.
        version: String,
    },
    /// The removal attempt settled; `error` is `None` when the directory is gone.
    Finished {
        /// The app name.
        name: String,
        /// The version that was being removed.
        version: String,
        /// The removal error, or `None` on success.
        error: Option<String>,
    },
}

/// Observer receiving [`CleanupProgress`] from `cleanup`'s worker threads.
///
/// `Sync` rather than `Send`: every worker borrows it, it is never moved.
pub type CleanupObserver<'a> = dyn Fn(CleanupProgress) + Sync + 'a;

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
/// install record is *not* a failure here — upstream's `Select-CurrentVersion`
/// reads the version from `current/scoop-manifest.json` (else
/// `current/manifest.json`).
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

/// One app's cleanup plan: the version directories a sweep will remove.
struct PlannedApp {
    /// The app name.
    name: String,
    /// Version directories to remove, newest first (reproducible order).
    old_versions: Vec<String>,
}

/// Old version directories of one installed app, newest first — every directory
/// except `current` and the version `current` resolves to.
fn old_version_dirs(pkg_dir: &Path, current_ver: &str) -> Vec<String> {
    let mut old_dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(pkg_dir) {
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
    // `read_dir` returns an arbitrary order; sorting newest-first makes both
    // the removal order and the rendered version list reproducible.
    old_dirs.sort_by(|a, b| super::list::compare_versions_desc(a, b));
    old_dirs
}

/// How many threads a cleanup sweep may use.
///
/// Removing directories costs ~0.87 ms per file on NTFS and scales close to
/// linearly with threads (measured 2026-09-20: 4 threads ≈ 4.3× on 30k files),
/// while the workers share nothing but the outcome slots. The cap stays low on
/// purpose: more threads show no measured gain, and every extra worker widens
/// the blast radius of this destructive operation.
fn worker_count() -> usize {
    const MAX_WORKERS: usize = 4;
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get().min(MAX_WORKERS))
        .unwrap_or(1)
        .max(1)
}

/// Cleanup old versions of packages.
///
/// Removes all version directories except the current one for each named
/// package, and returns one [`CleanupResult`] per app it processed (including
/// apps that were already clean), together with the cache entries `-k` dropped.
/// `names` is always an explicit target list (upstream's `$apps`): an empty
/// list cleans nothing, so callers sweeping every app must enumerate it through
/// [`installed_app_names`]. A name that does not exist yields
/// [`PackageNotFound`][1] unless `options.ignore_failures` is set.
///
/// The sweep removes version directories with `worker_count()` threads and
/// reports each one through `observer` when it is claimed and when it settles.
/// The returned results keep the planned order regardless of which worker
/// finished first, so a frontend renders a run reproducibly.
///
/// # Errors
///
/// A [`PackageNotFound`][1] error will be returned if a named package does not
/// exist and `options.ignore_failures` is false. A failed removal returns
/// [`Error::Custom`] under the same condition: the first failure in planned
/// order is reported and the remaining workers are stopped.
///
/// [1]: crate::Error::PackageNotFound
pub fn cleanup(
    session: &Session,
    names: &[String],
    options: &CleanupOptions,
    observer: &CleanupObserver<'_>,
) -> Fallible<CleanupOutcome> {
    let ignore_failures = options.ignore_failures;
    let apps_dir = session.apps_dir();
    let no_junction = session.config().no_junction();
    let mut cache_removed: Vec<CacheFile> = Vec::new();

    // ── Plan ────────────────────────────────────────────────────────────────
    // Resolving each app's current version and enumerating its old version dirs
    // stays on this thread: it is cheap next to the removals, and it keeps the
    // `current`-version guard (deleting the wrong directory is destructive) and
    // the per-app cache drop out of the worker pool.
    let mut plans: Vec<PlannedApp> = Vec::new();

    for name in names {
        let pkg_dir = apps_dir.join(name);
        if !pkg_dir.exists() {
            if !ignore_failures {
                return Err(Error::PackageNotFound(name.to_owned()));
            }
            continue;
        }

        // Determine the current version through the one shared resolver
        // (upstream `Select-CurrentVersion`) — a local `read_link("current")`
        // diverges from it on broken/nightly installs, and deleting the wrong
        // dir here is destructive.
        let resolved = install_state::resolve_current_version(&pkg_dir, no_junction);

        // A junction install whose `current` metadata is unusable still
        // resolves — through the version dirs, exactly as upstream's
        // `Get-InstalledVersion` fallback does — and that resolved version is
        // then *excluded* from deletion, so a wrong answer preserves the wrong
        // directory. Upstream says so through `parse_json`'s warning; hok's own
        // parse warnings sit behind `-v`/`-vv`, so the degradation is surfaced
        // here, before removals are planned.
        if !no_junction
            && let Some(resolved) = &resolved
            && resolved.source == install_state::CurrentVersionSource::VersionDirs
        {
            session.output().warn(hok_i18n::t!(
                "cmd.cleanup_current_from_version_dirs",
                name = name,
                version = &resolved.version
            ));
        }

        let current_version = resolved.map(|resolved| resolved.version);

        let Some(ref current_ver) = current_version else {
            // No install info — skip broken package. Its cache still goes:
            // upstream's exclude pattern needs a version to name, so with none
            // it matches nothing and the app's whole cache is dropped
            // (`scoop-cleanup.ps1:32-33`). Leaving it behind would pin the
            // downloads of an install the user is about to repair by hand.
            if options.cache {
                cache_removed.extend(cache::remove_all_for_app(session, name)?);
            }
            continue;
        };

        // Upstream drops the app's outdated cache before touching its version
        // dirs, and it does so for every app in the batch — including one with
        // no old versions. The current version is excluded so a reinstall does
        // not re-download it.
        if options.cache {
            cache_removed.extend(cache::remove_outdated(session, name, current_ver)?);
        }

        plans.push(PlannedApp {
            name: name.clone(),
            old_versions: old_version_dirs(&pkg_dir, current_ver),
        });
    }

    // ── Remove ──────────────────────────────────────────────────────────────
    // One work item per (app, old version), claimed from a shared counter by
    // `worker_count()` threads. Outcomes land in per-item slots, so the result
    // keeps the planned order no matter which worker finished first.
    let units: Vec<(&str, &str)> = plans
        .iter()
        .flat_map(|plan| {
            plan.old_versions
                .iter()
                .map(move |version| (plan.name.as_str(), version.as_str()))
        })
        .collect();
    let outcomes: Mutex<Vec<Option<String>>> = Mutex::new(vec![None; units.len()]);
    let next = AtomicUsize::new(0);
    // Strict mode keeps its stop-at-the-first-failure shape: once a worker
    // records one, no worker claims further units.
    let abort = AtomicBool::new(false);

    std::thread::scope(|scope| {
        for _ in 0..worker_count() {
            scope.spawn(|| {
                loop {
                    if abort.load(Ordering::Relaxed) {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&(name, version)) = units.get(index) else {
                        break;
                    };
                    observer(CleanupProgress::Started {
                        name: name.to_owned(),
                        version: version.to_owned(),
                    });
                    let error = internal::fs::remove_dir(apps_dir.join(name).join(version))
                        .err()
                        .map(|e| e.to_string());
                    if let Some(error) = &error {
                        outcomes.lock().unwrap()[index] = Some(error.clone());
                        if !ignore_failures {
                            abort.store(true, Ordering::Relaxed);
                        }
                    }
                    observer(CleanupProgress::Finished {
                        name: name.to_owned(),
                        version: version.to_owned(),
                        error,
                    });
                }
            });
        }
    });

    let outcomes = outcomes.into_inner().unwrap();

    // Strict mode reports the first failure in planned order; the removals that
    // ran before it are already done, exactly as the serial version behaved.
    if !ignore_failures && let Some(index) = outcomes.iter().position(Option::is_some) {
        let (name, version) = units[index];
        let error = outcomes[index].clone().unwrap_or_default();
        return Err(Error::Custom(
            hok_i18n::t!(
                "cmd.cleanup_remove_failed",
                name = name,
                ver = version,
                e = error
            )
            .to_string(),
        ));
    }

    // ── Assemble ────────────────────────────────────────────────────────────
    // `removed` and `failures` keep the planned (newest-first) version order.
    let mut results = Vec::with_capacity(plans.len());
    let mut index = 0;
    for plan in plans {
        let mut removed = Vec::new();
        let mut failures = Vec::new();
        for version in plan.old_versions {
            match &outcomes[index] {
                Some(error) => failures.push(CleanupFailure {
                    version: version.clone(),
                    error: error.clone(),
                }),
                None => removed.push(version),
            }
            index += 1;
        }
        results.push(CleanupResult {
            name: plan.name,
            removed,
            failures,
        });
    }

    Ok(CleanupOutcome {
        apps: results,
        cache_removed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(ignore_failures: bool) -> CleanupOptions {
        CleanupOptions {
            ignore_failures,
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

        let outcome = cleanup(&session, &["app".to_owned()], &opts(false), &|_| {}).unwrap();

        assert_eq!(
            outcome.apps,
            vec![CleanupResult {
                name: "app".to_owned(),
                removed: vec!["0.9.0".to_owned(), "0.8.0".to_owned()],
                failures: Vec::new(),
            }]
        );
        assert!(root.0.join("apps/app/1.0.0").exists(), "current kept");
        assert!(!root.0.join("apps/app/0.9.0").exists(), "old removed");
        assert!(!root.0.join("apps/app/0.8.0").exists(), "old removed");
    }

    /// A junction install whose `current` manifest is unreadable still resolves
    /// — through the version dirs, as upstream's `Get-InstalledVersion` fallback
    /// does — and that resolved version is the one cleanup *keeps*. The
    /// degradation therefore has to be visible before removals are planned, not
    /// only as a `-v`/`-vv` parse warning.
    ///
    /// The resolution itself is unchanged: the app is still cleaned.
    #[test]
    fn cleanup_warns_when_current_manifest_is_unreadable() {
        use std::sync::Arc;

        let (session, root) = setup("broken_current_warns");
        install_versions(&root.0, "app", &["1.0.0", "0.9.0"], "1.0.0");
        // Through the junction this is `1.0.0/manifest.json`; corrupt it and
        // leave the install record the version-dir fallback orders by.
        std::fs::write(
            root.0.join("apps/app/current/manifest.json"),
            b"{ \"version\": ",
        )
        .unwrap();
        std::fs::write(
            root.0.join("apps/app/1.0.0/install.json"),
            r#"{"architecture":"64bit","bucket":"main"}"#,
        )
        .unwrap();

        let warnings: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&warnings);
        session
            .set_output(Arc::new(move |out| {
                if let crate::output::Output::Warn(msg) = out {
                    sink.lock().unwrap().push(msg);
                }
            }))
            .unwrap();

        let outcome = cleanup(&session, &["app".to_owned()], &opts(false), &|_| {}).unwrap();

        let warnings = warnings.lock().unwrap();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("no usable manifest under 'current'")),
            "the version-dir fallback must be reported, got {warnings:?}"
        );
        assert_eq!(
            outcome.apps[0].removed,
            vec!["0.9.0".to_owned()],
            "reporting the fallback must not change what is removed"
        );
    }

    /// `cleanup -k` drops the app's outdated cached downloads but keeps the
    /// current version's (upstream excludes `$app#$current_version#*`), leaves
    /// other apps' cache alone, and reports what it dropped — upstream is silent
    /// here, so the returned list is the only way `-k` becomes visible.
    #[test]
    fn cleanup_with_cache_keeps_current_version_download() {
        let (session, root) = setup("keep_current_cache");
        install_versions(&root.0, "app", &["2.0.0", "1.0.0"], "2.0.0");
        let cache = root.0.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        for name in ["app#1.0.0#a.zip", "app#2.0.0#b.zip", "other#1.0#c.zip"] {
            std::fs::write(cache.join(name), b"x").unwrap();
        }

        let outcome = cleanup(
            &session,
            &["app".to_owned()],
            &CleanupOptions {
                ignore_failures: false,
                cache: true,
            },
            &|_| {},
        )
        .unwrap();

        assert_eq!(outcome.apps.len(), 1);
        assert_eq!(outcome.apps[0].name, "app");
        assert_eq!(outcome.apps[0].removed, ["1.0.0"]);
        assert!(outcome.apps[0].failures.is_empty());
        assert_eq!(
            outcome
                .cache_removed
                .iter()
                .map(CacheFile::file_name)
                .collect::<Vec<_>>(),
            ["app#1.0.0#a.zip"],
            "the dropped cache entry travels back for the summary line"
        );
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

    /// A resolvable `current` manifest is enough, even without an install
    /// record: upstream reads the version from the current manifest
    /// (`scoop-manifest.json`, else `manifest.json`), not from the
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
    fn cleanup_no_old_versions_yields_an_empty_result() {
        let (session, root) = setup("only_current");
        install_versions(&root.0, "app", &["1.0.0"], "1.0.0");

        let outcome = cleanup(&session, &["app".to_owned()], &opts(false), &|_| {}).unwrap();

        assert_eq!(outcome.apps.len(), 1, "the app was visited");
        assert_eq!(outcome.apps[0].name, "app");
        assert!(outcome.apps[0].removed.is_empty(), "nothing to clean");
        assert!(outcome.apps[0].failures.is_empty());
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
        let outcome = cleanup(&session, &names, &opts(false), &|_| {}).unwrap();

        let mut cleaned = outcome
            .apps
            .iter()
            .map(|r| (r.name.as_str(), r.removed.len(), r.failures.len()))
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

        let outcome = cleanup(&session, &[], &opts(false), &|_| {}).unwrap();

        assert!(outcome.apps.is_empty());
        assert!(
            root.0.join("apps/app/1.0.0").exists(),
            "an empty list cleaned an app"
        );
    }

    #[test]
    fn cleanup_missing_package_errors_without_ignore() {
        let (session, _root) = setup("missing");

        let err = cleanup(&session, &["nope".to_owned()], &opts(false), &|_| {}).unwrap_err();

        assert!(matches!(err, Error::PackageNotFound(name) if name == "nope"));
    }

    #[test]
    fn cleanup_missing_package_skipped_with_ignore() {
        let (session, root) = setup("missing_ignored");
        install_versions(&root.0, "app", &["1.0.0"], "1.0.0");

        let outcome = cleanup(
            &session,
            &["nope".to_owned(), "app".to_owned()],
            &opts(true),
            &|_| {},
        )
        .unwrap();

        assert_eq!(
            outcome.apps.len(),
            1,
            "missing skipped, the present app visited"
        );
        assert_eq!(outcome.apps[0].name, "app");
        assert!(outcome.apps[0].removed.is_empty(), "nothing to clean");
    }

    /// A broken install (no resolvable current version) keeps its version dirs
    /// — hok deliberately skips an app it cannot judge — but `-k` still drops
    /// its cache: with no current version to exclude upstream's pattern
    /// degenerates to `"$app##*"`, matches nothing, and the whole app cache goes
    /// (`scoop-cleanup.ps1:32-33`).
    #[test]
    fn cleanup_broken_package_keeps_versions_but_drops_its_cache() {
        let (session, root) = setup("broken");
        // apps/app exists but has no `current` symlink
        std::fs::create_dir_all(root.0.join("apps/app/1.0.0")).unwrap();
        let cache = root.0.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        for name in ["app#1.0.0#a.zip", "other#1.0#c.zip"] {
            std::fs::write(cache.join(name), b"x").unwrap();
        }

        let outcome = cleanup(
            &session,
            &["app".to_owned()],
            &CleanupOptions {
                ignore_failures: false,
                cache: true,
            },
            &|_| {},
        )
        .unwrap();

        assert!(outcome.apps.is_empty(), "broken package skipped");
        assert_eq!(
            outcome
                .cache_removed
                .iter()
                .map(CacheFile::file_name)
                .collect::<Vec<_>>(),
            ["app#1.0.0#a.zip"],
            "a broken install must not pin its download cache"
        );
        assert!(!cache.join("app#1.0.0#a.zip").exists());
        assert!(
            cache.join("other#1.0#c.zip").exists(),
            "other apps' cache untouched"
        );
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
        // Real installs carry an install record in every version dir — that
        // file is the resolver's ordering key under NO_JUNCTION.
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

        let outcome = cleanup(&session, &["app".to_owned()], &opts(false), &|_| {}).unwrap();

        assert_eq!(outcome.apps.len(), 1);
        assert_eq!(outcome.apps[0].name, "app");
        assert_eq!(outcome.apps[0].removed.len(), 2);
        assert!(outcome.apps[0].failures.is_empty());
        assert!(pkg_dir.join("2.0.0").exists(), "newest version kept");
        assert!(!pkg_dir.join("1.0.0").exists(), "old version removed");
        assert!(!pkg_dir.join("_1.0.0.old").exists(), "old dir removed");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A sweep reports every planned version through the observer and keeps the
    /// results in the requested order — parallel workers must not leak their
    /// completion order into what the frontend renders.
    #[test]
    fn cleanup_reports_every_version_and_keeps_input_order() {
        let (session, root) = setup("parallel_order");
        let names: Vec<String> = ["zeta", "alpha", "mid", "beta"]
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        for name in &names {
            install_versions(&root.0, name, &["2.0.0", "1.5.0", "1.0.0"], "2.0.0");
        }

        let events = std::sync::Mutex::new(Vec::new());
        let outcome = cleanup(&session, &names, &opts(true), &|event| {
            events.lock().unwrap().push(event);
        })
        .unwrap();

        assert_eq!(
            outcome
                .apps
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            ["zeta", "alpha", "mid", "beta"],
            "results follow the requested order, not completion order"
        );
        for result in &outcome.apps {
            assert_eq!(result.removed, ["1.5.0", "1.0.0"], "newest first");
            assert!(result.failures.is_empty());
        }

        let events = events.into_inner().unwrap();
        assert_eq!(
            events.len(),
            16,
            "one Started + one Finished for each of the 8 planned versions"
        );
        let mut started: Vec<&str> = events
            .iter()
            .filter_map(|event| match event {
                CleanupProgress::Started { version, .. } => Some(version.as_str()),
                CleanupProgress::Finished { .. } => None,
            })
            .collect();
        let finished: Vec<&str> = events
            .iter()
            .filter_map(|event| match event {
                CleanupProgress::Finished {
                    version,
                    error: None,
                    ..
                } => Some(version.as_str()),
                _ => None,
            })
            .collect();
        started.sort();
        assert_eq!(
            started,
            [
                "1.0.0", "1.0.0", "1.0.0", "1.0.0", "1.5.0", "1.5.0", "1.5.0", "1.5.0"
            ]
        );
        assert_eq!(finished.len(), 8, "every started version settled clean");
    }

    /// A version directory that cannot be removed is reported with its version
    /// and the OS error, while the rest of the batch still completes.
    ///
    /// The lock is real: the handle is opened without `FILE_SHARE_DELETE`, the
    /// state any process that merely reads a file is in.
    #[cfg(windows)]
    #[test]
    fn cleanup_reports_a_locked_version_and_removes_the_rest() {
        use std::os::windows::fs::OpenOptionsExt;

        let (session, root) = setup("locked_version");
        install_versions(&root.0, "app", &["2.0.0", "1.5.0", "1.0.0"], "2.0.0");
        let locked = root.0.join("apps/app/1.0.0/locked.dll");
        std::fs::write(&locked, b"x").unwrap();
        // FILE_SHARE_READ only — without FILE_SHARE_DELETE the directory cannot go.
        let _lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1)
            .open(&locked)
            .unwrap();

        let outcome = cleanup(&session, &["app".to_owned()], &opts(true), &|_| {}).unwrap();

        assert_eq!(outcome.apps.len(), 1);
        assert_eq!(
            outcome.apps[0].removed,
            ["1.5.0"],
            "removable version still went"
        );
        assert_eq!(outcome.apps[0].failures.len(), 1);
        assert_eq!(outcome.apps[0].failures[0].version, "1.0.0");
        assert!(
            outcome.apps[0].failures[0].error.contains("os error"),
            "the OS error travels back in the result: {:?}",
            outcome.apps[0].failures[0].error
        );
        assert!(root.0.join("apps/app/2.0.0").exists(), "current kept");
        assert!(locked.exists(), "the locked file survived");
    }

    /// Strict mode (no `ignore_failures`) surfaces the removal failure as the
    /// command error instead of a result entry.
    #[cfg(windows)]
    #[test]
    fn cleanup_strict_mode_reports_the_failing_version() {
        use std::os::windows::fs::OpenOptionsExt;

        let (session, root) = setup("strict_locked");
        install_versions(&root.0, "app", &["2.0.0", "1.0.0"], "2.0.0");
        let locked = root.0.join("apps/app/1.0.0/locked.dll");
        std::fs::write(&locked, b"x").unwrap();
        let _lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1)
            .open(&locked)
            .unwrap();

        let err = cleanup(&session, &["app".to_owned()], &opts(false), &|_| {}).unwrap_err();

        assert!(
            matches!(err, Error::Custom(ref msg) if msg.contains("1.0.0")),
            "strict mode reports the failing version: {err:?}"
        );
    }
}
