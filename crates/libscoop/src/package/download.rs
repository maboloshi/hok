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
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};
use tracing::debug;

use crate::constant::DEFAULT_USER_AGENT;
use crate::{error::Fallible, internal, Event, Session};

use super::Package;

pub use super::download_apps::{download_apps, DownloadOptions};
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
                        && let Ok(metadata) = file.metadata() {
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
        let (cache_root, proxy, user_agent, fragmentation_enabled, chunk_count, min_split_size) = {
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
            )
        };

        let package_caches = self.caches.get_mut().unwrap();

        let mut chunk_file_map: HashMap<PathBuf, (Vec<PathBuf>, String)> = HashMap::new();
        let mut filepaths: Vec<(PathBuf, PathBuf, String)> = vec![];
        let mut failed: Vec<String> = vec![];

        internal::fs::ensure_dir(&cache_root)?;

        // Shared request options (proxy + UA); the cookie is injected per
        // file below. The network capability is injected per session so tests
        // can substitute an in-memory fake.
        let base_opts = internal::network::RequestOptions {
            proxy: proxy.as_deref(),
            timeout_secs: 120,
            user_agent: Some(&user_agent),
            ..internal::network::RequestOptions::default()
        };
        let net = self.session.network();

        for cache in package_caches.values() {
            if self.reuse_cache && cache.valid == CacheMaybeValid::Full {
                if let Some(tx) = self.session.emitter() {
                    for filename in cache.inner.keys() {
                        let _ = tx.try_send(Event::PackageCacheHit(filename.clone()));
                    }
                }
                continue;
            }

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

            // A failure on any file marks the whole package as failed so the
            // rest of the transaction can still be committed (IgnoreFailure).
            let mut pkg_error: Option<crate::Error> = None;
            let emitter = self.session.emitter();

            for (filename, dlinfo) in cache.inner.iter() {
                if self.reuse_cache
                    && dlinfo.local_size > 0
                    && dlinfo.local_size == dlinfo.remote_size
                {
                    continue;
                }

                let use_fragments = fragmentation_enabled
                    && !dlinfo.estimated
                    && dlinfo.remote_size >= min_split_size
                    && dlinfo.remote_size > 0
                    && chunk_count > 1
                    // Avoid `(chunk_idx + 1) * chunk_size - 1` underflow when
                    // the file is smaller than the configured chunk count.
                    && chunk_count <= dlinfo.remote_size;

                let result: Fallible<()> = async {
                    if use_fragments {
                        let path = cache_root.join(filename);
                        let part_dir = cache_root.join(format!("{}.parts", filename));
                        internal::fs::ensure_dir(&part_dir)?;

                        // Chunks whose retries are exhausted are re-downloaded
                        // serially (see `download_fragmented`), so a weak link
                        // degrades to fewer connections instead of failing the
                        // whole file.
                        let url_str = dlinfo.url.to_owned();
                        // Aggregate per-chunk deltas into a file-level count so
                        // the progress event matches the single-stream path.
                        let progress_total = Arc::new(AtomicU64::new(0));
                        let progress_total_h = progress_total.clone();
                        let throttle = Arc::new(ProgressThrottle::new());
                        let throttle_h = throttle.clone();
                        let emitter_h = emitter.clone();
                        let ident_h = ident.clone();
                        let filename_h = filename.to_owned();
                        let url_h = url_str.clone();
                        let on_progress = move |delta: u64| {
                            let dlnow =
                                progress_total_h.fetch_add(delta, Ordering::Relaxed) + delta;
                            // Throttle to one event per 100 ms so a slow event
                            // loop cannot back up the bounded channel (aligned
                            // with rake's THROTTLE_DURATION); force the final
                            // event so the bar reaches 100%.
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
                            &self.session.cancellation_token(),
                        )
                        .await
                        .map_err(crate::error::Error::Custom)?;

                        chunk_file_map.insert(path, (part_paths, ident.clone()));
                    } else {
                        // Single download (with resume for known-size files).
                        let path = cache_root.join(filename);
                        let tmp = cache_root.join(format!("{}.download", filename));
                        let _ = std::fs::remove_file(&path);

                        // Resume support: when the total size is known (not
                        // estimated) and a partial `.download` already exists,
                        // resume from its current size via a Range request
                        // instead of re-downloading from scratch.
                        let known_total = dlinfo.remote_size > 0 && !dlinfo.estimated;
                        let existing_len = tmp.metadata().map(|m| m.len()).unwrap_or(0);
                        let resume_from = if known_total {
                            existing_len.min(dlinfo.remote_size)
                        } else {
                            0
                        };

                        // A `.download` already at the full size needs no
                        // network transfer — promote it straight to the cache.
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

                            // Stream the body (from `resume_from` when
                            // resuming) through the network capability
                            // (reports cumulative bytes via on_progress).
                            let throttle = Arc::new(ProgressThrottle::new());
                            let throttle_h = throttle.clone();
                            let emitter_h = emitter.clone();
                            let ident_h = ident.clone();
                            let url_h = url_str.clone();
                            let fname_h = fname.clone();
                            let mut on_progress = move |dlnow: u64| -> Pin<
                                Box<dyn Future<Output = ()> + Send>,
                            > {
                                // Clone per call: the future is `move` so it
                                // must own its captures.
                                let throttle = throttle_h.clone();
                                let tx = emitter_h.clone();
                                let ident = ident_h.clone();
                                let url = url_h.clone();
                                let filename = fname_h.clone();
                                let total = dlinfo_total;
                                Box::pin(async move {
                                    // Throttle to one event per 100 ms
                                    // (aligned with rake's THROTTLE_DURATION);
                                    // force the final event so the bar
                                    // reaches 100%.
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
                                        };
                                        // `send().await` gives backpressure:
                                        // a slow event loop cannot silently
                                        // drop progress (throttled events are
                                        // always delivered).
                                        let _ = tx.send(Event::PackageDownloadProgress(ctx)).await;
                                    }
                                })
                            };

                            let resumable_opts = internal::network::RequestOptions {
                                range_start: resume_from,
                                ..file_opts.clone()
                            };
                            // Race the stream against cooperative Ctrl-C
                            // cancellation; on cancel the partial `.download`
                            // file is kept so the next run resumes from here.
                            let cancel = self.session.cancellation_token();
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
                                    res.map_err(|e| {
                                        crate::error::Error::Custom(e.to_string())
                                    })?;
                                    // A resumed download must reach the full
                                    // size — a short 206 would leave a corrupt
                                    // part that must not be promoted to cache.
                                    if resume_from > 0
                                        && tmp.metadata().map(|m| m.len()).unwrap_or(0)
                                            != dlinfo.remote_size
                                    {
                                        return Err(crate::error::Error::Custom(format!(
                                            "short resumed download: {} != {} bytes",
                                            tmp.display(),
                                            dlinfo.remote_size
                                        )));
                                    }
                                }
                            }

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
                if self.ignore_failure {
                    self.session
                        .output()
                        .error(format!("failed to download '{}': {}", ident, err));
                    failed.push(ident);
                } else {
                    return Err(err);
                }
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
                    self.session
                        .output()
                        .error(format!("failed to reassemble '{}': {}", ident, e));
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
                    self.session
                        .output()
                        .error(format!("failed to move download of '{}': {}", ident, e));
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

        let (proxy, user_agent) = {
            let config = self.session.config();
            (
                config.proxy().map(|s| s.to_owned()),
                self.session
                    .user_agent
                    .get()
                    .map(|s| s.as_str())
                    .unwrap_or(DEFAULT_USER_AGENT)
                    .to_owned(),
            )
        };

        let package_caches = self.caches.get_mut().unwrap();

        let mut total = 0u64;
        let mut estimated = false;

        for &pkg in self.packages.iter() {
            let pkg = pkg.upgradable().unwrap_or(pkg);
            let urls = pkg.download_urls();
            let filenames = pkg.download_filenames();
            let cookie = pkg.cookie().unwrap_or_default();

            for (url, filename) in urls.iter().zip(filenames.iter()) {
                let ident = pkg.ident();
                let package_cache = package_caches.get_mut(&ident).unwrap();
                let info = package_cache
                    .inner
                    .get_mut(filename)
                    .expect("failed to get cache info");

                // HEAD request via the unified network layer
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
                let opts = internal::network::RequestOptions {
                    proxy: proxy.as_deref(),
                    timeout_secs: 30,
                    user_agent: Some(&user_agent),
                    cookies: cookie_map.as_ref(),
                    ..internal::network::RequestOptions::default()
                };
                let result = self.session.network().head(url, &opts).await;
                let code = result.status_code;

                if code == 200 {
                    // head() returns both the status code and Content-Length
                    // in a single round trip; reusing it avoids a redundant
                    // second HEAD request per URL.
                    info.remote_size = result.content_length.unwrap_or(0);
                    if info.remote_size != info.local_size {
                        total += info.remote_size;
                    }
                } else {
                    debug!("code: {}, ident: {}, url: {}", code, ident, url)
                }

                if info.remote_size == 0 {
                    info.estimated = true;
                    estimated = true;
                }

                package_cache.update_valid_state();
            }
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
    pub dltotal: u64,
    pub dlnow: u64,
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
