//! Package query engine — search and filter packages across buckets.
//!
//! Provides functions to query available and installed packages by name,
//! description, or binary name, with regex support.
//!
//! # Design
//!
//! - **Parallel scanning**: Bucket manifests are scanned in parallel via
//!   `rayon` for performance. The manifest cache (`package::manifest_cache`)
//!   is checked first to avoid re-parsing unchanged files.
//! - **Flexible query options**: [`QueryOption`] controls search scope
//!   (`Description`, `Binary`, `Explicit`) and whether to include
//!   upgradability status (`Upgradable`, `UpgradableAll`).
//! - **Multi-bucket with isolation**: Installed packages from the
//!   `__isolated__` bucket (URL/path installs) are included alongside
//!   regular buckets.
//! - **Regex matching**: Package names are matched as regex patterns
//!   by default; `Explicit` mode disables regex for exact matching.
//!
//! The matcher machinery lives in [`query_matcher`], installed-state
//! resolution in [`install_state`]; this module holds the query walkers and
//! the session-level operations built on them.

use rayon::prelude::*;
use std::path::Path;
use tracing::debug;

use crate::{
    Session, constant::ISOLATED_PACKAGE_BUCKET, error::Fallible, package::manifest::parse_file,
};

use super::install_state::{
    fill_install_state, load_bucket_manifest, maybe_fill_upgradable, parse_install_info,
    select_current_version,
};
use super::query_matcher::{build_matchers, manifest_matches, name_prefiltered_out, QueryOption};
use super::{manifest_cache, InstallState, InstallStateInstalled, Manifest, Package};

/// Search installed packages.
pub fn query_installed(
    session: &Session,
    queries: &[&str],
    options: &[QueryOption],
) -> Fallible<Vec<Package>> {
    let is_explicit_mode = options.contains(&QueryOption::Explicit);
    let is_wildcard_query = queries.contains(&"*") || queries.is_empty();
    let apps_dir = session.apps_dir();
    let root_path = session.config().root_path().to_path_buf();
    let no_junction = session.config().no_junction();
    let matchers = build_matchers(queries, is_wildcard_query, is_explicit_mode)?;

    let mut ret = vec![];
    match apps_dir.read_dir() {
        Err(err) => {
            debug!("failed to read apps dir (err: {})", err);
        }
        Ok(entries) => {
            ret = entries
                .par_bridge()
                .filter_map(|item| {
                    if let Ok(e) = item {
                        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or_default();
                        let filename = e.file_name();
                        // Non-UTF-8 directory names cannot be package names —
                        // skip instead of panicking.
                        let name = filename.to_str()?;
                        // The name `scoop` is reserved for Scoop, ignore it
                        let is_scoop = name == "scoop";
                        // Broken installs cannot be upgraded, so under the
                        // `--upgradable` filter (QueryOption::Upgradable) they are
                        // excluded instead of surfacing alongside upgradable ones.
                        let include_broken = options.contains(&QueryOption::Broken)
                            && !options.contains(&QueryOption::Upgradable);

                        if !is_dir || is_scoop {
                            return None;
                        }

                        if name_prefiltered_out(name, is_wildcard_query, &matchers, options) {
                            return None;
                        }

                        let pkg_dir = apps_dir.join(name);
                        let meta_dir = if no_junction {
                            match select_current_version(&pkg_dir) {
                                Some(v) => pkg_dir.join(v),
                                None => {
                                    if include_broken {
                                        return broken_package(name, &pkg_dir);
                                    }
                                    return None;
                                }
                            }
                        } else {
                            pkg_dir.join("current")
                        };
                        let manifest_path = meta_dir.join("manifest.json");
                        let install_info_path = meta_dir.join("install.json");
                        let is_not_broken = manifest_path.exists() && install_info_path.exists();

                        if !is_not_broken {
                            if include_broken {
                                return broken_package(name, &pkg_dir);
                            }
                            return None;
                        }

                        if let Ok(manifest) = parse_file(manifest_path)
                            && let Ok(install_info) = parse_install_info(&install_info_path) {
                                // Noted that packages installed via URLs don't have
                                // bucket info in install info file. We mark them as
                                // isolated packages and use `ISOLATED_PACKAGE_BUCKET`
                                // as bucket name.
                                let bucket =
                                    install_info.bucket().unwrap_or(ISOLATED_PACKAGE_BUCKET);

                                if !manifest_matches(
                                    name,
                                    bucket,
                                    &manifest,
                                    is_wildcard_query,
                                    is_explicit_mode,
                                    &matchers,
                                    options,
                                ) {
                                    return None;
                                }

                                let current_version = manifest.version().to_owned();

                                let state = InstallState::Installed(InstallStateInstalled {
                                    version: current_version.clone(),
                                    bucket: install_info.bucket().map(|s| s.to_owned()),
                                    arch: install_info.arch().to_owned(),
                                    held: install_info.is_held(),
                                    url: install_info.url().map(|s| s.to_owned()),
                                });

                                let package = Package::from(name, bucket, manifest);
                                package.fill_install_state(state.clone());

                                // Mark when the origin bucket no longer exists.
                                if bucket != ISOLATED_PACKAGE_BUCKET
                                    && !internal_bucket_dir(&root_path, bucket).exists()
                                {
                                    package.mark_bucket_removed();
                                }

                                if !maybe_fill_upgradable(
                                    &root_path,
                                    &package,
                                    name,
                                    bucket,
                                    &current_version,
                                    &state,
                                    options,
                                ) {
                                    return None;
                                }

                                return Some(package);
                            } else if include_broken {
                                // manifest/install exist but failed to parse.
                                return broken_package(name, &pkg_dir);
                            }
                    }
                    None
                })
                .collect::<Vec<_>>();
        }
    }

    Ok(ret)
}


/// Build a placeholder package for a broken/partial install so `list` can
/// surface it as "Install failed" instead of silently dropping it.
///
/// Version and bucket are best-effort: the version directory name is used
/// as the version when resolvable, and the bucket is read from any
/// `install.json` found under `current/` (or the resolved version dir).
fn broken_package(name: &str, pkg_dir: &std::path::Path) -> Option<Package> {
    let current = select_current_version(pkg_dir);
    let version = current.clone().unwrap_or_default();

    // Best-effort bucket: prefer the resolved version dir, then `current`,
    // then any versioned subdirectory with a readable install.json.
    let mut candidates = Vec::new();
    if let Some(v) = current {
        candidates.push(pkg_dir.join(v));
    }
    candidates.push(pkg_dir.join("current"));
    if let Ok(entries) = std::fs::read_dir(pkg_dir) {
        for entry in entries.flatten() {
            if entry.path() == pkg_dir.join("current") {
                continue;
            }
            candidates.push(entry.path());
        }
    }
    let bucket = candidates
        .into_iter()
        .find_map(|dir| parse_install_info(&dir.join("install.json")).ok())
        .and_then(|info| info.bucket().map(|b| b.to_owned()))
        .unwrap_or_else(|| ISOLATED_PACKAGE_BUCKET.to_owned());

    // A minimal manifest carrying the (best-effort) version.
    let json = format!(r#"{{"version": "{}"}}"#, version);
    let manifest = Manifest::from_json(name, &json).ok()?;

    let package = Package::from(name, &bucket, manifest);
    package.mark_install_failed();
    Some(package)
}

/// The `<root>/buckets/<name>` directory (user-level root).
fn internal_bucket_dir(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    crate::internal::path::bucket_dir(root, name)
}

/// Search available packages.
///
/// Uses the SQLite manifest cache when enabled (the fast path for display
/// queries such as search / list --available); falls back to reading the
/// bucket files directly on a cache miss/error.
pub(crate) fn query_synced(
    session: &Session,
    queries: &[&str],
    options: &[QueryOption],
) -> Fallible<Vec<Package>> {
    // Fast path: use SQLite cache when enabled
    if session.config().use_sqlite_cache()
        && let Ok(packages) = query_synced_cached(session, queries, options)
    {
        return Ok(packages);
    }
    // Fall through to file-based on cache error
    query_synced_files(session, queries, options)
}

/// Search available packages by reading manifests directly from the bucket
/// files, bypassing the SQLite cache. Always fresh, never a stale cached
/// manifest. Operation-class resolution (install / update, including
/// dependency resolution) uses this so it never acts on stale data; the cache
/// is reserved for display-only queries.
pub(crate) fn query_synced_files(
    session: &Session,
    queries: &[&str],
    options: &[QueryOption],
) -> Fallible<Vec<Package>> {
    let is_explicit_mode = options.contains(&QueryOption::Explicit);
    let is_wildcard_query = queries.contains(&"*") || queries.is_empty();
    let buckets = crate::bucket::bucket_added(session)?;
    let apps_dir = session.apps_dir();
    let no_junction = session.config().no_junction();
    let matchers = build_matchers(queries, is_wildcard_query, is_explicit_mode)?;

    let packages = buckets
        .iter()
        .par_bridge()
        .filter_map(|bucket| {
            if let Ok(manifest_files) = bucket.manifests() {
                let bucket_packages = manifest_files
                    .into_iter()
                    .par_bridge()
                    .filter_map(|entry| {
                        let filename = entry.file_name();
                        // Non-UTF-8 manifest names are skipped (cannot be
                        // queried by name).
                        let name = filename.to_str()?.strip_suffix(".json")?;

                        if name_prefiltered_out(name, is_wildcard_query, &matchers, options) {
                            return None;
                        }

                        if let Ok(manifest) = parse_file(entry.path()) {
                            let bucket = bucket.name();

                            if !manifest_matches(
                                name,
                                bucket,
                                &manifest,
                                is_wildcard_query,
                                is_explicit_mode,
                                &matchers,
                                options,
                            ) {
                                return None;
                            }

                            let package = Package::from(name, bucket, manifest);
                            fill_install_state(&package, &apps_dir, name, no_junction);

                            return Some(package);
                        }
                        None
                    })
                    .collect::<Vec<_>>();

                return Some(bucket_packages);
            }
            None
        })
        .flatten()
        .collect::<Vec<_>>();

    Ok(packages)
}

/// `query_synced` backed by the SQLite manifest cache.
///
/// The cache supplies the package/bucket index and the stored manifest JSON,
/// so queries avoid a full directory walk *and* re-reading every manifest file
/// from disk (the dominant cost on large bucket sets). Manifests are served
/// from the cache, which is refreshed on every `hok update`; reading the file
/// (load_bucket_manifest) is a fallback when a cached entry fails to parse.
/// Falls back silently if the cache is unavailable.
fn query_synced_cached(
    session: &Session,
    queries: &[&str],
    options: &[QueryOption],
) -> Fallible<Vec<Package>> {
    let conn = manifest_cache::open(session)?;
    if !manifest_cache::is_populated(&conn)? {
        manifest_cache::populate(&conn, session)?;
    }

    let entries = manifest_cache::query(&conn, None, None)?;
    let apps_dir = session.apps_dir();
    let root_path = session.config().root_path().to_path_buf();
    let no_junction = session.config().no_junction();

    let is_explicit_mode = options.contains(&QueryOption::Explicit);
    let is_wildcard_query = queries.contains(&"*") || queries.is_empty();
    let matchers = build_matchers(queries, is_wildcard_query, is_explicit_mode)?;

    // Serve from the cache's stored manifest JSON, falling back to reading
    // the file only when a cached entry fails to parse. The cache is
    // (re)populated on every hok update, so within the normal workflow it is
    // fresh; re-reading 20k+ manifest files from disk on every query was the
    // dominant cost here. The scan is parallelized so per-package install-state
    // probing overlaps across entries.
    let packages = entries
        .par_iter()
        .filter_map(|entry| {
            let name = &entry.name;
            let bucket = &entry.bucket;

            if name_prefiltered_out(name, is_wildcard_query, &matchers, options) {
                return None;
            }

            let manifest = manifest_cache::entry_to_manifest(entry)
                .or_else(|| load_bucket_manifest(&root_path, bucket, name))?;

            if !manifest_matches(
                name,
                bucket,
                &manifest,
                is_wildcard_query,
                is_explicit_mode,
                &matchers,
                options,
            ) {
                return None;
            }

            let package = Package::from(name, bucket, manifest);
            fill_install_state(&package, &apps_dir, name, no_junction);

            Some(package)
        })
        .collect::<Vec<_>>();

    Ok(packages)
}

// ─── Session-level query operations ─────────────────────────────────────────

/// Query packages.
///
/// # Note
/// Set `installed` to `true` to query installed packages. The returned list
/// will be sorted by package name.
///
/// # Returns
///
/// A list of packages that match the query.
///
/// # Errors
///
/// I/O errors will be returned if the `apps`/`buckets` directory is not readable.
///
/// A [`Regex`][1] error will be returned if the given query is not a valid regex.
///
/// [1]: crate::Error::Regex
pub fn query(
    session: &Session,
    queries: Vec<&str>,
    options: Vec<QueryOption>,
    installed: bool,
) -> Fallible<Vec<Package>> {
    // remove possible duplicates
    let mut queries = std::collections::HashSet::<&str>::from_iter(queries)
        .into_iter()
        .collect::<Vec<_>>();

    if queries.is_empty() {
        queries.push("*");
    }

    let mut packages = if installed {
        query_installed(session, &queries, &options)?
    } else {
        query_synced(session, &queries, &options)?
    };

    packages.sort_by_key(|p| p.name().to_lowercase());

    Ok(packages)
}

/// Prune already-installed packages from a list of queries.
///
/// Returns `(to_install, already_installed)` where `to_install` contains
/// queries that are not yet installed, and `already_installed` contains
/// the names of packages that are already installed.
///
/// # Errors
///
/// Returns an error if the installed package list cannot be queried.
pub fn prune_installed<'s>(
    session: &Session,
    queries: &[&'s str],
) -> Fallible<(Vec<&'s str>, Vec<String>)> {
    // Normalize queries to bare app names before matching installed
    // packages, mirroring upstream `parse_app`: `app@version`, manifest
    // URLs and local paths are matched by their install name
    // (`appname_from_url` for URL/path installs).
    let bare_names: Vec<String> = queries.iter().map(|q| bare_query_name(q)).collect();

    let installed = query(
        session,
        bare_names.iter().map(String::as_str).collect::<Vec<_>>(),
        vec![QueryOption::Explicit],
        true,
    )?;

    let mut already_installed = Vec::new();
    let mut to_install = Vec::new();

    for (q, bare) in queries.iter().zip(&bare_names) {
        let installed_names: Vec<&str> = installed
            .iter()
            .filter(|p| {
                let q_normalized = bare.to_lowercase();
                let p_name = p.name().to_lowercase();
                let p_ident = p.ident().to_lowercase();
                q_normalized == p_name || q_normalized == p_ident
            })
            .map(|p| p.name())
            .collect();

        if installed_names.is_empty() {
            to_install.push(*q);
        } else {
            already_installed.push(installed_names[0].to_string());
        }
    }

    Ok((to_install, already_installed))
}

/// Reduce an install query to the bare app name used for installed-package
/// matching: strip a `@version` suffix and map URL / local-path manifests
/// to their last path segment without `.json`.
fn bare_query_name(q: &str) -> String {
    match super::identity::parse_app(q) {
        Some(aq) => {
            if super::identity::is_manifest_url(&aq.app) || Path::new(&aq.app).exists() {
                super::identity::appname_from_url(&aq.app)
            } else {
                aq.app
            }
        }
        None => q.to_owned(),
    }
}

/// A single unsatisfied `suggest` entry: the package that suggests it and
/// the candidate apps it recommends installing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestEntry {
    /// The package name that carries the `suggest` entry.
    pub package: String,
    /// The candidate apps the manifest suggests installing.
    pub candidates: Vec<String>,
}

/// Collect the unsatisfied `suggest` entries for the given packages,
/// matching Scoop's `show_suggestions` (lib/install.ps1): a feature is only
/// reported when **none** of its candidate apps is installed. Candidates are
/// matched by app name (bucket/name forms included) against installed apps
/// in both local and global scopes.
///
/// # Errors
///
/// Returns an error if querying the installed packages fails.
pub fn suggest(session: &Session, packages: &[&str]) -> Fallible<Vec<SuggestEntry>> {
    // Normalize URL / path / `@version` queries to bare app names, same as
    // `prune_installed`, so suggestions are looked up for the installed name.
    let bare = packages
        .iter()
        .map(|p| bare_query_name(p))
        .collect::<Vec<_>>();
    let installed = query(
        session,
        bare.iter().map(String::as_str).collect::<Vec<_>>(),
        vec![QueryOption::Explicit],
        true,
    )?;

    // All installed app names in both scopes, matching `installed_apps $true + $false`.
    let original_global = session.is_global();
    let mut installed_apps = std::collections::HashSet::new();
    for global in [false, true] {
        session.set_global(global);
        if let Ok(pkgs) = query(session, vec![], vec![], true) {
            installed_apps.extend(pkgs.into_iter().map(|p| p.name().to_owned()));
        }
    }
    session.set_global(original_global);

    let mut entries = Vec::new();
    for pkg in &installed {
        let manifest = pkg.manifest();
        if let Some(suggest) = manifest.suggest() {
            let name = pkg.name();
            for values in suggest.values() {
                let candidates: Vec<String> = values
                    .devectorize()
                    .into_iter()
                    .map(|s| s.to_string())
                    .collect();
                // A suggestion like "bucket/app" is fulfilled by installed "app".
                let fulfilled = candidates.iter().any(|s| {
                    let app = s.split(['/', '\\']).next_back().unwrap_or(s);
                    installed_apps.contains(app)
                });
                if !fulfilled {
                    entries.push(SuggestEntry {
                        package: name.to_string(),
                        candidates,
                    });
                }
            }
        }
    }
    Ok(entries)
}
