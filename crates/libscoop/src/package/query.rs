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

use rayon::prelude::{ParallelBridge, ParallelIterator};
use regex::{Regex, RegexBuilder};
use std::path::Path;
use tracing::{debug, info};

use crate::{
    bucket::Bucket,
    constant::ISOLATED_PACKAGE_BUCKET,
    error::Fallible,
    internal::compare_versions,
    package::manifest::{InstallInfo, Manifest},
    Session,
};

use super::{manifest_cache, InstallState, InstallStateInstalled, Package};

/// Options that may be used to query Scoop packages.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum QueryOption {
    /// Enable query through package binaries.
    Binary,

    /// Enable query through package description.
    Description,

    /// Explicit mode. Regex is disabled in this mode.
    ///
    /// Query will be performed through the package name only. `Description`
    /// and `Binary` options will be ignored.
    Explicit,

    /// Additionally check if the matched package is upgradable.
    ///
    /// This option only takes effect on querying installed packages.
    Upgradable,

    /// Check upgradable status without filtering out non-upgradable packages.
    ///
    /// Like `Upgradable` but does NOT exclude packages that are already
    /// at the latest version.
    UpgradableCheck,
}

/// A trait represents a matcher that can be used to do string matching.
trait Matcher {
    fn is_match(&self, s: &str) -> bool;
}

/// A matcher that does explicit match.
///
/// # Note
///
/// This matcher is case-insensitive.
struct ExplicitMatcher<'a>(&'a str);

/// A matcher that does regex match.
struct RegexMatcher(Regex);

impl Matcher for ExplicitMatcher<'_> {
    fn is_match(&self, s: &str) -> bool {
        self.0.eq_ignore_ascii_case(s)
    }
}

impl Matcher for RegexMatcher {
    fn is_match(&self, s: &str) -> bool {
        self.0.is_match(s)
    }
}

type QueryMatchers<'a> = Vec<(Option<String>, Box<dyn Matcher + Send + Sync + 'a>)>;

fn has_extra_query(options: &[QueryOption]) -> bool {
    options.contains(&QueryOption::Binary) || options.contains(&QueryOption::Description)
}

fn build_matchers<'a>(
    queries: &[&'a str],
    is_wildcard_query: bool,
    is_explicit_mode: bool,
) -> Fallible<QueryMatchers<'a>> {
    if is_wildcard_query {
        return Ok(vec![]);
    }

    let mut matchers: QueryMatchers<'a> = vec![];
    for query in queries {
        let (bucket_prefix, name) = super::identity::split_bucket_query(query);

        if is_explicit_mode {
            matchers.push((bucket_prefix, Box::new(ExplicitMatcher(name))));
        } else {
            let re = RegexBuilder::new(name)
                .case_insensitive(true)
                .multi_line(true)
                .build()?;
            matchers.push((bucket_prefix, Box::new(RegexMatcher(re))));
        }
    }

    Ok(matchers)
}

fn name_prefiltered_out(
    name: &str,
    is_wildcard_query: bool,
    matchers: &QueryMatchers<'_>,
    options: &[QueryOption],
) -> bool {
    !is_wildcard_query
        && !has_extra_query(options)
        && !matchers.iter().any(|(_, matcher)| matcher.is_match(name))
}

fn manifest_matches(
    name: &str,
    bucket: &str,
    manifest: &Manifest,
    is_wildcard_query: bool,
    is_explicit_mode: bool,
    matchers: &QueryMatchers<'_>,
    options: &[QueryOption],
) -> bool {
    if is_wildcard_query {
        return true;
    }

    let prefixed_name_matched = matchers
        .iter()
        .filter(|(_, matcher)| matcher.is_match(name))
        .any(|(prefix, _)| prefix.is_none() || prefix.as_deref().unwrap() == bucket);

    if prefixed_name_matched {
        return true;
    }

    if is_explicit_mode {
        return false;
    }

    if options.contains(&QueryOption::Description)
        && matchers.iter().any(|(_, matcher)| {
            manifest
                .description()
                .map(|description| matcher.is_match(description))
                .unwrap_or(false)
        })
    {
        return true;
    }

    if options.contains(&QueryOption::Binary) {
        let binaries = manifest.shims().unwrap_or_default();
        if matchers
            .iter()
            .any(|(_, matcher)| binaries.iter().any(|binary| matcher.is_match(binary)))
        {
            return true;
        }
    }

    false
}

fn load_install_state(apps_dir: &Path, name: &str) -> Option<InstallState> {
    let mut path = apps_dir.join(name);
    path.push("current");
    path.push("install.json");

    let install_info = InstallInfo::parse(&path).ok()?;
    path.pop();
    path.push("manifest.json");
    let install_manifest = Manifest::parse(path).ok()?;

    Some(InstallState::Installed(InstallStateInstalled {
        version: install_manifest.version().to_owned(),
        bucket: install_info.bucket().map(|s| s.to_owned()),
        arch: install_info.arch().to_owned(),
        held: install_info.is_held(),
        url: install_info.url().map(|s| s.to_owned()),
    }))
}

fn fill_install_state(package: &Package, apps_dir: &Path, name: &str) {
    package.fill_install_state(
        load_install_state(apps_dir, name).unwrap_or(InstallState::NotInstalled),
    );
}

fn load_bucket_manifest(root_path: &Path, bucket: &str, name: &str) -> Option<Manifest> {
    let bucket_path = crate::internal::path::bucket_dir(root_path, bucket);
    let bucket = Bucket::from(&bucket_path).ok()?;
    let manifest_path = bucket.path_of_manifest(name)?;
    Manifest::parse(manifest_path).ok()
}

fn maybe_fill_upgradable(
    root_path: &Path,
    package: &Package,
    name: &str,
    bucket: &str,
    current_version: &str,
    state: &InstallState,
    options: &[QueryOption],
) -> bool {
    let filter_non_upgradable = options.contains(&QueryOption::Upgradable);
    let check_upgradable = filter_non_upgradable || options.contains(&QueryOption::UpgradableCheck);

    if !check_upgradable {
        return true;
    }

    if bucket == ISOLATED_PACKAGE_BUCKET {
        if filter_non_upgradable {
            info!("ignored isolated package '{}'", name);
            return false;
        }
        return true;
    }

    let Some(origin_manifest) = load_bucket_manifest(root_path, bucket, name) else {
        return !filter_non_upgradable;
    };

    let is_upgradable =
        compare_versions(origin_manifest.version(), current_version) == std::cmp::Ordering::Greater;

    if !is_upgradable {
        return !filter_non_upgradable;
    }

    let origin_pkg = Package::from(name, bucket, origin_manifest);
    origin_pkg.fill_install_state(state.clone());
    package.fill_upgradable(origin_pkg);
    true
}

/// Search installed packages.
pub(crate) fn query_installed(
    session: &Session,
    queries: &[&str],
    options: &[QueryOption],
) -> Fallible<Vec<Package>> {
    let is_explicit_mode = options.contains(&QueryOption::Explicit);
    let is_wildcard_query = queries.contains(&"*") || queries.is_empty();
    let apps_dir = session.apps_dir();
    let root_path = session.config().root_path().to_path_buf();
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
                        let name = filename.to_str().unwrap();
                        // The name `scoop` is reserved for Scoop, ignore it
                        let is_scoop = name == "scoop";
                        let manifest_path = e.path().join("current/manifest.json");
                        let install_info_path = e.path().join("current/install.json");
                        let is_not_broken = manifest_path.exists() && install_info_path.exists();

                        if !is_dir || is_scoop || !is_not_broken {
                            return None;
                        }

                        if name_prefiltered_out(name, is_wildcard_query, &matchers, options) {
                            return None;
                        }

                        if let Ok(manifest) = Manifest::parse(manifest_path) {
                            if let Ok(install_info) = InstallInfo::parse(install_info_path) {
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
                            }
                        }
                    }
                    None
                })
                .collect::<Vec<_>>();
        }
    }

    Ok(ret)
}

/// Search available packages.
pub(crate) fn query_synced(
    session: &Session,
    queries: &[&str],
    options: &[QueryOption],
) -> Fallible<Vec<Package>> {
    // Fast path: use SQLite cache when enabled
    if session.config().use_sqlite_cache() {
        if let Ok(packages) = query_synced_cached(session, queries, options) {
            return Ok(packages);
        }
        // Fall through to file-based on cache error
    }

    let is_explicit_mode = options.contains(&QueryOption::Explicit);
    let is_wildcard_query = queries.contains(&"*") || queries.is_empty();
    let buckets = crate::bucket::bucket_added(session)?;
    let apps_dir = session.apps_dir();
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
                        let name = filename.to_str().unwrap().strip_suffix(".json").unwrap();

                        if name_prefiltered_out(name, is_wildcard_query, &matchers, options) {
                            return None;
                        }

                        if let Ok(manifest) = Manifest::parse(entry.path()) {
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
                            fill_install_state(&package, &apps_dir, name);

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

/// Fast version of `query_synced` using SQLite manifest cache.
///
/// Avoids reading and parsing thousands of JSON files on every command.
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

    let is_explicit_mode = options.contains(&QueryOption::Explicit);
    let is_wildcard_query = queries.contains(&"*") || queries.is_empty();
    let matchers = build_matchers(queries, is_wildcard_query, is_explicit_mode)?;

    let mut packages = Vec::new();

    for entry in &entries {
        let name = &entry.name;
        let bucket = &entry.bucket;

        if name_prefiltered_out(name, is_wildcard_query, &matchers, options) {
            continue;
        }

        // Prefer reading manifest files directly to avoid stale cache.
        // Cache is populated once during bucket update, but the user
        // may have edited the file since then.
        let manifest = match load_bucket_manifest(&root_path, bucket, name)
            .or_else(|| manifest_cache::entry_to_manifest(entry))
        {
            Some(m) => m,
            None => continue,
        };

        if !manifest_matches(
            name,
            bucket,
            &manifest,
            is_wildcard_query,
            is_explicit_mode,
            &matchers,
            options,
        ) {
            continue;
        }

        let package = Package::from(name, bucket, manifest);
        fill_install_state(&package, &apps_dir, name);

        packages.push(package);
    }

    Ok(packages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::manifest::Manifest;

    // ── build_matchers ───────────────────────────────────────────────────────

    #[test]
    fn wildcard_query_yields_empty_matchers() {
        let matchers = build_matchers(&["*"], true, false).unwrap();
        assert!(matchers.is_empty());
    }

    #[test]
    fn explicit_mode_builds_explicit_matcher() {
        let matchers = build_matchers(&["curl"], false, true).unwrap();
        assert_eq!(matchers.len(), 1);
    }

    #[test]
    fn regex_mode_builds_regex_matcher() {
        let matchers = build_matchers(&["curl"], false, false).unwrap();
        assert_eq!(matchers.len(), 1);
    }

    #[test]
    fn bucket_prefixed_query_is_parsed() {
        let matchers = build_matchers(&["extras/curl"], false, true).unwrap();
        assert_eq!(matchers.len(), 1);
        let (prefix, _) = &matchers[0];
        assert_eq!(prefix.as_deref(), Some("extras"));
    }

    #[test]
    fn backslash_bucket_prefixed_query_is_parsed() {
        let matchers = build_matchers(&["extras\\curl"], false, true).unwrap();
        assert_eq!(matchers.len(), 1);
        let (prefix, _) = &matchers[0];
        assert_eq!(prefix.as_deref(), Some("extras"));
    }

    #[test]
    fn leading_backslash_is_not_a_separator() {
        // A regex escape like `\d` must not be split into a bucket prefix.
        let matchers = build_matchers(&["\\d"], false, false).unwrap();
        assert_eq!(matchers.len(), 1);
        let (prefix, _) = &matchers[0];
        assert!(prefix.is_none(), "leading backslash must not split bucket");
    }

    // ── name_prefiltered_out ─────────────────────────────────────────────────

    #[test]
    fn wildcard_never_prefilters() {
        let matchers = build_matchers(&[], true, false).unwrap();
        assert!(!name_prefiltered_out("anything", true, &matchers, &[]));
    }

    #[test]
    fn explicit_match_is_not_prefiltered() {
        let matchers = build_matchers(&["curl"], false, true).unwrap();
        assert!(!name_prefiltered_out("curl", false, &matchers, &[]));
    }

    #[test]
    fn non_match_is_prefiltered_when_no_extra_options() {
        let matchers = build_matchers(&["curl"], false, true).unwrap();
        assert!(name_prefiltered_out("wget", false, &matchers, &[]));
    }

    #[test]
    fn description_option_prevents_prefilter() {
        let matchers = build_matchers(&["downloader"], false, false).unwrap();
        // With description option, name prefilter should NOT apply
        let options = vec![QueryOption::Description];
        assert!(!name_prefiltered_out("curl", false, &matchers, &options));
    }

    // ── manifest_matches ─────────────────────────────────────────────────────

    fn make_manifest(version: &str) -> Manifest {
        let json = format!(
            r#"{{"version":"{version}","homepage":"https://example.com","license":"MIT"}}"#
        );
        Manifest::from_json("test", &json).unwrap()
    }

    #[test]
    fn wildcard_matches_all() {
        let m = make_manifest("1.0");
        let matchers = build_matchers(&[], true, false).unwrap();
        assert!(manifest_matches(
            "any-pkg",
            "bucket",
            &m,
            true,
            false,
            &matchers,
            &[]
        ));
    }

    #[test]
    fn explicit_exact_match() {
        let m = make_manifest("1.0");
        let matchers = build_matchers(&["curl"], false, true).unwrap();
        assert!(manifest_matches(
            "curl",
            "main",
            &m,
            false,
            true,
            &matchers,
            &[]
        ));
    }

    #[test]
    fn explicit_no_match() {
        let m = make_manifest("1.0");
        let matchers = build_matchers(&["curl"], false, true).unwrap();
        assert!(!manifest_matches(
            "wget",
            "main",
            &m,
            false,
            true,
            &matchers,
            &[]
        ));
    }

    #[test]
    fn regex_case_insensitive_match() {
        let m = make_manifest("1.0");
        let matchers = build_matchers(&["CURL"], false, false).unwrap();
        assert!(manifest_matches(
            "curl",
            "main",
            &m,
            false,
            false,
            &matchers,
            &[]
        ));
    }

    #[test]
    fn bucket_prefixed_query_only_matches_correct_bucket() {
        let m = make_manifest("1.0");
        let matchers = build_matchers(&["extras/curl"], false, true).unwrap();
        // Matches when bucket is "extras"
        assert!(manifest_matches(
            "curl",
            "extras",
            &m,
            false,
            true,
            &matchers,
            &[]
        ));
        // Does NOT match when bucket is "main"
        assert!(!manifest_matches(
            "curl",
            "main",
            &m,
            false,
            true,
            &matchers,
            &[]
        ));
    }
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
    let installed = query(session, queries.to_vec(), vec![QueryOption::Explicit], true)?;

    let mut already_installed = Vec::new();
    let mut to_install = Vec::new();

    for q in queries {
        let installed_names: Vec<&str> = installed
            .iter()
            .filter(|p| {
                let q_normalized = q.to_lowercase();
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
    let installed = query(
        session,
        packages.to_vec(),
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
