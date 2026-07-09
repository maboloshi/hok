//! Operations that can be performed on a Scoop instance.
//!
//! This module contains publicly available operations that can be executed on
//! a Scoop session. Certain operations may read or write Scoop's data, hence
//! a session is required to perform these functions.
//!
//! # Note
//!
//! operations with description ending with `*` alter the config.
//!
//! # Examples
//!
//! ```rust
//! use libscoop::{Session, operation};
//! let session = Session::new();
//! let buckets = operation::bucket_list(&session).expect("failed to get buckets");
//! println!("{} bucket(s)", buckets.len());
//! ```
use std::{
    collections::HashSet,
    iter::FromIterator,
    path::Path,
    sync::{Arc, Mutex},
};
use tracing::debug;

use crate::{
    bucket::{Bucket, BucketUpdateProgressContext},
    cache::CacheFile,
    error::{Error, Fallible},
    event::Event,
    internal, package,
    package::{InstallInfo, Package, QueryOption},
    Session, SyncOption,
};

/// Add a bucket to Scoop.
///
/// # Errors
///
/// This method will return an error if the bucket already exists, or the remote
/// url is not specified when adding a non built-in bucket.
///
/// A git error will be returned if failed to clone the bucket.
pub fn bucket_add(session: &Session, name: &str, remote_url: &str) -> Fallible<()> {
    let config = session.config();
    let mut path = config.root_path().to_owned();
    path.push("buckets");

    internal::fs::ensure_dir(&path)?;

    path.push(name);
    if path.exists() {
        return Err(Error::BucketAlreadyExists(name.to_owned()));
    }

    let proxy = config.proxy();
    let remote_url = match remote_url.is_empty() {
        false => remote_url,
        true => crate::constant::BUILTIN_BUCKET_LIST
            .iter()
            .find(|&&(n, _)| n == name)
            .map(|&(_, remote)| remote)
            .ok_or_else(|| Error::BucketAddRemoteRequired(name.to_owned()))?,
    };

    internal::git::clone_repo(remote_url, path, proxy)
}

/// Get a list of added buckets.
///
/// # Returns
///
/// A list of added buckets sorted by name.Buckets cannot be parsed will be
/// filtered out.
///
/// # Errors
///
/// I/O errors will be returned if the `buckets` directory is not readable.
pub fn bucket_list(session: &Session) -> Fallible<Vec<Bucket>> {
    crate::bucket::bucket_added(session).map(|mut buckets| {
        buckets.sort_by_key(|b| b.name().to_owned());
        buckets
    })
}

/// Get a list of known (built-in) buckets.
///
/// # Returns
///
/// A list of known buckets.
pub fn bucket_list_known() -> Vec<(&'static str, &'static str)> {
    crate::constant::BUILTIN_BUCKET_LIST.to_vec()
}

/// Update all added buckets. *
///
/// # Errors
///
/// I/O errors will be returned if the `buckets` directory is not readable or
/// failed to start up the update threads.
///
/// A [`ConfigInUse`][1] error will be returned if the config is borrowed elsewhere.
///
/// [1]: crate::Error::ConfigInUse
pub fn bucket_update(session: &Session) -> Fallible<()> {
    let buckets = crate::bucket::bucket_added(session)?;

    if buckets.is_empty() {
        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::BucketUpdateDone);
        }

        return Ok(());
    }

    // Doing bucket update will update the last_update timestamp in the config.
    // A mutable reference to the config is borrowed here.
    let mut config = session.config_mut()?;
    let any_bucket_updated = Arc::new(Mutex::new(false));
    let proxy = config.proxy().map(|s| s.to_owned());
    let emitter = session.emitter();

    let handles: Vec<_> = buckets
        .iter()
        .filter(|b| b.remote_url().is_some())
        .map(|bucket| {
            let repo = bucket.path().to_owned();
            let name = bucket.name().to_owned();
            let flag = Arc::clone(&any_bucket_updated);
            let proxy = proxy.clone();
            let emitter = emitter.clone();

            std::thread::spawn(move || {
                let mut ctx = BucketUpdateProgressContext::new(name.as_str());

                if let Some(tx) = emitter.clone() {
                    let _ = tx.send(Event::BucketUpdateProgress(ctx.clone()));
                }

                match internal::git::reset_head(repo, proxy) {
                    Ok(_) => {
                        *flag.lock().unwrap() = true;

                        if let Some(tx) = emitter {
                            ctx.set_succeeded();
                            let _ = tx.send(Event::BucketUpdateProgress(ctx));
                        }
                    }
                    Err(err) => {
                        if let Some(tx) = emitter {
                            ctx.set_failed(err.to_string().as_str());
                            let _ = tx.send(Event::BucketUpdateProgress(ctx));
                        }
                    }
                };
            })
        })
        .collect();

    for handle in handles {
        let _ = handle.join();
    }

    if *any_bucket_updated.lock().unwrap() {
        let time = jiff::Timestamp::now().to_string();
        config.set("last_update", time.as_str())?;
    }

    if let Some(tx) = emitter {
        let _ = tx.send(Event::BucketUpdateDone);
    }

    // Refresh SQLite manifest cache after bucket update
    // Refresh SQLite manifest cache after bucket update
    if session.config().use_sqlite_cache() {
        if let Ok(conn) = internal::manifest_cache::open(session) {
            let _ = internal::manifest_cache::populate(&conn, session);
        }
    }

    Ok(())
}

/// Remove a bucket from Scoop.
///
/// # Errors
///
/// This method will return an error if the bucket does not exist. I/O errors
/// will be returned if the bucket directory is unable to be removed.
pub fn bucket_remove(session: &Session, name: &str) -> Fallible<()> {
    let mut path = session.config().root_path().to_owned();
    path.push("buckets");
    path.push(name);

    if !path.exists() {
        return Err(Error::BucketNotFound(name.to_owned()));
    }

    Ok(remove_dir_all::remove_dir_all(path.as_path())?)
}

/// Get a list of downloaded cache files.
///
/// # Returns
///
/// A list of downloaded cache files.
///
/// # Errors
///
/// I/O errors will be returned if the cache directory is not readable.
pub fn cache_list(session: &Session, query: &str) -> Fallible<Vec<CacheFile>> {
    let is_wildcard_query = query.eq("*") || query.is_empty();
    let config = session.config();
    let cache_dir = config.cache_path();
    let mut files = vec![];

    match cache_dir.read_dir() {
        Err(err) => {
            debug!("failed to read cache dir (err: {})", err);
        }
        Ok(entires) => {
            files = entires
                .filter_map(|de| {
                    if let Ok(entry) = de {
                        let is_file = entry.file_type().unwrap().is_file();
                        if is_file {
                            if let Ok(item) = CacheFile::from(entry.path()) {
                                if !is_wildcard_query {
                                    let matched = item
                                        .package_name()
                                        .to_lowercase()
                                        .contains(&query.to_lowercase());
                                    if matched {
                                        return Some(item);
                                    } else {
                                        return None;
                                    }
                                }

                                return Some(item);
                            }
                        }
                    }
                    None
                })
                .collect::<Vec<_>>();
        }
    }

    Ok(files)
}

/// Remove cache files by query.
///
/// # Errors
///
/// I/O errors will be returned if the cache directory is not readable or failed
/// to remove the cache files.
pub fn cache_remove(session: &Session, query: &str) -> Fallible<()> {
    match query {
        "*" => {
            let config = session.config();
            Ok(internal::fs::empty_dir(config.cache_path())?)
        }
        query => {
            let files = cache_list(session, query)?;
            for f in files.into_iter() {
                std::fs::remove_file(f.path())?;
            }
            Ok(())
        }
    }
}

/// Check if a URL is accessible via HTTP HEAD, using the session's proxy config.
pub fn head_url(session: &Session, url: &str, timeout_secs: u64) -> Fallible<bool> {
    let config = session.config();
    internal::network::head_url(url, config.proxy(), timeout_secs)
        .map_err(|e| Error::Custom(e.to_string()))
}

/// Download a file via HTTP GET and save to a local path, using the session's proxy.
pub fn download_file(session: &Session, url: &str, dest: &Path) -> Fallible<()> {
    let config = session.config();
    let data = internal::network::download_file(url, config.proxy())
        .map_err(|e| crate::error::Error::Custom(e.to_string()))?;
    if let Some(parent) = dest.parent() {
        internal::fs::ensure_dir(parent)?;
    }
    std::fs::write(dest, &data)?;
    Ok(())
}

/// Download a URL's content as a UTF-8 string using the session's proxy.
pub fn download_page(session: &Session, url: &str) -> Fallible<String> {
    let config = session.config();
    let data = internal::network::download_file(url, config.proxy())
        .map_err(|e| Error::Custom(e.to_string()))?;
    String::from_utf8(data).map_err(|e| Error::Custom(format!("UTF-8 decode error: {}", e)))
}

/// Reset a package to reapply its shims, shortcuts, and run post_install.
///
/// If `version` is `None`, the currently installed version is used.
pub fn package_reset(session: &Session, name: &str, version: Option<&str>) -> Fallible<()> {
    package::sync::reset(session, name, version)
}

/// Get the configuation list.
///
/// # Returns
///
/// A string of the configuation list in pretty-printed JSON format.
///
/// # Errors
///
/// Serde errors will be returned if the config cannot be serialized.
pub fn config_list(session: &Session) -> Fallible<String> {
    let config = session.config();
    config.pretty()
}

/// Set a configuation key. *
///
/// # Errors
///
/// A [`ConfigInUse`][1] error will be returned if the config is borrowed
/// elsewhere.
///
/// A [`ConfigKeyInvalid`][2] error will be returned if the key is invalid.
///
/// A [`ConfigValueInvalid`][3] error will be returned if the value is invalid.
///
/// [1]: crate::Error::ConfigInUse
/// [2]: crate::Error::ConfigKeyInvalid
/// [3]: crate::Error::ConfigValueInvalid
pub fn config_set(session: &Session, key: &str, value: &str) -> Fallible<()> {
    session.config_mut()?.set(key, value)
}

/// Hold or unhold a package.
///
/// # Errors
///
/// This method will return an error if the package is not installed.
///
/// A [`PackageHoldBrokenInstall`][1] error will be returned if the install is
/// broken (`install.json` is missing or broken).
///
/// I/O errors will be returned if failed to write the `install.json` file.
/// Serde errors will be returned if the install info cannot be serialized.
///
/// [1]: crate::Error::PackageHoldBrokenInstall
pub fn package_hold(session: &Session, name: &str, flag: bool) -> Fallible<()> {
    let mut path = session.config().root_path().to_owned();
    path.push("apps");
    path.push(name);

    if !path.exists() {
        return Err(Error::PackageHoldNotInstalled(name.to_owned()));
    }

    path.push("current");
    path.push("install.json");

    if let Ok(mut install_info) = InstallInfo::parse(&path) {
        install_info.set_held(flag);
        internal::fs::write_json(path, install_info)
    } else {
        Err(Error::PackageHoldBrokenInstall(name.to_owned()))
    }
}

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
pub fn package_query(
    session: &Session,
    queries: Vec<&str>,
    options: Vec<QueryOption>,
    installed: bool,
) -> Fallible<Vec<Package>> {
    // remove possible duplicates
    let mut queries = HashSet::<&str>::from_iter(queries)
        .into_iter()
        .collect::<Vec<_>>();

    if queries.is_empty() {
        queries.push("*");
    }

    let mut packages = if installed {
        package::query::query_installed(session, &queries, &options)?
    } else {
        package::query::query_synced(session, &queries, &options)?
    };

    packages.sort_by_key(|p| p.name().to_owned());

    Ok(packages)
}

/// Cleanup old versions of packages.
///
/// Removes all version directories except the current one for each package.
/// If `names` is empty, cleans up all installed packages.
/// Returns a list of (package_name, old_versions_removed_count).
pub fn package_cleanup(session: &Session, names: &[String], ignore_failure: bool) -> Fallible<Vec<(String, usize)>> {
    let config = session.config();
    let apps_dir = config.root_path().join("apps");
    let mut results = Vec::new();

    // If no names given, scan all installed packages
    let scan_names: Vec<String> = if names.is_empty() {
        let mut all = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&apps_dir) {
            for entry in entries.flatten() {
                if entry.file_type().map_or(false, |t| t.is_dir()) {
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

        // Determine current version by reading the "current" symlink target
        let current_version = (|| -> Option<String> {
            let current_link = pkg_dir.join("current");
            std::fs::read_link(&current_link).ok()?
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
        })();

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
                if entry.file_type().map_or(false, |t| t.is_dir()) {
                    old_dirs.push(name_str.to_owned());
                }
            }
        }

        let count = old_dirs.len();
        if count == 0 {
            continue;
        }

        for ver in &old_dirs {
            let ver_dir = pkg_dir.join(ver);
            if let Err(e) = internal::fs::remove_dir(&ver_dir) {
                let msg = format!("failed to remove {} v{}: {}", name, ver, e);
                if ignore_failure {
                    eprintln!("{}", msg);
                } else {
                    return Err(Error::Custom(msg));
                }
            }
        }

        results.push((name.clone(), count));
    }

    Ok(results)
}

/// Sync packages.
///
/// # Note
/// The meaning of `sync` packages is to download, (un)install and/or upgrade
/// packages.
///
/// # Errors
///
/// I/O errors will be returned if the `apps`/`buckets` directory is not readable.
///
/// A [`PackageNotFound`][1] error will be returned if no package is found for
/// the given query.
///
/// A [`PackageMultipleCandidates`][2] error will be returned if multiple
/// candidates are found for the given query and not able to ask for a selection.
///
/// [1]: crate::Error::PackageNotFound
/// [2]: crate::Error::PackageMultipleCandidates
pub fn package_sync(
    session: &Session,
    queries: Vec<&str>,
    options: Vec<SyncOption>,
) -> Fallible<()> {
    // remove possible duplicates
    let queries = HashSet::<&str>::from_iter(queries)
        .into_iter()
        .collect::<Vec<_>>();

    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackageResolveStart);
    }

    let is_op_remove = options.contains(&SyncOption::Remove);
    if is_op_remove {
        package::sync::remove(session, &queries, &options)?;
    } else {
        package::sync::install(session, &queries, &options)?;
    }

    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackageSyncDone);
    }

    Ok(())
}
