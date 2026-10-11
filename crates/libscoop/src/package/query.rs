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
use std::sync::Mutex;
use tracing::debug;

use crate::{
    Session, bucket::Bucket, constant::ISOLATED_PACKAGE_BUCKET, error::Fallible,
    package::identity::split_bucket_query, package::manifest::parse_file,
};

use super::install_state::{
    CurrentVersionSource, bucket_manifest_path, fill_install_state, install_info_path,
    installed_manifest_path, load_bucket_manifest, maybe_fill_upgradable, parse_install_info,
    resolve_current_version,
};
use super::query_matcher::{QueryOption, build_matchers, manifest_matches, name_prefiltered_out};
use super::{InstallState, InstallStateInstalled, Manifest, Package, manifest, manifest_cache};

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
                        // Resolve the installed version dir through the one
                        // shared resolver (upstream `Select-CurrentVersion`).
                        // In junction mode an intact `current` resolves to
                        // itself; when `current` carries no usable metadata —
                        // an interrupted upgrade left the junction pointing at
                        // an incomplete version dir — the resolver falls back
                        // to the newest version dir, exactly as upstream does,
                        // so the last good version is still reported as
                        // upgradable. The `VersionDirs` source is the signal
                        // that this happened: a junction install resolving
                        // that way is damaged. Under NO_JUNCTION it is the
                        // normal path and therefore not damage.
                        let resolved = resolve_current_version(&pkg_dir, no_junction);
                        let current_broken = !no_junction
                            && resolved
                                .as_ref()
                                .is_some_and(|r| r.source == CurrentVersionSource::VersionDirs);
                        let Some(resolved) = resolved else {
                            if include_broken {
                                return broken_package(name, &pkg_dir, no_junction);
                            }
                            return None;
                        };
                        // The metadata lives in the junction when the junction
                        // itself resolved, and in the resolved version dir
                        // otherwise (NO_JUNCTION, or a damaged `current` —
                        // upstream reads `installed_manifest` there).
                        let meta_dir = if no_junction
                            || resolved.source == CurrentVersionSource::VersionDirs
                        {
                            pkg_dir.join(&resolved.version)
                        } else {
                            pkg_dir.join("current")
                        };
                        if let (Some(manifest_path), Some(install_info_path)) = (
                            installed_manifest_path(&meta_dir),
                            install_info_path(&meta_dir),
                        ) && let Ok(manifest) = parse_file(manifest_path)
                            && let Ok(install_info) = parse_install_info(&install_info_path)
                        {
                            // Noted that packages installed via URLs don't have
                            // bucket info in install info file. We mark them as
                            // isolated packages and use `ISOLATED_PACKAGE_BUCKET`
                            // as bucket name.
                            let bucket = install_info.bucket().unwrap_or(ISOLATED_PACKAGE_BUCKET);

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

                            // A damaged install that resolved to a valid older
                            // version dir is still damaged: keep the install-failed
                            // flag (so `list` shows it) alongside the upgradable
                            // status. This makes `update *` / `list -u` offer the
                            // upgrade while `list` still reports the breakage.
                            if current_broken {
                                package.mark_install_failed();
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
                            return broken_package(name, &pkg_dir, no_junction);
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
/// install record (`scoop-install.json`, else `install.json`) found under
/// `current/` (or the resolved version dir).
fn broken_package(name: &str, pkg_dir: &std::path::Path, no_junction: bool) -> Option<Package> {
    let current = resolve_current_version(pkg_dir, no_junction).map(|resolved| resolved.version);
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
        .find_map(|dir| install_info_path(&dir).and_then(|path| parse_install_info(&path).ok()))
        .and_then(|info| info.bucket().map(|b| b.to_owned()))
        .unwrap_or_else(|| ISOLATED_PACKAGE_BUCKET.to_owned());

    // A minimal manifest carrying the (best-effort) version.
    let json = format!(r#"{{"version": "{}"}}"#, version);
    let manifest = Manifest::from_json(name, &json).ok()?;

    let package = Package::from(name, &bucket, manifest);
    package.mark_install_failed();
    Some(package)
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

// ─── Manifest parse failures ─────────────────────────────────────────────────

/// A manifest file that exists but does not parse, met while answering a query.
///
/// Plain data, so it can cross a rayon worker boundary; the notice is emitted
/// after the parallel section (the threading note in [`query_synced_cached`]
/// explains why). Without it a bucket file that fails to parse makes its
/// package vanish from the answer — the user then sees "not found", which is
/// simply false (official Scoop warns `Error parsing JSON at '<path>'.` and
/// returns `$null`, `lib/manifest.ps1:5-12`).
struct ParseFailure {
    bucket: String,
    name: String,
    /// The rendered notice (see [`manifest::parse_failure_message`]).
    message: String,
}

impl ParseFailure {
    fn new(bucket: &str, name: &str, path: &Path, err: &crate::Error) -> Self {
        ParseFailure {
            bucket: bucket.to_owned(),
            name: name.to_owned(),
            message: manifest::parse_failure_message(path, err),
        }
    }

    /// The once-per-session latch key. Keyed by the package, not the file, so
    /// the several paths that can meet the same broken manifest (an in-scan
    /// hit, the fallback behind a cache row, a by-name probe) report it once.
    fn latch_key(&self) -> String {
        format!("parse:{}/{}", self.bucket, self.name)
    }
}

/// Emit the parse-failure notices for one query walk.
///
/// **Named scope** — the query named specific package(s), so each unparsable
/// manifest is a fact the user asked about: one warning per package, with the
/// path and the real reason, *next to* the not-found error rather than instead
/// of it (the error and its exit code are unchanged).
///
/// **Full scan** — a bucket can carry dozens of unparsable manifests, nearly
/// all of them far stricter than official Scoop would be; listing them by
/// default is noise the user cannot act on. The walk reports one counted
/// summary instead and leaves the paths to `-vv` ([`Output::Detail`]).
fn emit_parse_failures(session: &Session, failures: Vec<ParseFailure>, full_scan: bool) {
    if failures.is_empty() {
        return;
    }

    if full_scan {
        session.output().warn(hok_i18n::t!(
            "cmd.manifest_parse_failed_summary",
            count = failures.len()
        ));
        for failure in &failures {
            session.output().detail(failure.message.clone());
        }
        return;
    }

    for failure in &failures {
        if session.mark_warned_once(&failure.latch_key()) {
            session.output().warn(failure.message.clone());
        }
    }
}

/// Probe the named queries' manifests on disk.
///
/// Bounded by design: one read per queried name per bucket, never a bucket
/// walk. This is what covers a broken manifest the cache-driven walk cannot
/// see at all — a file that fails to parse is never inserted into the cache
/// (`manifest_cache::insert_manifest_row`), so an explicit query under
/// `use_sqlite_cache` reaches it only by name.
fn named_parse_failures(buckets: &[Bucket], queries: &[&str]) -> Vec<ParseFailure> {
    let mut failures = vec![];
    for bucket in buckets {
        for query in queries {
            let (query_bucket, name) = split_bucket_query(query);
            if let Some(query_bucket) = query_bucket
                && !bucket.name().eq_ignore_ascii_case(&query_bucket)
            {
                continue;
            }
            let Some(path) = bucket.path_of_manifest(name) else {
                continue;
            };
            if let Err(err) = parse_file(&path) {
                failures.push(ParseFailure::new(bucket.name(), name, &path, &err));
            }
        }
    }
    failures
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

    // A full scan counts what it drops (one summary afterwards); a named query
    // instead probes its own names on disk, so a broken manifest is reported
    // as a warning even when the name prefilter would not have reached its
    // file (e.g. `search --with-binary`, which disables the prefilter and
    // would otherwise turn a scan of the whole bucket into per-file noise).
    let failures = Mutex::new(Vec::new());

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

                        match parse_file(entry.path()) {
                            Ok(manifest) => {
                                // Only real manifests: `version` is required, and a
                                // bucket directory can hold unrelated JSON
                                // (`.markdownlint.json`). Upstream skips such a file
                                // when it writes its DB (lib/database.ps1:176), and
                                // the cached path skips it too — the two must agree.
                                if manifest.version().is_empty() {
                                    return None;
                                }
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

                                Some(package)
                            }
                            Err(err) => {
                                if is_wildcard_query {
                                    failures
                                        .lock()
                                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                                        .push(ParseFailure::new(
                                            bucket.name(),
                                            name,
                                            &entry.path(),
                                            &err,
                                        ));
                                }
                                None
                            }
                        }
                    })
                    .collect::<Vec<_>>();

                return Some(bucket_packages);
            }
            None
        })
        .flatten()
        .collect::<Vec<_>>();

    let failures = if is_wildcard_query {
        failures
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    } else {
        named_parse_failures(&buckets, queries)
    };
    emit_parse_failures(session, failures, is_wildcard_query);

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
///
/// An **explicit hit** (one named package: `info`, `search -e`, `cat`, `home`,
/// `depends`) additionally re-reads the file behind the row and answers with
/// the version it declares. The cache only knows what the last `hok update`
/// saw, so a bucket changed outside it would otherwise report a version that
/// no longer exists; a row whose file has since been deleted is dropped.
fn query_synced_cached(
    session: &Session,
    queries: &[&str],
    options: &[QueryOption],
) -> Fallible<Vec<Package>> {
    let conn = manifest_cache::open(session)?;
    if !manifest_cache::is_populated(&conn)? {
        manifest_cache::populate(&conn, session)?;
    }

    // The rows below are served from the cache, which only `hok update`
    // refreshes: say so when the cache may trail the buckets, so the user can
    // act instead of trusting a stale version.
    manifest_cache::warn_if_cache_stale(session, &conn);

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
    //
    // Explicit hits are checked against their file (see the function doc): the
    // check is per *hit*, so it costs one file read per matched package, never
    // a walk over the bucket.
    let verify = is_explicit_mode && !is_wildcard_query;

    // Parse failures met while serving the rows; emitted after the parallel
    // section (see the comment there).
    let failures = Mutex::new(Vec::new());

    let checked = entries
        .par_iter()
        .filter_map(|entry| {
            let name = &entry.name;
            let bucket = &entry.bucket;

            if name_prefiltered_out(name, is_wildcard_query, &matchers, options) {
                return None;
            }

            let mut manifest = match manifest_cache::entry_to_manifest(entry) {
                Some(manifest) => manifest,
                // The stored JSON no longer parses. Fall back to the file, and
                // record the failure that leaves the row's package invisible
                // when the file cannot answer either.
                None => {
                    let path = bucket_manifest_path(&root_path, bucket, name)?;
                    match parse_file(&path) {
                        Ok(manifest) => manifest,
                        Err(err) => {
                            failures
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .push(ParseFailure::new(bucket, name, &path, &err));
                            return None;
                        }
                    }
                }
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
                return None;
            }

            let mismatch = if verify {
                match load_bucket_manifest(&root_path, bucket, name) {
                    Some(fresh) => {
                        if fresh.version() != manifest.version() {
                            let mismatch = manifest_cache::CacheMismatch::Version {
                                cached: manifest.version().to_owned(),
                                file: fresh.version().to_owned(),
                            };
                            // The file is the source of truth: serve it whole,
                            // so every field of the hit is as fresh as its
                            // version.
                            manifest = fresh;
                            Some(mismatch)
                        } else {
                            None
                        }
                    }
                    // The hit is a phantom row: its file is gone, so there is
                    // no package to return — but the disappearance is still
                    // worth reporting. A file that is *there* but does not
                    // parse is not a phantom: it is a parse failure, and must
                    // not be reported as "no manifest on disk".
                    None => {
                        if let Some(path) = bucket_manifest_path(&root_path, bucket, name)
                            && let Err(err) = parse_file(&path)
                        {
                            failures
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .push(ParseFailure::new(bucket, name, &path, &err));
                            return None;
                        }
                        return Some(CheckedHit::Phantom {
                            bucket: bucket.clone(),
                            name: name.clone(),
                        });
                    }
                }
            } else {
                None
            };

            let package = Package::from(name, bucket, manifest);
            fill_install_state(&package, &apps_dir, name, no_junction);

            Some(CheckedHit::Package(Box::new(package), mismatch))
        })
        .collect::<Vec<_>>();

    // The notices are emitted outside the parallel section: the rayon workers
    // carry plain values, not `&Session`. That is a data-threading choice, not
    // a thread-safety requirement — `Session` is `Sync + Send` since the
    // RwLock/AtomicBool/OnceLock refactor (see `session.rs`
    // `session_is_sync_send`).
    let mut packages = Vec::with_capacity(checked.len());
    for hit in checked {
        match hit {
            CheckedHit::Package(package, mismatch) => {
                if let Some(mismatch) = &mismatch {
                    manifest_cache::warn_if_cache_mismatch(
                        session,
                        package.bucket(),
                        package.name(),
                        mismatch,
                    );
                }
                packages.push(*package);
            }
            CheckedHit::Phantom { bucket, name } => {
                manifest_cache::warn_if_cache_mismatch(
                    session,
                    &bucket,
                    &name,
                    &manifest_cache::CacheMismatch::Missing,
                );
            }
        }
    }

    // A named query also probes its names on disk: a manifest that fails to
    // parse has no cache row at all, so the row-driven walk above cannot reach
    // it. Full scans report the rows that failed instead — walking the bucket
    // would defeat the cache they are served from.
    let mut failures = failures
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !is_wildcard_query {
        failures.extend(named_parse_failures(
            &crate::bucket::bucket_added(session)?,
            queries,
        ));
    }
    emit_parse_failures(session, failures, is_wildcard_query);

    Ok(packages)
}

/// One survivor of a cache-backed query, after the explicit-hit file check.
///
/// A phantom cache row produces no package but still has to be reported, which
/// is why this is not simply a `Package`.
enum CheckedHit {
    /// A package to return, plus the cache disagreement the check found.
    ///
    /// Boxed: a package dwarfs the phantom variant, which would otherwise make
    /// every hit as large as the largest one.
    Package(Box<Package>, Option<manifest_cache::CacheMismatch>),
    /// A cache row whose manifest file is gone.
    Phantom { bucket: String, name: String },
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

/// What [`prune_installed`] split a query list into.
///
/// A named struct rather than a tuple: the nested `Vec<(String, String)>` made
/// the signature trip `clippy::type_complexity`, and the fields read better at
/// the call site than positional indices.
pub struct PrunedInstall<'s> {
    /// Queries that are not installed yet, ready to hand to the installer.
    pub to_install: Vec<&'s str>,
    /// `(name, installed version)` for queries that are already installed —
    /// upstream names the installed version in its message
    /// (`libexec/scoop-install.ps1:83`).
    pub already_installed: Vec<(String, String)>,
}

/// Prune already-installed packages from a list of queries.
///
/// # Errors
///
/// Returns an error if the installed package list cannot be queried.
pub fn prune_installed<'s>(session: &Session, queries: &[&'s str]) -> Fallible<PrunedInstall<'s>> {
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
        let installed_matches: Vec<(&str, &str)> = installed
            .iter()
            .filter(|p| {
                let q_normalized = bare.to_lowercase();
                let p_name = p.name().to_lowercase();
                let p_ident = p.ident().to_lowercase();
                q_normalized == p_name || q_normalized == p_ident
            })
            .map(|p| (p.name(), p.version()))
            .collect();

        if let Some(&(name, version)) = installed_matches.first() {
            already_installed.push((name.to_owned(), version.to_owned()));
        } else {
            to_install.push(*q);
        }
    }

    Ok(PrunedInstall {
        to_install,
        already_installed,
    })
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
            // Upstream reports the features in `Get-Member` order, i.e.
            // case-insensitive alphabetical (lib/install.ps1:407) — unlike
            // `scoop info`, which walks the object's properties in manifest
            // order. `suggest` preserves declaration order, so sort here.
            let mut features = suggest.iter().collect::<Vec<_>>();
            features.sort_by(|(a, _), (b, _)| {
                a.to_lowercase()
                    .cmp(&b.to_lowercase())
                    .then_with(|| a.cmp(b))
            });
            for (_, values) in features {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{test_session, tmpdir, write_bucket_manifest};

    fn mini_manifest(version: &str) -> String {
        format!(
            r#"{{"version": "{}", "homepage": "https://example.com", "license": "MIT"}}"#,
            version
        )
    }

    /// A valid, fully-installed version dir under `apps/<name>/<version>`.
    fn install_version(root: &std::path::Path, name: &str, version: &str, bucket: &str) {
        let dir = root.join("apps").join(name).join(version);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.json"), mini_manifest(version)).unwrap();
        std::fs::write(
            dir.join("install.json"),
            format!(r#"{{"architecture": "64bit", "bucket": "{}"}}"#, bucket),
        )
        .unwrap();
    }

    /// Interrupted upgrade: the `current` junction (simulated here as a plain
    /// `current` dir) points at an incomplete version dir with **no**
    /// metadata, while a valid older version dir remains. Scoop resolves the
    /// installed version via `Select-CurrentVersion` fallback (newest version
    /// dir with install.json) and reports the app outdated; hok must do the
    /// same and surface it under `Upgradable` instead of dropping it.
    #[test]
    fn query_installed_upgradable_survives_broken_current_junction() {
        let root = tmpdir("query_broken_current_upgrade");
        let session = test_session(&root);

        // The incomplete new version dir (no manifest/install metadata).
        std::fs::create_dir_all(root.join("apps/Fiddler/6.0.20261.7291")).unwrap();
        // Simulate `current` junction -> the incomplete dir (no metadata).
        std::fs::create_dir_all(root.join("apps/Fiddler/current")).unwrap();
        // The valid older installed version.
        install_version(&root, "Fiddler", "5.0.20262.6151", "extras");
        // Bucket offers the newer version.
        write_bucket_manifest(&root, "extras", "Fiddler", &mini_manifest("6.0.20261.7291"));

        let pkgs = query_installed(&session, &["*"], &[QueryOption::Upgradable]).unwrap();
        let fiddler = pkgs.iter().find(|p| p.name() == "Fiddler");
        assert!(
            fiddler.is_some(),
            "app with broken current junction but valid older version must be upgradable"
        );
        let f = fiddler.unwrap();
        assert_eq!(f.installed_version(), Some("5.0.20262.6151"));
        assert_eq!(f.upgradable_version(), Some("6.0.20261.7291"));
        // The broken `current` junction is real damage: `list` must still
        // flag the app install-failed even though an older version resolves
        // and an upgrade is available.
        assert!(f.is_install_failed(), "damaged install must stay flagged");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A version dir carrying ONLY the Scoop >= 0.6.0 metadata names
    /// (`scoop-manifest.json` + `scoop-install.json`) is reported as installed
    /// with the right version and bucket. Regression: such an app used to be
    /// invisible to `list`/`status`/`update`.
    #[test]
    fn query_installed_reads_scoop_prefixed_metadata_only() {
        let root = tmpdir("query_scoop_names_only");
        let session = test_session(&root);
        let dir = root.join("apps/App").join("2.0.0");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("scoop-manifest.json"), mini_manifest("2.0.0")).unwrap();
        std::fs::write(
            dir.join("scoop-install.json"),
            r#"{"architecture": "64bit", "bucket": "extras"}"#,
        )
        .unwrap();
        crate::internal::fs::symlink_dir(&dir, root.join("apps/App/current")).unwrap();

        let pkgs = query_installed(&session, &["*"], &[]).unwrap();
        let app = pkgs
            .iter()
            .find(|p| p.name() == "App")
            .expect("a new-name-only install must be visible");
        assert_eq!(app.installed_version(), Some("2.0.0"));
        assert_eq!(app.installed_bucket(), Some("extras"));
        assert!(
            !app.is_install_failed(),
            "a complete new-name install is not broken"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The pre-0.6.0 names keep working: a version dir carrying ONLY
    /// `manifest.json` + `install.json` is still read (read-side fallback; the
    /// old names are not renamed or dropped).
    #[test]
    fn query_installed_reads_legacy_metadata_only() {
        let root = tmpdir("query_legacy_names_only");
        let session = test_session(&root);
        crate::test_utils::mark_installed_legacy(
            &root,
            "old",
            "main",
            &mini_manifest("1.0.0"),
            false,
        );
        let current = root.join("apps/old/current");
        assert!(current.join("manifest.json").exists());
        assert!(current.join("install.json").exists());
        assert!(!current.join("scoop-manifest.json").exists());

        let pkgs = query_installed(&session, &["*"], &[]).unwrap();
        let app = pkgs
            .iter()
            .find(|p| p.name() == "old")
            .expect("a legacy-name install must stay visible");
        assert_eq!(app.installed_version(), Some("1.0.0"));
        assert_eq!(app.installed_bucket(), Some("main"));
        assert!(!app.is_install_failed());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The install report walks `suggest` the way upstream's `Get-Member` does:
    /// case-insensitive alphabetical, not manifest order and not
    /// case-sensitive.
    #[test]
    fn suggest_reports_features_in_get_member_order() {
        let root = tmpdir("query_suggest_order");
        let session = test_session(&root);
        let manifest = r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "suggest": {"vcredist": ["extras/vcredist"], "Boost": ["boost"], "alpha": ["extras/alpha"]}
            }"#;
        write_bucket_manifest(&root, "extras", "app", manifest);
        crate::test_utils::mark_installed(&root, "app", "extras", manifest, false);

        let entries = suggest(&session, &["app"]).unwrap();
        let features = entries
            .iter()
            .map(|entry| entry.candidates[0].as_str())
            .collect::<Vec<_>>();
        // Case-insensitive: alpha, Boost, vcredist (manifest declares
        // vcredist first; a case-sensitive sort would put Boost first).
        assert_eq!(features, ["extras/alpha", "boost", "extras/vcredist"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    // ── Manifest parse-failure visibility ───────────────────────────────────

    /// Install a sink that records everything the session emits.
    fn record_output(
        session: &Session,
    ) -> std::sync::Arc<std::sync::Mutex<Vec<crate::output::Output>>> {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        session
            .set_output(std::sync::Arc::new(move |out| {
                sink.lock().unwrap().push(out)
            }))
            .unwrap();
        seen
    }

    fn outputs_of(
        seen: &std::sync::Mutex<Vec<crate::output::Output>>,
        pick: impl Fn(&crate::output::Output) -> Option<String>,
    ) -> Vec<String> {
        seen.lock().unwrap().iter().filter_map(pick).collect()
    }

    /// The parse-failure warnings a sink received (the per-path notices).
    fn parse_warnings(seen: &std::sync::Mutex<Vec<crate::output::Output>>) -> Vec<String> {
        outputs_of(seen, |out| match out {
            crate::output::Output::Warn(msg) if msg.contains("Error parsing JSON at '") => {
                Some(msg.clone())
            }
            _ => None,
        })
    }

    /// Every warning a sink received.
    fn warnings_of(seen: &std::sync::Mutex<Vec<crate::output::Output>>) -> Vec<String> {
        outputs_of(seen, |out| match out {
            crate::output::Output::Warn(msg) => Some(msg.clone()),
            _ => None,
        })
    }

    /// A named query that finds the broken file reports one warning naming the
    /// file and the real reason — `install` / `update` / `depends` reach the
    /// walk with empty options, exactly like this call.
    #[test]
    fn a_named_query_reports_an_unparsable_manifest_once() {
        let root = tmpdir("query_named_parse_failure");
        let session = test_session(&root);
        write_bucket_manifest(&root, "main", "good", &mini_manifest("1.0.0"));
        write_bucket_manifest(&root, "main", "broken", "{ not json");

        let seen = record_output(&session);
        let packages = query_synced_files(&session, &["broken"], &[]).unwrap();
        assert!(packages.is_empty(), "a broken manifest yields no package");
        assert_eq!(packages.len(), 0);

        let warnings = parse_warnings(&seen);
        assert_eq!(warnings.len(), 1, "exactly one warning: {warnings:?}");
        assert!(warnings[0].contains("broken.json"), "{}", warnings[0]);
        assert!(
            warnings[0].contains("main"),
            "the path identifies the bucket: {}",
            warnings[0]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A full scan never prints one line per broken manifest: it emits a single
    /// counted summary, and the paths are `-vv` details ([`Output::Detail`]).
    #[test]
    fn a_wildcard_scan_reports_one_counted_summary() {
        let root = tmpdir("query_wildcard_parse_summary");
        let session = test_session(&root);
        write_bucket_manifest(&root, "main", "good", &mini_manifest("1.0.0"));
        write_bucket_manifest(&root, "main", "broken_a", "{ not json");
        write_bucket_manifest(&root, "main", "broken_b", "[1, 2");

        let seen = record_output(&session);
        let packages = query_synced_files(&session, &["*"], &[]).unwrap();
        assert_eq!(packages.len(), 1, "only the healthy manifest is listed");

        let warnings = warnings_of(&seen);
        assert_eq!(warnings.len(), 1, "one summary, not one line per file");
        assert!(
            warnings[0].contains("could not be parsed"),
            "the summary must count both failures: {}",
            warnings[0]
        );
        assert!(
            warnings[0].contains('2'),
            "the summary must count both failures: {}",
            warnings[0]
        );

        let details = outputs_of(&seen, |out| match out {
            crate::output::Output::Detail(msg) => Some(msg.clone()),
            _ => None,
        });
        assert_eq!(details.len(), 2, "the paths are -vv details: {details:?}");
        assert!(
            details
                .iter()
                .all(|d| d.contains("Error parsing JSON at '"))
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A bucket whose manifests all parse adds nothing at all: the visibility
    /// must not cost the healthy path a single byte (named or wildcard).
    #[test]
    fn a_healthy_bucket_emits_no_new_output() {
        let root = tmpdir("query_healthy_silent");
        let session = test_session(&root);
        write_bucket_manifest(&root, "main", "good", &mini_manifest("1.0.0"));

        let seen = record_output(&session);
        assert_eq!(
            query_synced_files(&session, &["good"], &[]).unwrap().len(),
            1
        );
        assert_eq!(query_synced_files(&session, &["*"], &[]).unwrap().len(), 1);

        let all: Vec<_> = seen.lock().unwrap().clone();
        assert!(
            all.is_empty(),
            "a healthy bucket must emit nothing, got {all:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
