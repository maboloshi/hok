//! Concurrent package download via the async `Network` trait (reqwest).
//!
//! Replaced the blocking `ureq` client (phase 4) so downloads are真 async I/O.
//! Fragmented downloads run concurrent chunk tasks with `buffer_unordered`.
//!
//! The module is split into focused files: the download core lives here,
//! fragmented (range-parallel) machinery in [`download_fragmented`],
//! verification in [`download_verify`], and the standalone `download`
//! command in [`download_apps`].

use std::cell::OnceCell;
use std::{
    collections::HashMap,
    fs::File,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tracing::debug;

use futures::StreamExt;

use crate::constant::DEFAULT_USER_AGENT;
use crate::{Event, Session, error::Fallible, internal};

use super::Package;

pub use super::download_apps::{DownloadOptions, download_apps};
use super::download_fragmented::download_fragmented;
pub(crate) use super::download_verify::download_and_verify;

/// Download size information.
#[derive(Clone, Copy)]
pub struct DownloadSize {
    /// Total size to download.
    pub total: u64,
    /// Whether the total size is estimated.
    pub estimated: bool,
}

/// A set of packages to download.
pub struct PackageSet<'a> {
    /// Backing session (exposed to `download_verify`/`download_apps`).
    pub(crate) session: &'a Session,
    pub packages: &'a [&'a Package],
    caches: OnceCell<HashMap<String, PackageCache<'a>>>,
    reuse_cache: bool,
    /// Skip packages whose download fails instead of aborting the operation.
    ignore_failure: bool,
}

struct FileDownloadInfo<'a> {
    url: &'a str,
    local_size: u64,
    remote_size: u64,
    estimated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CacheMaybeValid {
    Full,
    Partial,
    None,
}

struct PackageCache<'a> {
    package: &'a Package,
    valid: CacheMaybeValid,
    inner: HashMap<String, FileDownloadInfo<'a>>,
}

impl PackageCache<'_> {
    fn update_valid_state(&mut self) {
        let mut cnt = 0;
        for cache in self.inner.values() {
            // Only a file that actually exists locally (local_size > 0) and
            // matches the server size counts as a valid cache entry. A missing
            // local file must not compare equal to an unknown (0) remote size:
            // the equality 0 == 0 previously marked the package as fully
            // cached and skipped its download, then hash verification failed
            // with "file not found".
            if cache.local_size > 0 && cache.local_size == cache.remote_size {
                cnt += 1;
            }
        }
        self.valid = if cnt == self.inner.len() {
            CacheMaybeValid::Full
        } else if cnt > 0 {
            CacheMaybeValid::Partial
        } else {
            CacheMaybeValid::None
        };
    }
}

/// Result of one package's concurrent download.
struct PkgDownloadOutcome {
    /// Pending simple-download renames `(tmp, path, ident)`.
    filepaths: Vec<(PathBuf, PathBuf, String)>,
    /// Fragmented files to reassemble: `final_path -> (part_paths, ident)`.
    chunk_file_map: HashMap<PathBuf, (Vec<PathBuf>, String)>,
    /// The package ident when it failed (only under IgnoreFailure).
    failed_ident: Option<String>,
}

/// Download the files of a single package. Runs on a concurrent task so
/// multiple packages download in parallel (cross-package concurrency).
///
/// The body mirrors the per-package branch of the former sequential loop in
/// [`PackageSet::download`]: fragmented or single-stream download per file,
/// reporting progress, honouring reuse-cache and the cancellation token.
/// Each package writes only its own cache files, so concurrent tasks never
/// collide.
#[allow(clippy::too_many_arguments)]
async fn download_one_package<'a>(
    session: &'a Session,
    cache: &PackageCache<'a>,
    cache_root: &Path,
    base_opts: &internal::network::RequestOptions<'_>,
    net: &Arc<dyn internal::network::Network + Send + Sync>,
    reuse_cache: bool,
    ignore_failure: bool,
    fragmentation_enabled: bool,
    chunk_count: u64,
    min_split_size: u64,
) -> Fallible<PkgDownloadOutcome> {
    let mut filepaths: Vec<(PathBuf, PathBuf, String)> = vec![];
    let mut chunk_file_map: HashMap<PathBuf, (Vec<PathBuf>, String)> = HashMap::new();

    let cookie = cache.package.cookie().unwrap_or_default();
    let ident = cache.package.ident();
    let cookie_map: Option<HashMap<String, String>> = if cookie.is_empty() {
        None
    } else {
        Some(
            cookie
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    };
    let file_opts = internal::network::RequestOptions {
        cookies: cookie_map.as_ref(),
        ..base_opts.clone()
    };

    // A failure on any file marks the whole package as failed so the rest of
    // the transaction can still be committed (IgnoreFailure).
    let mut pkg_error: Option<crate::Error> = None;
    let emitter = session.emitter();

    // Total bytes the package must fetch (all files that are not already
    // fully cached). Mirrors the skip condition in the loop below, so the
    // reported pkg_total equals the bytes actually downloaded.
    let pkg_total: u64 = cache
        .inner
        .iter()
        .filter(|(_, d)| !(reuse_cache && d.local_size > 0 && d.local_size == d.remote_size))
        .map(|(_, d)| d.remote_size)
        .sum();

    // Running total of bytes this package has fully downloaded (across its
    // files). Shared across the per-file progress closures so each progress
    // event reports the package-wide cumulative done.
    let pkg_done = Arc::new(AtomicU64::new(0));

    for (filename, dlinfo) in cache.inner.iter() {
        if reuse_cache && dlinfo.local_size > 0 && dlinfo.local_size == dlinfo.remote_size {
            continue;
        }

        let use_fragments = fragmentation_enabled
            && !dlinfo.estimated
            && dlinfo.remote_size >= min_split_size
            && dlinfo.remote_size > 0
            && chunk_count > 1
            // Avoid `(chunk_idx + 1) * chunk_size - 1` underflow when the
            // file is smaller than the configured chunk count.
            && chunk_count <= dlinfo.remote_size;

        let result: Fallible<()> = async {
            if use_fragments {
                let path = cache_root.join(filename);
                let part_dir = cache_root.join(format!("{}.parts", filename));
                internal::fs::ensure_dir(&part_dir)?;

                // Chunks whose retries are exhausted are re-downloaded serially
                // (see `download_fragmented`), so a weak link degrades to fewer
                // connections instead of failing the whole file.
                let url_str = dlinfo.url.to_owned();
                // Aggregate per-chunk deltas into a file-level count so the
                // progress event matches the single-stream path.
                let progress_total = Arc::new(AtomicU64::new(0));
                let progress_total_h = progress_total.clone();
                let throttle = Arc::new(ProgressThrottle::new());
                let throttle_h = throttle.clone();
                let emitter_h = emitter.clone();
                let ident_h = ident.clone();
                let filename_h = filename.to_owned();
                let url_h = url_str.clone();
                let pkg_done_h = pkg_done.clone();
                let pkg_total_h = pkg_total;
                let on_progress = move |delta: u64| {
                    let dlnow = progress_total_h.fetch_add(delta, Ordering::Relaxed) + delta;
                    // Throttle to one event per 100 ms so a slow event loop
                    // cannot back up the bounded channel (aligned with rake's
                    // THROTTLE_DURATION); force the final event so the bar
                    // reaches 100%.
                    let force = dlnow >= dlinfo.remote_size;
                    if throttle_h.should_report(force)
                        && let Some(tx) = &emitter_h
                    {
                        let _ = tx.try_send(Event::PackageDownloadProgress(
                            PackageDownloadProgressContext {
                                ident: ident_h.clone(),
                                url: url_h.clone(),
                                filename: filename_h.clone(),
                                dltotal: dlinfo.remote_size,
                                dlnow,
                                pkg_total: pkg_total_h,
                                pkg_done: pkg_done_h.load(Ordering::Relaxed) + dlnow,
                            },
                        ));
                    }
                };
                let part_paths = download_fragmented(
                    net.as_ref(),
                    &url_str,
                    &file_opts,
                    dlinfo.remote_size,
                    chunk_count,
                    &part_dir,
                    &on_progress,
                    &session.cancellation_token(),
                )
                .await
                .map_err(crate::error::Error::Custom)?;

                pkg_done.fetch_add(dlinfo.remote_size, Ordering::Relaxed);
                chunk_file_map.insert(path, (part_paths, ident.clone()));
            } else {
                // Single download (with resume for known-size files).
                let path = cache_root.join(filename);
                let tmp = cache_root.join(format!("{}.download", filename));
                let _ = std::fs::remove_file(&path);

                // Resume support: when the total size is known (not estimated)
                // and a partial `.download` already exists, resume from its
                // current size via a Range request instead of re-downloading.
                let known_total = dlinfo.remote_size > 0 && !dlinfo.estimated;
                let existing_len = tmp.metadata().map(|m| m.len()).unwrap_or(0);
                let resume_from = if known_total {
                    existing_len.min(dlinfo.remote_size)
                } else {
                    0
                };

                // A `.download` already at the full size needs no network
                // transfer — promote it straight to the cache.
                if known_total && existing_len == dlinfo.remote_size {
                    filepaths.push((tmp, path, ident.clone()));
                } else {
                    // Fresh start: drop any stale partial.
                    if resume_from == 0 {
                        let _ = std::fs::remove_file(&tmp);
                    }

                    let fname = filename.to_owned();
                    let url_str = dlinfo.url.to_owned();
                    let dlinfo_total = dlinfo.remote_size;
                    let pkg_total_here = pkg_total;
                    let pkg_done_h = pkg_done.clone();

                    // Stream the body (from `resume_from` when resuming)
                    // through the network capability (reports cumulative bytes
                    // via on_progress).
                    let throttle = Arc::new(ProgressThrottle::new());
                    let throttle_h = throttle.clone();
                    let emitter_h = emitter.clone();
                    let ident_h = ident.clone();
                    let url_h = url_str.clone();
                    let fname_h = fname.clone();
                    let mut on_progress =
                        move |dlnow: u64| -> Pin<Box<dyn Future<Output = ()> + Send>> {
                            // Clone per call: the future is `move` so it must own
                            // its captures.
                            let throttle = throttle_h.clone();
                            let tx = emitter_h.clone();
                            let ident = ident_h.clone();
                            let url = url_h.clone();
                            let filename = fname_h.clone();
                            let total = dlinfo_total;
                            let pkg_done_here = pkg_done_h.clone();
                            Box::pin(async move {
                                // Throttle to one event per 100 ms (aligned with
                                // rake's THROTTLE_DURATION); force the final event
                                // so the bar reaches 100%.
                                let force = dlnow >= total;
                                if throttle.should_report(force)
                                    && let Some(tx) = &tx
                                {
                                    let ctx = PackageDownloadProgressContext {
                                        ident,
                                        url,
                                        filename,
                                        dltotal: total,
                                        dlnow,
                                        pkg_total: pkg_total_here,
                                        pkg_done: pkg_done_here.load(Ordering::Relaxed) + dlnow,
                                    };
                                    // `send().await` gives backpressure: a slow
                                    // event loop cannot silently drop progress.
                                    let _ = tx.send(Event::PackageDownloadProgress(ctx)).await;
                                }
                            })
                        };

                    let resumable_opts = internal::network::RequestOptions {
                        range_start: resume_from,
                        ..file_opts.clone()
                    };
                    // Race the stream against cooperative Ctrl-C cancellation;
                    // on cancel the partial `.download` file is kept so the
                    // next run resumes from here.
                    let cancel = session.cancellation_token();
                    tokio::select! {
                        _ = cancel.cancelled() => {
                            return Err(crate::error::Error::Custom(
                                "download cancelled".to_owned(),
                            ));
                        }
                        res = net.download_stream(
                            &url_str,
                            &resumable_opts,
                            &tmp,
                            &mut on_progress,
                        ) => {
                            res.map_err(|e| crate::error::Error::Custom(e.to_string()))?;
                            // A resumed download must reach the full size — a
                            // short 206 would leave a corrupt part that must
                            // not be promoted to cache.
                            if resume_from > 0
                                && tmp.metadata().map(|m| m.len()).unwrap_or(0)
                                    != dlinfo.remote_size
                            {
                                return Err(crate::error::Error::Custom(format!(
                                    "short resumed download: {} != {} bytes",
                                    tmp.display(),
                                    dlinfo.remote_size,
                                )));
                            }
                        }
                    }

                    pkg_done.fetch_add(dlinfo.remote_size, Ordering::Relaxed);
                    filepaths.push((tmp, path, ident.clone()));
                }
            }
            Ok(())
        }
        .await;

        if let Err(e) = result {
            pkg_error = Some(e);
            break;
        }
    }

    if let Some(err) = pkg_error {
        if ignore_failure {
            session.output().error(hok_i18n::t!(
                "cmd.failed_download",
                name = ident,
                error = err
            ));
            return Ok(PkgDownloadOutcome {
                filepaths,
                chunk_file_map,
                failed_ident: Some(ident),
            });
        }
        return Err(err);
    }

    Ok(PkgDownloadOutcome {
        filepaths,
        chunk_file_map,
        failed_ident: None,
    })
}

impl<'a> PackageSet<'a> {
    pub fn new(
        session: &'a Session,
        packages: &'a [&Package],
        reuse_cache: bool,
    ) -> Fallible<PackageSet<'a>> {
        Ok(PackageSet {
            session,
            packages,
            caches: OnceCell::new(),
            reuse_cache,
            ignore_failure: false,
        })
    }

    /// Ignore per-package download failures and keep going with the rest of
    /// the transaction.
    ///
    /// When enabled, [`download`][1] skips packages whose download fails (and
    /// reports them in the returned list of failed idents) instead of aborting
    /// the whole operation, so the remaining packages can still be committed.
    ///
    /// [1]: PackageSet::download
    pub fn set_ignore_failure(&mut self, ignore: bool) {
        self.ignore_failure = ignore;
    }

    fn load_cache(&self) {
        if self.caches.get().is_some() {
            return;
        }

        let config = self.session.config();
        let cache_root = config.cache_path();
        let mut caches = HashMap::new();

        for &pkg in self.packages.iter() {
            let pkg = pkg.upgradable().unwrap_or(pkg);
            let urls = pkg.download_urls();
            let filenames = pkg.download_filenames();

            let mut package_cache = PackageCache {
                package: pkg,
                valid: CacheMaybeValid::None,
                inner: HashMap::new(),
            };

            let mut file_cached_count = 0;
            for (url, filename) in urls.iter().zip(filenames.iter()) {
                let remote_size = 0u64;
                let mut local_size = 0u64;

                if self.reuse_cache
                    && let Ok(file) = File::open(cache_root.join(filename))
                    && let Ok(metadata) = file.metadata()
                {
                    local_size = metadata.len();
                    file_cached_count += 1;
                }

                package_cache.inner.insert(
                    filename.to_owned(),
                    FileDownloadInfo {
                        url,
                        local_size,
                        remote_size,
                        estimated: false,
                    },
                );
            }

            if self.reuse_cache {
                if file_cached_count == urls.len() {
                    package_cache.valid = CacheMaybeValid::Full;
                } else if file_cached_count > 0 {
                    package_cache.valid = CacheMaybeValid::Partial;
                }
            }

            caches.insert(pkg.ident(), package_cache);
        }

        let _ = self.caches.set(caches);
    }

    // ─── Download ─────────────────────────────────────────────────────────────

    /// Download packages.
    ///
    /// # Returns
    ///
    /// The identifiers (`bucket/name`) of the packages whose download failed
    /// and were skipped. Unless [`set_ignore_failure`][1] was enabled, the
    /// first failure aborts with an error, so the list is empty on success.
    ///
    /// [1]: PackageSet::set_ignore_failure
    pub async fn download(&mut self) -> Fallible<Vec<String>> {
        if self.caches.get().is_none() {
            self.load_cache();
        }

        // Extract config data into owned values inside a scope so the config
        // read guard is dropped before the awaits below (a guard held across an
        // await would block a concurrent `config_mut()`).
        let (
            cache_root,
            proxy,
            user_agent,
            fragmentation_enabled,
            chunk_count,
            min_split_size,
            jobs,
        ) = {
            let config = self.session.config();
            (
                config.cache_path().to_path_buf(),
                config.proxy().map(|s| s.to_owned()),
                self.session
                    .user_agent
                    .get()
                    .map(|s| s.as_str())
                    .unwrap_or(DEFAULT_USER_AGENT)
                    .to_owned(),
                config.aria2_enabled(),
                config
                    .aria2_split()
                    .min(config.aria2_max_connection_per_server()) as u64,
                config.aria2_min_split_size(),
                config.jobs() as usize,
            )
        };

        // Extract session-owned copies up front so the concurrent futures
        // below never capture `self` (which would conflict with the caches
        // borrow). Session is Sync + Send, so all of these share cleanly.
        let session = self.session;
        let reuse_cache = self.reuse_cache;
        let ignore_failure = self.ignore_failure;
        let net = self.session.network();
        let emitter = self.session.emitter();

        // Read caches immutably: the download phase never mutates them.
        let package_caches = self.caches.get().unwrap();

        let mut chunk_file_map: HashMap<PathBuf, (Vec<PathBuf>, String)> = HashMap::new();
        let mut filepaths: Vec<(PathBuf, PathBuf, String)> = vec![];
        let mut failed: Vec<String> = vec![];

        internal::fs::ensure_dir(&cache_root)?;

        // Shared request options (proxy + UA); the cookie is injected per
        // file inside the per-package task. The network capability is
        // injected per session so tests can substitute an in-memory fake.
        let base_opts = internal::network::RequestOptions {
            proxy: proxy.as_deref(),
            timeout_secs: 120,
            user_agent: Some(&user_agent),
            ..internal::network::RequestOptions::default()
        };

        // Concurrent cross-package download: each package's files are fetched
        // on its own task, bounded by `jobs`. Every package writes only its
        // own cache files (names embed `name#version#`), so concurrent tasks
        // never collide. Fully-cached packages are skipped (their cache-hit
        // events are still emitted) and packages whose download fails are
        // reported per-package, matching the sequential semantics.
        let jobs = jobs.max(1);

        // Tell the frontend which packages are about to be downloaded and their
        // total bytes, so it can render one aggregate progress bar (count and
        // grand total) before the per-package progress events start arriving.
        if let Some(tx) = &emitter {
            let packages: Vec<(String, u64)> = package_caches
                .values()
                .filter(|c| !(reuse_cache && c.valid == CacheMaybeValid::Full))
                .map(|c| {
                    let total: u64 = c
                        .inner
                        .iter()
                        .filter(|(_, d)| {
                            !(reuse_cache && d.local_size > 0 && d.local_size == d.remote_size)
                        })
                        .map(|(_, d)| d.remote_size)
                        .sum();
                    (c.package.ident(), total)
                })
                .collect();
            let _ = tx.try_send(Event::PackageDownloadInfo { packages });
        }

        let outcomes: Vec<Fallible<PkgDownloadOutcome>> =
            futures::stream::iter(package_caches.values().filter(|c| {
                if reuse_cache && c.valid == CacheMaybeValid::Full {
                    if let Some(tx) = &emitter {
                        for filename in c.inner.keys() {
                            let _ = tx.try_send(Event::PackageCacheHit(filename.clone()));
                        }
                    }
                    false
                } else {
                    true
                }
            }))
            .map(|cache| {
                download_one_package(
                    session,
                    cache,
                    &cache_root,
                    &base_opts,
                    &net,
                    reuse_cache,
                    ignore_failure,
                    fragmentation_enabled,
                    chunk_count,
                    min_split_size,
                )
            })
            .buffer_unordered(jobs)
            .collect()
            .await;

        // Merge the per-package outcomes into the shared accumulators.
        for outcome in outcomes {
            let outcome = outcome?; // a hard (non-ignore) failure aborts
            filepaths.extend(outcome.filepaths);
            chunk_file_map.extend(outcome.chunk_file_map);
            if let Some(ident) = outcome.failed_ident {
                failed.push(ident);
            }
        }
        // Reassemble fragmented files
        for (final_path, (part_paths, ident)) in chunk_file_map.iter() {
            let reassembled = (|| -> Fallible<()> {
                let _ = std::fs::remove_file(final_path);
                let mut dest = File::create(final_path)?;
                for part in part_paths {
                    let mut src = File::open(part)?;
                    std::io::copy(&mut src, &mut dest)?;
                    drop(src);
                    let _ = std::fs::remove_file(part);
                }
                if let Some(parent) = final_path.parent() {
                    let part_dir = parent.join(format!(
                        "{}.parts",
                        final_path.file_name().unwrap().to_string_lossy()
                    ));
                    let _ = std::fs::remove_dir(&part_dir);
                }
                Ok(())
            })();
            if let Err(e) = reassembled {
                if self.ignore_failure {
                    self.session.output().error(hok_i18n::t!(
                        "cmd.failed_reassemble",
                        name = ident,
                        error = e
                    ));
                    failed.push(ident.clone());
                } else {
                    return Err(e);
                }
            }
        }

        // Rename simple downloads
        for (tmp, path, ident) in filepaths.iter() {
            if let Err(e) = std::fs::rename(tmp, path) {
                if self.ignore_failure {
                    self.session.output().error(hok_i18n::t!(
                        "cmd.failed_move_download",
                        name = ident,
                        error = e
                    ));
                    failed.push(ident.clone());
                } else {
                    return Err(e.into());
                }
            }
        }

        Ok(failed)
    }

    // ─── Calculate download size ──────────────────────────────────────────────

    pub async fn calculate_download_size(&mut self) -> Fallible<DownloadSize> {
        if self.caches.get().is_none() {
            self.load_cache();
        }

        let (proxy, user_agent, jobs) = {
            let config = self.session.config();
            (
                config.proxy().map(|s| s.to_owned()),
                self.session
                    .user_agent
                    .get()
                    .map(|s| s.as_str())
                    .unwrap_or(DEFAULT_USER_AGENT)
                    .to_owned(),
                config.jobs() as usize,
            )
        };

        // Fire all HEAD requests concurrently (one per URL), then apply the
        // results back to the cache entries serially. The HEAD is pure network
        // I/O and only writes `remote_size`/`estimated` into the caches, so the
        // concurrent phase captures owned data and the apply phase mutates the
        // caches under a single `&mut`.
        let net = self.session.network();

        // Collect every (url, filename, ident, cookie) as an owned HEAD task.
        type HeadTask = (String, String, String, Option<HashMap<String, String>>);
        let tasks: Vec<HeadTask> = self
            .packages
            .iter()
            .map(|&pkg| pkg.upgradable().unwrap_or(pkg))
            .flat_map(|pkg| {
                let ident = pkg.ident().to_owned();
                let cookie = pkg.cookie().unwrap_or_default();
                let cookie_map: Option<HashMap<String, String>> = if cookie.is_empty() {
                    None
                } else {
                    Some(
                        cookie
                            .iter()
                            .map(|(k, v)| (k.to_string(), v.to_string()))
                            .collect(),
                    )
                };
                pkg.download_urls()
                    .iter()
                    .zip(pkg.download_filenames().iter())
                    .map(move |(url, filename)| {
                        (
                            url.to_string(),
                            filename.clone(),
                            ident.clone(),
                            cookie_map.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        let results: Vec<(String, String, u16, Option<u64>)> = futures::stream::iter(tasks)
            .map(|(url, filename, ident, cookie_map)| {
                let net = net.clone();
                let proxy = proxy.clone();
                let user_agent = user_agent.clone();
                async move {
                    let opts = internal::network::RequestOptions {
                        proxy: proxy.as_deref(),
                        timeout_secs: 30,
                        user_agent: Some(&user_agent),
                        cookies: cookie_map.as_ref(),
                        ..internal::network::RequestOptions::default()
                    };
                    let result = net.head(&url, &opts).await;
                    (ident, filename, result.status_code, result.content_length)
                }
            })
            .buffer_unordered(jobs.max(1))
            .collect()
            .await;

        // Apply HEAD results back into the caches (serial, per file).
        let package_caches = self.caches.get_mut().unwrap();
        let mut total = 0u64;
        let mut estimated = false;

        for (ident, filename, code, content_length) in results {
            let package_cache = package_caches.get_mut(&ident).unwrap();
            let info = package_cache
                .inner
                .get_mut(&filename)
                .expect("failed to get cache info");

            if code == 200 {
                info.remote_size = content_length.unwrap_or(0);
                if info.remote_size != info.local_size {
                    total += info.remote_size;
                }
            } else {
                debug!("code: {}, ident: {}, url: {}", code, ident, filename)
            }

            if info.remote_size == 0 {
                info.estimated = true;
                estimated = true;
            }

            package_cache.update_valid_state();
        }

        Ok(DownloadSize { total, estimated })
    }
}

// ─── Progress throttle ─────────────────────────────────────────────────────

/// Minimum interval between two download progress events.
///
/// Aligned with rake's `THROTTLE_DURATION` (100 ms): download progress can
/// otherwise fire far faster than the event loop consumes, and the bounded
/// `tokio::sync::mpsc` bus drops `try_send` events when it fills, making the
/// progress bar jump. Throttling to one event per interval bounds the event
/// rate so the bar updates smoothly and `send().await` backpressure is cheap.
const PROGRESS_THROTTLE: Duration = Duration::from_millis(100);

/// Throttle download-progress events to at most one per [`PROGRESS_THROTTLE`].
///
/// `Sync` (via `Mutex`) so a single shared throttle can be handed to the
/// concurrent fragmented-download chunk tasks.
#[derive(Debug, Default)]
pub(crate) struct ProgressThrottle {
    last: Mutex<Option<Instant>>,
}

impl ProgressThrottle {
    /// Create a fresh throttle (the first report always passes).
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Whether a progress event should be emitted now: true when at least
    /// [`PROGRESS_THROTTLE`] has elapsed since the previous report (or on the
    /// first call). `force` bypasses the throttle so the final event (bytes
    /// reaching the total) is always delivered — otherwise the bar could sit
    /// below 100% until the download-done event. Updates the timestamp when
    /// it reports (forced or not).
    pub(crate) fn should_report(&self, force: bool) -> bool {
        let now = Instant::now();
        let mut last = self.last.lock().unwrap();
        match *last {
            Some(prev) if !force && now.duration_since(prev) < PROGRESS_THROTTLE => false,
            _ => {
                *last = Some(now);
                true
            }
        }
    }
}

// ─── Progress context ───────────────────────────────────────────────────────

/// Progress context for package download.
#[derive(Clone, Debug)]
pub struct PackageDownloadProgressContext {
    pub ident: String,
    pub url: String,
    pub filename: String,
    /// Total bytes of the *current file*.
    pub dltotal: u64,
    /// Bytes downloaded so far of the *current file*.
    pub dlnow: u64,
    /// Total bytes of the whole package (all files to download). Lets the UI
    /// aggregate per-file progress into a per-package bar / single total bar.
    pub pkg_total: u64,
    /// Bytes of the package downloaded so far (sum of completed files plus the
    /// current file's progress). Monotonic within a package.
    pub pkg_done: u64,
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_chunk_boundaries() {
        let size = 100u64;
        let chunks = 4u64;
        let chunk_size = size / chunks;
        assert_eq!(chunk_size, 25);
        assert_eq!(chunk_size - 1, 24);
        assert_eq!(chunk_size, 25);
        assert_eq!((1 + 1) * chunk_size - 1, 49);
        assert_eq!(3 * chunk_size, 75);
        assert_eq!(size - 1, 99);
    }

    #[test]
    fn test_chunk_cover_all_bytes() {
        check_chunks_cover_all(100, 5);
    }

    #[test]
    fn test_chunk_remainder() {
        check_chunks_cover_all(10, 3);
    }

    /// Verify that `chunks` evenly cover all `size` bytes without gaps.
    fn check_chunks_cover_all(size: u64, chunks: u64) {
        let chunk_size = size / chunks;
        let mut covered = vec![false; size as usize];
        for i in 0..chunks {
            let start = i * chunk_size;
            let end = if i == chunks - 1 {
                size - 1
            } else {
                (i + 1) * chunk_size - 1
            };
            for b in start..=end {
                covered[b as usize] = true;
            }
        }
        assert!(covered.iter().all(|&c| c));
    }

    #[test]
    fn test_chunk_single_byte() {
        let size = 1u64;
        let chunks = 1u64;
        let chunk_size = size / chunks;
        assert_eq!(chunk_size, 1);
        assert_eq!(size - 1, 0);
    }

    #[test]
    fn test_progress_throttle_first_always_reports() {
        let t = super::ProgressThrottle::new();
        assert!(t.should_report(false), "first report must pass");
    }

    #[test]
    fn test_progress_throttle_blocks_rapid_reports() {
        let t = super::ProgressThrottle::new();
        assert!(t.should_report(false));
        // Immediate second report within the 100 ms window is throttled.
        assert!(!t.should_report(false));
    }

    #[test]
    fn test_progress_throttle_force_bypasses_window() {
        let t = super::ProgressThrottle::new();
        assert!(t.should_report(false));
        // `force` bypasses the throttle so the final event always lands.
        assert!(t.should_report(true));
    }

    #[test]
    fn test_progress_throttle_reports_after_interval() {
        let t = super::ProgressThrottle::new();
        assert!(t.should_report(false));
        assert!(!t.should_report(false));
        // Past the throttle interval the throttle re-opens.
        std::thread::sleep(std::time::Duration::from_millis(110));
        assert!(t.should_report(false));
    }

    #[test]
    fn test_update_valid_state_missing_file_with_unknown_size_is_not_cached() {
        // Regression: a package whose cache file is missing (local_size 0) and
        // whose remote size is unknown (0, e.g. a failed HEAD request) must NOT
        // be treated as fully cached. Previously local_size == remote_size
        // evaluated 0 == 0, marking the package Full so download skipped it
        // (emitting a cache-hit event), and hash verification then failed with
        // "file not found".
        use super::{CacheMaybeValid, FileDownloadInfo, PackageCache};
        use std::collections::HashMap;

        let pkg = crate::package::Package::from(
            "app",
            "test",
            crate::package::manifest::Manifest::from_json(
                "app",
                r#"{"version": "2.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
            )
            .unwrap(),
        );
        let mut pc = PackageCache {
            package: &pkg,
            valid: CacheMaybeValid::None,
            inner: HashMap::new(),
        };
        pc.inner.insert(
            "app#2.0.0#abc123.7z".to_owned(),
            FileDownloadInfo {
                url: "https://example.com/app.7z",
                local_size: 0,
                remote_size: 0,
                estimated: true,
            },
        );
        pc.update_valid_state();
        assert_eq!(pc.valid, CacheMaybeValid::None);
    }

    #[test]
    fn test_update_valid_state_full_only_for_present_matching_file() {
        use super::{CacheMaybeValid, FileDownloadInfo, PackageCache};
        use std::collections::HashMap;

        let pkg = crate::package::Package::from(
            "app",
            "test",
            crate::package::manifest::Manifest::from_json(
                "app",
                r#"{"version": "2.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
            )
            .unwrap(),
        );
        let mut pc = PackageCache {
            package: &pkg,
            valid: CacheMaybeValid::None,
            inner: HashMap::new(),
        };
        // A present local file matching the remote size is a valid cache entry.
        pc.inner.insert(
            "app#2.0.0#abc123.7z".to_owned(),
            FileDownloadInfo {
                url: "https://example.com/app.7z",
                local_size: 100,
                remote_size: 100,
                estimated: false,
            },
        );
        pc.update_valid_state();
        assert_eq!(pc.valid, CacheMaybeValid::Full);

        // A present local file whose size differs from the remote is invalid.
        pc.inner.insert(
            "app#2.0.0#abc123.7z".to_owned(),
            FileDownloadInfo {
                url: "https://example.com/app.7z",
                local_size: 50,
                remote_size: 100,
                estimated: false,
            },
        );
        pc.update_valid_state();
        assert_eq!(pc.valid, CacheMaybeValid::None);
    }
}
