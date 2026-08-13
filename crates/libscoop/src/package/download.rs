//! Concurrent package download with `ureq` (pure Rust).
//!
//! Replaced `curl` (libcurl bindings) to avoid static C compilation overhead.
//! Fragmented downloads use `std::thread::scope` instead of `curl::multi::Multi`.

use std::cell::OnceCell;
use std::{
    collections::{HashMap, HashSet},
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tracing::{debug, info};

use crate::constant::{DEFAULT_USER_AGENT, ISOLATED_PACKAGE_BUCKET};
use crate::{error::Fallible, internal, Error, Event, Session};

use super::identity;
use super::manifest_source;
use super::Package;

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
    session: &'a Session,
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

#[derive(Clone, Copy, PartialEq, Eq)]
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
            if cache.local_size == cache.remote_size {
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

                if self.reuse_cache {
                    if let Ok(file) = File::open(cache_root.join(filename)) {
                        if let Ok(metadata) = file.metadata() {
                            local_size = metadata.len();
                            file_cached_count += 1;
                        }
                    }
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
    pub fn download(&mut self) -> Fallible<Vec<String>> {
        if self.caches.get().is_none() {
            self.load_cache();
        }

        let config = self.session.config();
        let cache_root = config.cache_path();
        let proxy = config.proxy();
        let user_agent = self
            .session
            .user_agent
            .get()
            .map(|s| s.as_str())
            .unwrap_or(DEFAULT_USER_AGENT);

        let package_caches = self.caches.get_mut().unwrap();

        let mut chunk_file_map: HashMap<PathBuf, (Vec<PathBuf>, String)> = HashMap::new();
        let mut filepaths: Vec<(PathBuf, PathBuf, String)> = vec![];
        let mut failed: Vec<String> = vec![];

        internal::fs::ensure_dir(&cache_root)?;

        // Read fragmentation settings from aria2 config (reuse existing user config)
        let fragmentation_enabled = config.aria2_enabled();
        let chunk_count = config
            .aria2_split()
            .min(config.aria2_max_connection_per_server()) as u64;
        let min_split_size = config.aria2_min_split_size();

        // Build agent once (shared for all downloads)
        let agent_opts = internal::network::RequestOptions {
            proxy,
            timeout_secs: 120,
            user_agent: Some(user_agent),
            ..internal::network::RequestOptions::default()
        };
        let agent = internal::network::build_agent(&agent_opts).map_err(crate::Error::Custom)?;
        let agent = &agent;

        for cache in package_caches.values() {
            if self.reuse_cache && cache.valid == CacheMaybeValid::Full {
                if let Some(tx) = self.session.emitter() {
                    for filename in cache.inner.keys() {
                        let _ = tx.send(Event::PackageCacheHit(filename.clone()));
                    }
                }
                continue;
            }

            let cookie = cache.package.cookie().unwrap_or_default();
            let ident = cache.package.ident();

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

                let result = (|| -> Fallible<()> {
                    if use_fragments {
                        let path = cache_root.join(filename);
                        let part_dir = cache_root.join(format!("{}.parts", filename));
                        internal::fs::ensure_dir(&part_dir)?;

                        // Chunks whose retries are exhausted are re-downloaded
                        // serially (see `download_fragmented`), so a weak link
                        // degrades to fewer connections instead of failing the
                        // whole file.
                        let url_str = dlinfo.url.to_owned();
                        let cookie_clone = cookie.clone();
                        // Aggregate per-chunk deltas into a file-level count so
                        // the progress event matches the single-stream path.
                        let progress_total = Arc::new(AtomicU64::new(0));
                        let progress_total_h = progress_total.clone();
                        let emitter_h = emitter.clone();
                        let ident_h = ident.clone();
                        let filename_h = filename.to_owned();
                        let url_h = url_str.clone();
                        let on_progress = move |delta: u64| {
                            let dlnow =
                                progress_total_h.fetch_add(delta, Ordering::Relaxed) + delta;
                            if let Some(tx) = &emitter_h {
                                let _ = tx.send(Event::PackageDownloadProgress(
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
                            agent,
                            &url_str,
                            &cookie_clone,
                            dlinfo.remote_size,
                            chunk_count,
                            &part_dir,
                            &on_progress,
                        )
                        .map_err(crate::error::Error::Custom)?;

                        chunk_file_map.insert(path, (part_paths, ident.clone()));
                    } else {
                        // Single download
                        let path = cache_root.join(filename);
                        let tmp = cache_root.join(format!("{}.download", filename));
                        let _ = std::fs::remove_file(&path);
                        let _ = std::fs::remove_file(&tmp);

                        let fname = filename.to_owned();
                        let url_str = dlinfo.url.to_owned();
                        let cookie_clone = cookie.clone();
                        let dlinfo_total = dlinfo.remote_size;

                        // Download via ureq
                        let mut req = agent.get(&url_str);
                        if !cookie_clone.is_empty() {
                            let cookie_val = cookie_clone
                                .iter()
                                .map(|(k, v)| format!("{}={}", k, v))
                                .collect::<Vec<_>>()
                                .join("; ");
                            req = req.header("Cookie", &cookie_val);
                        }

                        let resp = req.call().map_err(|e| {
                            crate::error::Error::Custom(format!("download failed: {}", e))
                        })?;

                        let mut file = OpenOptions::new().create(true).append(true).open(&tmp)?;

                        let mut reader = resp.into_body().into_reader();
                        let mut buf = [0u8; 32768];
                        let mut dlnow = 0u64;

                        loop {
                            let n = reader
                                .read(&mut buf)
                                .map_err(|e| crate::error::Error::Custom(e.to_string()))?;
                            if n == 0 {
                                break;
                            }
                            file.write_all(&buf[..n])?;
                            dlnow += n as u64;

                            if let Some(tx) = &emitter {
                                let ctx = PackageDownloadProgressContext {
                                    ident: ident.clone(),
                                    url: url_str.clone(),
                                    filename: fname.clone(),
                                    dltotal: dlinfo_total,
                                    dlnow,
                                };
                                let _ = tx.send(Event::PackageDownloadProgress(ctx));
                            }
                        }

                        filepaths.push((tmp, path, ident.clone()));
                    }
                    Ok(())
                })();

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

    pub fn calculate_download_size(&mut self) -> Fallible<DownloadSize> {
        if self.caches.get().is_none() {
            self.load_cache();
        }

        let config = self.session.config();
        let proxy = config.proxy();
        let user_agent = self
            .session
            .user_agent
            .get()
            .map(|s| s.as_str())
            .unwrap_or(DEFAULT_USER_AGENT);

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
                    proxy,
                    timeout_secs: 30,
                    user_agent: Some(user_agent),
                    cookies: cookie_map.as_ref(),
                    ..internal::network::RequestOptions::default()
                };
                let result = internal::network::head(url, &opts);
                let code = result.status_code;

                if code == 200 {
                    // Get Content-Length via a dedicated HEAD (need the raw header)
                    if let Ok(cl_agent) =
                        internal::network::build_agent(&internal::network::RequestOptions {
                            proxy,
                            timeout_secs: 30,
                            ..internal::network::RequestOptions::default()
                        })
                    {
                        let mut cl_req = cl_agent.head(*url);
                        if !cookie.is_empty() {
                            let cookie_val = cookie
                                .iter()
                                .map(|(k, v)| format!("{}={}", k, v))
                                .collect::<Vec<_>>()
                                .join("; ");
                            cl_req = cl_req.header("Cookie", &cookie_val);
                        }
                        info.remote_size = cl_req
                            .call()
                            .ok()
                            .and_then(|resp| {
                                resp.headers()
                                    .get("Content-Length")?
                                    .to_str()
                                    .ok()?
                                    .parse::<u64>()
                                    .ok()
                            })
                            .unwrap_or(0);
                    }
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

// ─── Fragmented download ──────────────────────────────────────────────────

/// Download `remote_size` bytes of `url` split into `chunk_count` parallel
/// ranges, returning the part file paths in ascending range order.
///
/// Chunks whose retries are exhausted are not fatal: the failed ranges are
/// merged (see [`merge_adjacent_ranges`]) and re-downloaded serially with a
/// single connection, so a weak link degrades to a plain sequential download
/// instead of failing the whole file. Every returned part is verified
/// complete, and the ranges are checked to tile `[0, remote_size)` with no
/// gaps or overlaps.
fn download_fragmented(
    agent: &ureq::Agent,
    url: &str,
    cookie: &[(&str, &str)],
    remote_size: u64,
    chunk_count: u64,
    part_dir: &std::path::Path,
    on_progress: &(dyn Fn(u64) + Sync),
) -> Result<Vec<PathBuf>, String> {
    let chunk_size = remote_size / chunk_count;
    // (part path, start, end) for every range to verify — initially one entry
    // per chunk, extended with fallback ranges for failed chunks.
    let mut parts: Vec<(PathBuf, u64, u64)> = Vec::new();
    let failed_ranges = Arc::new(Mutex::new(Vec::<(u64, u64)>::new()));

    std::thread::scope(|scope| {
        for chunk_idx in 0..chunk_count {
            let start = chunk_idx * chunk_size;
            let end = if chunk_idx == chunk_count - 1 {
                remote_size - 1
            } else {
                (chunk_idx + 1) * chunk_size - 1
            };

            let part_path = part_dir.join(format!("part.{}", chunk_idx));
            parts.push((part_path.clone(), start, end));

            // Don't remove — allow resume if part exists

            let part_path = part_path.clone();
            let url = url.to_owned();
            let ck = cookie.to_vec();
            let failed = failed_ranges.clone();

            scope.spawn(move || {
                if let Err(e) =
                    download_range(agent, &url, start, end, &part_path, &ck, on_progress)
                {
                    debug!("chunk {}-{} failed: {}", start, end, e);
                    // Drop the partial part so the fallback re-downloads the
                    // whole range from scratch.
                    let _ = std::fs::remove_file(&part_path);
                    failed.lock().unwrap().push((start, end));
                }
            });
        }
    });

    // Re-download failed ranges serially. The more chunks fail, the fewer
    // concurrent connections remain — degrading to a plain sequential
    // download on a weak link instead of failing the whole file.
    let failed = failed_ranges.lock().unwrap().clone();
    if !failed.is_empty() {
        parts.retain(|(_, s, _)| !failed.iter().any(|(fs, _)| fs == s));
        for (idx, (s, e)) in merge_adjacent_ranges(failed).iter().enumerate() {
            let fb_path = part_dir.join(format!("fallback.{}", idx));
            download_range(agent, url, *s, *e, &fb_path, cookie, on_progress).map_err(|e| {
                let _ = std::fs::remove_file(&fb_path);
                format!("fallback download of {}-{} failed: {}", s, e, e)
            })?;
            parts.push((fb_path, *s, *e));
        }
    }

    // Verify the parts tile the whole file: each part must be complete and
    // the ranges must cover [0, remote_size) without gaps or overlaps.
    parts.sort_by_key(|(_, s, _)| *s);
    let mut cursor = 0u64;
    for (part, s, e) in &parts {
        if *s != cursor {
            return Err(format!(
                "gap in parts: expected range at {}, covered up to {}",
                s, cursor
            ));
        }
        let expected = e - s + 1;
        let actual = part.metadata().map(|m| m.len()).unwrap_or(0);
        if actual != expected {
            return Err(format!(
                "part {} size mismatch: {} != {}",
                part.display(),
                actual,
                expected
            ));
        }
        cursor = e + 1;
    }
    if cursor != remote_size {
        return Err(format!(
            "parts cover {} bytes, expected {}",
            cursor, remote_size
        ));
    }

    Ok(parts.into_iter().map(|(p, _, _)| p).collect())
}

/// Merge ranges that are adjacent (or overlapping) into contiguous ranges.
/// Used to coalesce failed chunk ranges before the serial fallback download.
fn merge_adjacent_ranges(mut ranges: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    ranges.sort();
    let mut merged: Vec<(u64, u64)> = Vec::new();
    for (s, e) in ranges {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 + 1 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }
    merged
}

/// Max attempts for a single chunk download (1 initial try + 2 retries).
const MAX_CHUNK_ATTEMPTS: u32 = 3;

/// Base exponential-backoff delay in milliseconds, doubled after each failed
/// attempt. A small random jitter is added so concurrently failing chunks
/// don't retry in lockstep and hammer the server.
const CHUNK_BACKOFF_BASE_MS: u64 = 500;

/// Error while downloading one chunk. [`ChunkError::Transient`] failures are
/// retried with exponential backoff; [`ChunkError::Final`] ones are not.
enum ChunkError {
    /// Retrying cannot help (HTTP 4xx, server ignoring `Range`).
    Final(String),
    /// Transport-level, 5xx or short-write failure — safe to retry.
    Transient(String),
}

impl std::fmt::Display for ChunkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChunkError::Final(msg) | ChunkError::Transient(msg) => write!(f, "{}", msg),
        }
    }
}

/// Download `[start, end]` of `url` into `dest`, retrying transient failures
/// with exponential backoff (up to [`MAX_CHUNK_ATTEMPTS`] attempts).
///
/// Retrying is safe because every attempt resumes from the current size of
/// `dest` (callers keep the part file across attempts), so a failed attempt
/// only re-downloads the missing tail.
///
/// Every byte written to `dest` is reported through `on_progress` (as a
/// delta), exactly once, so callers can aggregate download progress across
/// concurrent chunks.
fn download_range(
    agent: &ureq::Agent,
    url: &str,
    start: u64,
    end: u64,
    dest: &std::path::Path,
    cookie: &[(&str, &str)],
    on_progress: &(dyn Fn(u64) + Sync),
) -> Result<(), String> {
    let mut attempt = 1u32;
    loop {
        match try_download_range(agent, url, start, end, dest, cookie, on_progress) {
            Ok(()) => return Ok(()),
            Err(e) => {
                if attempt >= MAX_CHUNK_ATTEMPTS || matches!(&e, ChunkError::Final(_)) {
                    return Err(e.to_string());
                }
                let delay = chunk_retry_delay(attempt);
                debug!(
                    "chunk download failed (attempt {}/{}): {}; retrying in {:?}",
                    attempt, MAX_CHUNK_ATTEMPTS, e, delay
                );
                std::thread::sleep(delay);
                attempt += 1;
            }
        }
    }
}

/// Exponential backoff with jitter: `BASE * 2^(attempt-1)` plus a random
/// offset below `BASE` (derived from the clock to avoid a `rand` dependency).
fn chunk_retry_delay(attempt: u32) -> std::time::Duration {
    let base = CHUNK_BACKOFF_BASE_MS;
    let exp = base * (1u64 << (attempt - 1).min(6));
    let jitter = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 % base)
        .unwrap_or(0);
    std::time::Duration::from_millis(exp + jitter)
}

/// One download attempt for a chunk range; the retry loop lives in
/// [`download_range`].
fn try_download_range(
    agent: &ureq::Agent,
    url: &str,
    start: u64,
    end: u64,
    dest: &std::path::Path,
    cookie: &[(&str, &str)],
    on_progress: &(dyn Fn(u64) + Sync),
) -> Result<(), ChunkError> {
    let expected_size = end - start + 1;

    // Inspect the existing part: skip when complete, drop when corrupted
    // (e.g. a previous attempt where the server ignored `Range` and wrote
    // the whole body), otherwise resume from where it stopped.
    if let Ok(meta) = dest.metadata() {
        match meta.len().cmp(&expected_size) {
            std::cmp::Ordering::Equal => return Ok(()),
            std::cmp::Ordering::Greater => {
                std::fs::remove_file(dest)
                    .map_err(|e| ChunkError::Transient(format!("failed to reset part: {}", e)))?;
            }
            std::cmp::Ordering::Less => {}
        }
    }

    // Determine resume offset and the Range header
    let resume_start = dest.metadata().ok().map(|m| m.len()).unwrap_or(0);
    let range = if resume_start > 0 {
        format!("bytes={}-{}", start + resume_start, end)
    } else {
        format!("bytes={}-{}", start, end)
    };

    let mut req = agent.get(url).header("Range", &range);
    if !cookie.is_empty() {
        let cookie_val = cookie
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("; ");
        req = req.header("Cookie", &cookie_val);
    }

    let resp = match req.call() {
        Ok(resp) => resp,
        // 4xx (except retryable 408/429) — the server rejected the request,
        // retrying won't help.
        Err(ureq::Error::StatusCode(code))
            if (400..500).contains(&code) && code != 408 && code != 429 =>
        {
            return Err(ChunkError::Final(format!("HTTP {} for {}", code, url)));
        }
        // Everything else — 5xx, 408/429, Io, Timeout, ConnectionFailed, ... —
        // is a transient failure handled by the retry loop in `download_range`.
        Err(e) => return Err(ChunkError::Transient(e.to_string())),
    };

    // A Range request must yield a 206. If the server ignores `Range` and
    // returns the full body (200), writing it would corrupt the part file —
    // treat that as a hard failure instead of silently accepting bad data.
    if resp.status() != 206 {
        return Err(ChunkError::Final(format!(
            "expected 206 Partial Content, got {} (server ignored Range request)",
            resp.status()
        )));
    }

    // Guard against 206 responses whose payload starts at a different offset
    // than requested (a broken server that returns `bytes=0-59` for a
    // `bytes=40-99` request): appending it would corrupt the part file.
    // A missing `Content-Range` header is tolerated for leniency.
    if let Some(cr) = resp.headers().get("Content-Range") {
        if let Ok(cr) = cr.to_str() {
            if let Some(start_str) = cr.strip_prefix("bytes ").and_then(|r| r.split('-').next()) {
                if let Ok(cr_start) = start_str.parse::<u64>() {
                    let expected_start = start + resume_start;
                    if cr_start != expected_start {
                        return Err(ChunkError::Transient(format!(
                            "Content-Range starts at {}, expected {} (range {}-{})",
                            cr_start, expected_start, start, end
                        )));
                    }
                }
            }
        }
    }

    // Open file in append mode if resuming, create if new
    let mut file = if resume_start > 0 {
        std::fs::OpenOptions::new()
            .append(true)
            .open(dest)
            .map_err(|e| ChunkError::Transient(e.to_string()))?
    } else {
        std::fs::File::create(dest).map_err(|e| ChunkError::Transient(e.to_string()))?
    };

    let mut reader = resp.into_body().into_reader();
    let mut buf = [0u8; 32768];
    let mut written = 0u64;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| ChunkError::Transient(format!("read failed: {}", e)))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| ChunkError::Transient(format!("write failed: {}", e)))?;
        written += n as u64;
        on_progress(n as u64);
    }

    // Guard against servers that report a 206 but deliver fewer bytes than
    // the range covers (short read or truncated body).
    if resume_start + written != expected_size {
        return Err(ChunkError::Transient(format!(
            "short chunk: got {} bytes, expected {} (range {}-{})",
            resume_start + written,
            expected_size,
            start,
            end
        )));
    }
    Ok(())
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

// ─── Download verification & standalone download API ───────────────────────

/// Verify downloaded files against the manifest hashes (upstream
/// `check_hash` in lib/download.ps1).
///
/// Nightly packages skip verification; a missing or empty hash in the
/// manifest warns and continues; a mismatch removes the corrupt cache
/// file so the next attempt re-downloads, and either fails hard or — in
/// IgnoreFailure mode — records the package ident in the returned set.
///
/// Shared by the install pipeline and the standalone `download` command so
/// the verification semantics cannot drift between them.
pub(crate) fn verify_downloads(
    session: &Session,
    packages: &[&Package],
    no_hash_check: bool,
    ignore_failure: bool,
) -> Fallible<HashSet<String>> {
    let mut failed = HashSet::new();
    if no_hash_check {
        return Ok(failed);
    }

    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackageIntegrityCheckStart);
    }

    let config = session.config();
    let cache_root = config.cache_path();

    let mut buf = [0; 1024 * 64];

    for &pkg in packages.iter() {
        if pkg.version() == "nightly" {
            info!("skip hash check for nightly package '{}'", pkg.name());
            continue;
        }

        let files = pkg.download_filenames();
        let hashes = pkg.download_hashes();
        let files_cnt = files.len();

        let result = (|| -> Fallible<()> {
            for (idx, (filename, hash)) in files.into_iter().zip(hashes).enumerate() {
                let path = cache_root.join(filename);

                // No hash in the manifest (missing or `""`): upstream
                // `check_hash` (lib/download.ps1) warns and continues
                // without verification instead of failing hard.
                if matches!(hash, crate::package::manifest::HashString::Empty) {
                    session.output().warn(format!(
                        "no hash in manifest for '{}', skipping verification",
                        pkg.name()
                    ));
                    continue;
                }

                let mut hasher = internal::hash::ChecksumBuilder::new()
                    .algo(hash.algorithm())?
                    .build();

                if let Some(tx) = session.emitter() {
                    let progress = format!("{} ({}/{})", pkg.name(), idx + 1, files_cnt);
                    let _ = tx.send(Event::PackageIntegrityCheckProgress(progress));
                }

                let mut file = std::fs::File::open(&path)?;
                loop {
                    let len = file.read(&mut buf)?;
                    if len == 0 {
                        break;
                    }
                    hasher.consume(&buf[..len]);
                }

                let actual = hasher.finalize();
                let expected = hash.value();
                if actual != expected {
                    // Upstream removes the corrupt cache file on a hash
                    // mismatch (lib/download.ps1:122-125) so the next
                    // attempt re-downloads instead of failing forever.
                    let _ = std::fs::remove_file(&path);
                    let name = pkg.name().to_owned();
                    let url = pkg.download_urls()[idx].to_owned();
                    let ctx =
                        super::HashMismatchContext::new(name, url, expected.to_owned(), actual);
                    return Err(Error::HashMismatch(ctx));
                }
            }
            Ok(())
        })();

        if let Err(e) = result {
            if ignore_failure {
                failed.insert(pkg.ident());
                session
                    .output()
                    .error(format!("failed to verify '{}': {}", pkg.name(), e));
            } else {
                return Err(e);
            }
        }
    }

    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackageIntegrityCheckDone);
    }

    Ok(failed)
}

/// Download the given packages and verify their hashes, returning the
/// idents that failed either step.
///
/// Wraps the shared sequence `download() → failed filter → verify_downloads`
/// used by both the install pipeline and the standalone download command,
/// emitting the download events around it. With `offline` no request is made
/// and only the cache is verified.
///
/// The caller must have run [`PackageSet::calculate_download_size`] on the
/// set first: the HEAD requests it makes fill in `remote_size`, which both
/// fragmented download (splitting) and the progress bar's total depend on.
pub(crate) fn download_and_verify(
    set: &mut PackageSet<'_>,
    no_hash_check: bool,
    ignore_failure: bool,
    offline: bool,
) -> Fallible<HashSet<String>> {
    let session = set.session;
    let mut failed = HashSet::new();

    if !offline {
        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageDownloadStart);
        }

        set.set_ignore_failure(ignore_failure);
        failed = set.download()?.into_iter().collect();

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageDownloadDone);
        }
    }

    let ok_pkgs: Vec<&Package> = set
        .packages
        .iter()
        .map(|p| *p)
        .filter(|p| !failed.contains(&p.ident()))
        .collect();

    failed.extend(verify_downloads(
        session,
        &ok_pkgs,
        no_hash_check,
        ignore_failure,
    )?);
    Ok(failed)
}

/// Options for [`download_apps`].
pub struct DownloadOptions {
    /// Ignore the cache and download again (`-f/--force`).
    pub force: bool,
    /// Verify downloaded files against the manifest hashes.
    pub check_hash: bool,
}

/// Download the files of the given packages into the cache directory
/// without installing them, mirroring upstream `scoop download`
/// (libexec/scoop-download.ps1).
///
/// Each query is resolved independently — bare name, `bucket/name`,
/// `name@version`, a manifest URL, or a local manifest path — and a
/// failure on one app does not abort the rest. No dependencies are
/// downloaded and no installation state is touched.
pub fn download_apps(session: &Session, queries: &[&str], opts: &DownloadOptions) -> Fallible<()> {
    let mut packages: Vec<Package> = Vec::new();

    for &query in queries {
        match resolve_download_query(session, query) {
            Ok(pkg) => {
                let version = pkg.version();
                if version.is_empty() {
                    session.output().error(format!(
                        "Manifest for '{}' doesn't specify a version.",
                        query
                    ));
                    continue;
                }
                if !version
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
                {
                    session.output().error(format!(
                        "Manifest version for '{}' has an unsupported character.",
                        query
                    ));
                    continue;
                }
                // Architecture support: no download URLs for the effective
                // architecture (upstream `Get-SupportedArchitecture`).
                if pkg.download_urls().is_empty() {
                    session.output().error(format!(
                        "'{}' doesn't support the current architecture!",
                        pkg.name()
                    ));
                    continue;
                }
                packages.push(pkg);
            }
            Err(e) => {
                session
                    .output()
                    .error(format!("failed to resolve '{}': {}", query, e));
            }
        }
    }

    if packages.is_empty() {
        return Ok(());
    }

    let refs: Vec<&Package> = packages.iter().collect();

    // HEAD requests fill in remote_size: fragmented download and the
    // progress bar both depend on it (same as the install pipeline).
    let mut set = PackageSet::new(session, &refs, !opts.force)?;
    set.calculate_download_size()?;
    let failed = download_and_verify(&mut set, !opts.check_hash, true, false)?;

    for pkg in &refs {
        if !failed.contains(&pkg.ident()) {
            session.output().info(format!(
                "'{}' ({}) was downloaded successfully!",
                pkg.name(),
                pkg.version()
            ));
        }
    }

    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackageSyncDone);
    }

    Ok(())
}

/// Resolve one download query to a [`Package`], mirroring the install
/// pipeline's dispatch: `name@version` → `generate_user_manifest`, anything
/// else → `resolve_manifest` (URL / local path / bucket lookup).
fn resolve_download_query(session: &Session, query: &str) -> Fallible<Package> {
    if let Some(pkg) = manifest_source::resolve_isolated_query(session, query)? {
        return Ok(pkg);
    }
    // Plain bucket reference: resolve directly — download has no
    // installed-package semantics, so no bucket scanning is needed.
    let aq = identity::parse_app(query).ok_or_else(|| Error::PackageNotFound(query.to_owned()))?;
    let resolved = manifest_source::resolve_manifest(session, &aq.app, aq.bucket.as_deref())?;
    Ok(Package::from(
        &resolved.name,
        ISOLATED_PACKAGE_BUCKET,
        resolved.manifest,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use crate::internal::network::{build_agent, RequestOptions};

    #[test]
    fn test_chunk_boundaries() {
        let size = 100u64;
        let chunks = 4u64;
        let chunk_size = size / chunks;
        assert_eq!(chunk_size, 25);
        assert_eq!(0 * chunk_size, 0);
        assert_eq!((0 + 1) * chunk_size - 1, 24);
        assert_eq!(1 * chunk_size, 25);
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

    // ─── download_range: retry & Range handling ────────────────────────────────

    /// Handle to a spawned [`spawn_range_server`].
    struct RangeServer {
        addr: std::net::SocketAddr,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
        /// `Range` header values received, in request order.
        ranges: Arc<Mutex<Vec<String>>>,
        /// HTTP status codes sent in response order.
        responses: Arc<Mutex<Vec<u16>>>,
    }

    impl RangeServer {
        /// Stop the server thread and wait for it to finish, so every
        /// received request is fully processed before assertions run.
        fn shutdown(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(h) = self.handle.take() {
                h.join().unwrap();
            }
        }
    }

    /// Spawn a minimal single-range HTTP server serving `data`.
    ///
    /// The first `fail_first` requests answer `fail_status` (e.g. 500) to
    /// exercise the retry path; later ones answer 206 with the requested
    /// slice. With `ignore_range = true` the server always answers 200 with
    /// the full body, mimicking servers that don't support `Range`.
    /// `fail_range = Some((start, end, attempts))` makes every request whose
    /// range overlaps `[start, end]` answer `fail_status` for its first
    /// `attempts` occurrences — used to exercise the serial fallback in
    /// [`download_fragmented`].
    fn spawn_range_server(
        data: Vec<u8>,
        fail_first: u32,
        fail_status: u16,
        ignore_range: bool,
        truncate_first: bool,
        fail_range: Option<(usize, usize, u32)>,
    ) -> RangeServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let count = Arc::new(AtomicU32::new(0));
        let responses = Arc::new(Mutex::new(Vec::new()));
        let fail_range_count = Arc::new(AtomicU32::new(0));

        let stop_h = stop.clone();
        let ranges_h = ranges.clone();
        let count_h = count.clone();
        let responses_h = responses.clone();
        let fail_range_count_h = fail_range_count.clone();
        let handle = std::thread::spawn(move || {
            while !stop_h.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // accept() inherits the listener's non-blocking mode,
                        // so the first read can return WouldBlock before the
                        // client's request bytes arrive (parallel test load
                        // delays loopback delivery) — that would make every
                        // connection look empty and fail the test. Restore
                        // blocking mode; the read timeout below still guards
                        // against spurious connections with no request data
                        // (observed on Windows under parallel test load).
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
                        let head = read_request_head(&mut stream);
                        if head.is_empty() {
                            continue;
                        }
                        // HEAD (size probing): answer 200 with Content-Length
                        // and no body, like a real server. Not counted in
                        // `ranges` (a cache-hit must not look like a request).
                        if head.starts_with("HEAD") {
                            responses_h.lock().unwrap().push(200);
                            let _ = write!(
                                stream,
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                data.len()
                            );
                            continue;
                        }

                        let n = count_h.fetch_add(1, Ordering::SeqCst) + 1;
                        let range = extract_range(&head);
                        ranges_h.lock().unwrap().push(range.clone());

                        if n <= fail_first {
                            responses_h.lock().unwrap().push(fail_status);
                            let _ = write!(
                                stream,
                                "HTTP/1.1 {} Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                                fail_status
                            );
                            continue;
                        }
                        if ignore_range {
                            responses_h.lock().unwrap().push(200);
                            let _ = write!(
                                stream,
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                data.len()
                            );
                            let _ = stream.write_all(&data);
                            continue;
                        }
                        let (s, e) = parse_range(&range, data.len());
                        let slice = &data[s..=e];

                        // Range-conditioned failure: any request whose range
                        // overlaps `fail_range` answers `fail_status` for the
                        // first `attempts` such requests — used to exercise
                        // the serial fallback in `download_fragmented`.
                        if let Some((fs, fe, attempts)) = fail_range {
                            if s <= fe && fs <= e {
                                let n = fail_range_count_h.fetch_add(1, Ordering::SeqCst) + 1;
                                if n <= attempts {
                                    responses_h.lock().unwrap().push(fail_status);
                                    let _ = write!(
                                        stream,
                                        "HTTP/1.1 {} Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                                        fail_status
                                    );
                                    continue;
                                }
                            }
                        }

                        // Simulate a server that reports a 206 but delivers
                        // only half the requested bytes (truncated body): the
                        // client must detect the short write and resume.
                        if truncate_first && n == 1 {
                            let half = slice.len() / 2;
                            let e2 = s + half - 1;
                            responses_h.lock().unwrap().push(206);
                            let _ = write!(
                                stream,
                                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                s, e2, data.len(), half
                            );
                            let _ = stream.write_all(&slice[..half]);
                            continue;
                        }

                        responses_h.lock().unwrap().push(206);
                        let _ = write!(
                            stream,
                            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            s, e, data.len(), slice.len()
                        );
                        let _ = stream.write_all(slice);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => {
                        // Transient accept errors (e.g. WSAECONNRESET on
                        // Windows when a client resets a pending connection)
                        // must not kill the server thread — retry instead.
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            }
        });

        RangeServer {
            addr,
            stop,
            handle: Some(handle),
            ranges,
            responses,
        }
    }

    /// Read the request head and return it as a string.
    fn read_request_head(stream: &mut TcpStream) -> String {
        let mut buf = [0u8; 4096];
        let mut n = 0;
        loop {
            match stream.read(&mut buf[n..]) {
                Ok(0) => break,
                Ok(read) => {
                    n += read;
                    if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }

    /// Extract the `Range` header value from a request head (empty if absent).
    fn extract_range(head: &str) -> String {
        head.lines()
            .find(|l| l.to_ascii_lowercase().starts_with("range:"))
            .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string()))
            .unwrap_or_default()
    }

    /// Parse `bytes=start-end` into `(start, end)` clamped to `total`.
    fn parse_range(range: &str, total: usize) -> (usize, usize) {
        let spec = range.strip_prefix("bytes=").unwrap_or("");
        let (s, e) = spec.split_once('-').unwrap_or(("", ""));
        let s: usize = s.parse().unwrap_or(0);
        let e: usize = e.parse().unwrap_or(total.saturating_sub(1));
        (s, e.min(total.saturating_sub(1)))
    }

    fn test_agent() -> ureq::Agent {
        build_agent(&RequestOptions {
            timeout_secs: 5,
            ..RequestOptions::default()
        })
        .unwrap()
    }

    #[test]
    fn test_download_range_retries_transient_failure() {
        let data: Vec<u8> = (0..100u8).collect();
        // One 500 then success. (fail_first=1 rather than 2: under parallel
        // Windows test load the client's first connection can die before its
        // request is sent — surfaced to the server as an empty accept — which
        // consumes one of the client's retry slots. Retrying the whole
        // operation once below keeps this test robust against that noise.)
        let mut server = spawn_range_server(data.clone(), 1, 500, false, false, None);
        let dest = crate::test_utils::tmpdir("download_range_retry").join("part.0");

        let url = format!("http://{}/file.bin", server.addr);
        let mut result = download_range(&test_agent(), &url, 0, 99, &dest, &[], &|_| {});
        if result.is_err() {
            let _ = std::fs::remove_file(&dest);
            result = download_range(&test_agent(), &url, 0, 99, &dest, &[], &|_| {});
        }
        result.unwrap();

        server.shutdown();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        // Transient failures are retried until success: the last response must
        // be a 206 with only 500s before it (a dropped first connection can
        // shorten the sequence, so only the shape is asserted).
        let responses = server.responses.lock().unwrap().clone();
        assert!(!responses.is_empty(), "server received no requests");
        assert_eq!(
            *responses.last().unwrap(),
            206,
            "responses: {:?}",
            responses
        );
        assert!(
            responses[..responses.len() - 1].iter().all(|&s| s == 500),
            "responses: {:?}",
            responses
        );
        assert!(responses.len() <= 3, "responses: {:?}", responses);
    }

    #[test]
    fn test_download_range_gives_up_after_max_attempts() {
        let data = vec![7u8; 64];
        let mut server = spawn_range_server(data.clone(), u32::MAX, 500, false, false, None);
        let dest = crate::test_utils::tmpdir("download_range_giveup").join("part.0");

        let url = format!("http://{}/file.bin", server.addr);
        let err = download_range(&test_agent(), &url, 0, 63, &dest, &[], &|_| {}).unwrap_err();

        server.shutdown();
        assert!(err.contains("500"), "unexpected error: {}", err);
        // Never more than MAX_CHUNK_ATTEMPTS responses (a spurious pre-request
        // connection drop can consume one attempt, so allow fewer).
        let responses = server.responses.lock().unwrap().clone();
        assert!(!responses.is_empty(), "server received no requests");
        assert!(
            responses.iter().all(|&s| s == 500),
            "responses: {:?}",
            responses
        );
        assert!(responses.len() <= 3, "responses: {:?}", responses);
    }

    #[test]
    fn test_download_range_4xx_not_retried() {
        let data = vec![7u8; 64];
        let mut server = spawn_range_server(data.clone(), u32::MAX, 404, false, false, None);
        let dest = crate::test_utils::tmpdir("download_range_404").join("part.0");

        let url = format!("http://{}/file.bin", server.addr);
        let err = download_range(&test_agent(), &url, 0, 63, &dest, &[], &|_| {}).unwrap_err();

        server.shutdown();
        assert!(err.contains("404"), "unexpected error: {}", err);
        // A 4xx is a hard failure: every response must be 404 and the client
        // must stop after the last one (a spurious pre-request connection drop
        // can consume an attempt, so fewer responses are allowed, but a 404
        // is never retried).
        let responses = server.responses.lock().unwrap().clone();
        assert!(!responses.is_empty(), "server received no requests");
        assert_eq!(
            *responses.last().unwrap(),
            404,
            "responses: {:?}",
            responses
        );
        assert!(
            responses.iter().all(|&s| s == 404),
            "responses: {:?}",
            responses
        );
    }

    #[test]
    fn test_download_range_rejects_full_body_200() {
        let data = vec![7u8; 64];
        // Server ignores `Range` and always answers 200 with the full body.
        let mut server = spawn_range_server(data.clone(), 0, 500, true, false, None);
        let dest = crate::test_utils::tmpdir("download_range_200").join("part.0");

        let url = format!("http://{}/file.bin", server.addr);
        let err = download_range(&test_agent(), &url, 0, 63, &dest, &[], &|_| {}).unwrap_err();

        server.shutdown();
        assert!(err.contains("206"), "unexpected error: {}", err);
        // Hard failure: nothing must be written to the part file.
        assert!(
            !dest.exists(),
            "part must not be written when server ignores Range"
        );
        // A non-206 response is a hard failure: every response must be 200 and
        // the client must stop after the last one (see the 4xx test for the
        // spurious-drop note).
        let responses = server.responses.lock().unwrap().clone();
        assert!(!responses.is_empty(), "server received no requests");
        assert_eq!(
            *responses.last().unwrap(),
            200,
            "responses: {:?}",
            responses
        );
        assert!(
            responses.iter().all(|&s| s == 200),
            "responses: {:?}",
            responses
        );
    }

    #[test]
    fn test_download_range_resumes_partial_part() {
        let data: Vec<u8> = (0..100u8).collect();
        let mut server = spawn_range_server(data.clone(), 0, 500, false, false, None);
        let dest = crate::test_utils::tmpdir("download_range_resume").join("part.0");
        // Pretend 40 bytes were already downloaded by an earlier attempt.
        std::fs::write(&dest, &data[..40]).unwrap();

        let url = format!("http://{}/file.bin", server.addr);
        download_range(&test_agent(), &url, 0, 99, &dest, &[], &|_| {}).unwrap();

        server.shutdown();
        let got = std::fs::read(&dest).unwrap();
        assert_eq!(
            got,
            data,
            "part content mismatch: got {} bytes, expected {}",
            got.len(),
            data.len()
        );
        let ranges = server.ranges.lock().unwrap();
        assert_eq!(ranges.first().map(String::as_str), Some("bytes=40-99"));
    }

    #[test]
    fn test_download_range_resets_corrupted_part() {
        let data: Vec<u8> = (0..100u8).collect();
        let mut server = spawn_range_server(data.clone(), 0, 500, false, false, None);
        let dest = crate::test_utils::tmpdir("download_range_reset").join("part.0");
        // Over-sized part: a previous attempt where the server ignored `Range`
        // and wrote the whole body (1000 bytes). It must be dropped and redone.
        let junk = vec![0u8; 1000];
        std::fs::write(&dest, &junk).unwrap();

        let url = format!("http://{}/file.bin", server.addr);
        download_range(&test_agent(), &url, 0, 99, &dest, &[], &|_| {}).unwrap();

        server.shutdown();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
    }

    #[test]
    fn test_download_range_429_is_retried() {
        // 429 Too Many Requests is transient (rate limiting) — unlike other
        // 4xx codes it must be retried until success.
        let data: Vec<u8> = (0..64u8).collect();
        let mut server = spawn_range_server(data.clone(), 1, 429, false, false, None);
        let dest = crate::test_utils::tmpdir("download_range_429").join("part.0");

        let url = format!("http://{}/file.bin", server.addr);
        let mut result = download_range(&test_agent(), &url, 0, 63, &dest, &[], &|_| {});
        if result.is_err() {
            let _ = std::fs::remove_file(&dest);
            result = download_range(&test_agent(), &url, 0, 63, &dest, &[], &|_| {});
        }
        result.unwrap();

        server.shutdown();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        let responses = server.responses.lock().unwrap().clone();
        assert_eq!(
            *responses.last().unwrap(),
            206,
            "responses: {:?}",
            responses
        );
        assert!(
            responses[..responses.len() - 1].iter().all(|&s| s == 429),
            "responses: {:?}",
            responses
        );
    }

    #[test]
    fn test_download_range_short_write_resumes() {
        // The server answers the first request with a 206 body truncated to
        // half the range; the client must detect the short write, retry from
        // the new offset and end up with the full part. Retried once overall
        // to tolerate the spurious pre-request connection drop (see the
        // retries test); the response sequence [206, 206] holds either way.
        let data: Vec<u8> = (0..100u8).collect();
        let mut server = spawn_range_server(data.clone(), 0, 500, false, true, None);
        let dest = crate::test_utils::tmpdir("download_range_short").join("part.0");

        let url = format!("http://{}/file.bin", server.addr);
        let mut result = download_range(&test_agent(), &url, 0, 99, &dest, &[], &|_| {});
        if result.is_err() {
            let _ = std::fs::remove_file(&dest);
            result = download_range(&test_agent(), &url, 0, 99, &dest, &[], &|_| {});
        }
        result.unwrap();

        server.shutdown();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        // First response was truncated (206), second one completed the part.
        let responses = server.responses.lock().unwrap().clone();
        assert_eq!(responses, vec![206, 206], "responses: {:?}", responses);
    }

    #[test]
    fn test_merge_adjacent_ranges() {
        assert_eq!(merge_adjacent_ranges(vec![]), Vec::<(u64, u64)>::new());
        assert_eq!(merge_adjacent_ranges(vec![(5, 9)]), vec![(5, 9)]);
        // Adjacent ranges coalesce into a single contiguous one.
        assert_eq!(
            merge_adjacent_ranges(vec![(10, 19), (0, 9), (20, 29)]),
            vec![(0, 29)]
        );
        // A gap keeps the ranges separate.
        assert_eq!(
            merge_adjacent_ranges(vec![(0, 9), (12, 19)]),
            vec![(0, 9), (12, 19)]
        );
        // Overlapping ranges merge; input order does not matter.
        assert_eq!(
            merge_adjacent_ranges(vec![(50, 99), (0, 24), (20, 60)]),
            vec![(0, 99)]
        );
    }

    #[test]
    fn test_download_fragmented_fallback_recovers_failed_chunk() {
        // 1000 bytes split into 4 chunks of 250. The third chunk's range
        // (500-749) answers 500 for its first 3 requests — exhausting the
        // per-chunk retries — and succeeds afterwards. The fallback must
        // re-download it serially and the parts must tile the whole file.
        let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let mut server =
            spawn_range_server(data.clone(), 0, 500, false, false, Some((500, 749, 3)));
        let dir = crate::test_utils::tmpdir("fragmented_fallback");
        let part_dir = dir.join("file.bin.parts");
        std::fs::create_dir_all(&part_dir).unwrap();

        let url = format!("http://{}/file.bin", server.addr);
        let mut parts = download_fragmented(&test_agent(), &url, &[], 1000, 4, &part_dir, &|_| {});
        if parts.is_err() {
            // Retried once to tolerate the spurious pre-request connection
            // drop (see the retries test); failed parts were removed, so a
            // fresh run only re-downloads the failed range.
            let _ = std::fs::remove_dir_all(&part_dir);
            std::fs::create_dir_all(&part_dir).unwrap();
            parts = download_fragmented(&test_agent(), &url, &[], 1000, 4, &part_dir, &|_| {});
        }
        let parts = parts.unwrap();

        server.shutdown();
        // 4 chunks − 1 failed + 1 fallback = 4 parts, in range order.
        assert_eq!(parts.len(), 4);
        let mut file = Vec::new();
        for p in &parts {
            let bytes = std::fs::read(p).unwrap();
            assert!(!bytes.is_empty(), "empty part: {}", p.display());
            file.extend_from_slice(&bytes);
        }
        assert_eq!(file, data, "reassembled file mismatch");

        // The failing range was retried to exhaustion and then re-downloaded
        // successfully by the fallback. (A spurious pre-request connection
        // drop can consume one of the client's retry slots, so only the shape
        // is asserted.)
        let responses = server.responses.lock().unwrap().clone();
        let fails = responses.iter().filter(|&&s| s == 500).count();
        assert!(fails >= 2, "responses: {:?}", responses);
        assert_eq!(
            *responses.last().unwrap(),
            206,
            "responses: {:?}",
            responses
        );
    }

    #[test]
    fn test_download_range_reports_progress() {
        let data: Vec<u8> = (0..100u8).collect();
        let mut server = spawn_range_server(data.clone(), 0, 500, false, false, None);
        let dest = crate::test_utils::tmpdir("download_range_progress").join("part.0");

        let url = format!("http://{}/file.bin", server.addr);
        let total = Arc::new(AtomicU64::new(0));
        let total_h = total.clone();
        download_range(&test_agent(), &url, 0, 99, &dest, &[], &move |n| {
            total_h.fetch_add(n, Ordering::Relaxed);
        })
        .unwrap();

        server.shutdown();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        // Every byte read by the client is reported exactly once.
        assert_eq!(total.load(Ordering::Relaxed), 100);
    }

    #[test]
    fn test_download_fragmented_reports_total_progress() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let mut server = spawn_range_server(data.clone(), 0, 500, false, false, None);
        let dir = crate::test_utils::tmpdir("fragmented_progress");
        let part_dir = dir.join("file.bin.parts");
        std::fs::create_dir_all(&part_dir).unwrap();

        let url = format!("http://{}/file.bin", server.addr);
        let total = Arc::new(AtomicU64::new(0));
        let total_h = total.clone();
        let parts = download_fragmented(&test_agent(), &url, &[], 1000, 4, &part_dir, &move |n| {
            total_h.fetch_add(n, Ordering::Relaxed);
        })
        .unwrap();

        server.shutdown();
        assert_eq!(parts.len(), 4);
        // Concurrent chunk deltas aggregate to the full file size.
        assert_eq!(total.load(Ordering::Relaxed), 1000);
    }

    // ─── download_apps (standalone download command) ──────────────────────────

    /// SHA-256 hex of `data`, as used in manifest `hash` fields.
    fn sha256_hex(data: &[u8]) -> String {
        let mut hasher = internal::hash::ChecksumBuilder::new().sha256().build();
        hasher.consume(data);
        hasher.finalize()
    }

    /// Write a standalone manifest file for `name` pointing at `url`.
    fn write_manifest(
        dir: &std::path::Path,
        name: &str,
        version: &str,
        url: &str,
        hash: &str,
    ) -> std::path::PathBuf {
        let path = dir.join(format!("{}.json", name));
        let json = format!(
            r#"{{"version": "{}", "url": "{}", "hash": "{}"}}"#,
            version, url, hash
        );
        std::fs::write(&path, json).unwrap();
        path
    }

    /// Files in the session cache dir, sorted by name.
    fn cache_files(session: &Session) -> Vec<std::path::PathBuf> {
        let config = session.config();
        let dir = config.cache_path();
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.is_file())
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        files
    }

    #[test]
    fn test_download_apps_local_manifest() {
        let data = b"download-apps-test-data".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_basic");
        let session = crate::test_utils::test_session(&root);
        let url = format!("http://{}/app.bin", server.addr);
        let manifest = write_manifest(&root, "testapp", "1.0", &url, &sha256_hex(&data));

        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        download_apps(&session, &[manifest.to_str().unwrap()], &opts).unwrap();
        server.shutdown();

        let files = cache_files(&session);
        assert_eq!(files.len(), 1, "cache files: {:?}", files);
        assert!(files[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("testapp#1.0#"));
        assert_eq!(std::fs::read(&files[0]).unwrap(), data);
    }

    #[test]
    fn test_download_apps_hash_mismatch_removes_file() {
        let data = b"download-apps-mismatch".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_mismatch");
        let session = crate::test_utils::test_session(&root);
        let url = format!("http://{}/app.bin", server.addr);
        // Wrong hash: the downloaded file must be removed after verification.
        let manifest = write_manifest(&root, "badapp", "1.0", &url, &"0".repeat(64));

        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        download_apps(&session, &[manifest.to_str().unwrap()], &opts).unwrap();
        server.shutdown();

        assert!(
            cache_files(&session).is_empty(),
            "corrupt cache file must be removed"
        );
    }

    #[test]
    fn test_download_apps_no_hash_check_keeps_file() {
        let data = b"download-apps-no-hash".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_no_hash");
        let session = crate::test_utils::test_session(&root);
        let url = format!("http://{}/app.bin", server.addr);
        let manifest = write_manifest(&root, "nohashapp", "1.0", &url, &"0".repeat(64));

        let opts = DownloadOptions {
            force: false,
            check_hash: false,
        };
        download_apps(&session, &[manifest.to_str().unwrap()], &opts).unwrap();
        server.shutdown();

        assert_eq!(cache_files(&session).len(), 1);
    }

    #[test]
    fn test_download_apps_rejects_bad_version() {
        let root = crate::test_utils::tmpdir("download_apps_bad_version");
        let session = crate::test_utils::test_session(&root);

        // Missing version.
        let no_version = root.join("noversion.json");
        std::fs::write(&no_version, r#"{"url": "http://127.0.0.1:1/x.bin"}"#).unwrap();
        // Unsupported character in version.
        let bad_char = root.join("badchar.json");
        std::fs::write(
            &bad_char,
            r#"{"version": "1.0!", "url": "http://127.0.0.1:1/x.bin"}"#,
        )
        .unwrap();

        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        download_apps(
            &session,
            &[no_version.to_str().unwrap(), bad_char.to_str().unwrap()],
            &opts,
        )
        .unwrap();

        assert!(cache_files(&session).is_empty());
    }

    #[test]
    fn test_download_apps_one_bad_query_does_not_abort_others() {
        let data = b"download-apps-mixed".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_mixed");
        let session = crate::test_utils::test_session(&root);
        let url = format!("http://{}/app.bin", server.addr);
        let manifest = write_manifest(&root, "goodapp", "1.0", &url, &sha256_hex(&data));

        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        download_apps(
            &session,
            &[manifest.to_str().unwrap(), r"D:\nonexistent\app.json"],
            &opts,
        )
        .unwrap();
        server.shutdown();

        assert_eq!(cache_files(&session).len(), 1);
    }

    #[test]
    fn test_download_apps_force_redownloads() {
        let data = b"download-apps-force".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_force");
        let session = crate::test_utils::test_session(&root);
        let url = format!("http://{}/app.bin", server.addr);
        let manifest = write_manifest(&root, "forceapp", "1.0", &url, &sha256_hex(&data));
        let manifest_str = manifest.to_str().unwrap().to_owned();

        let cache_hit = DownloadOptions {
            force: false,
            check_hash: true,
        };
        download_apps(&session, &[&manifest_str], &cache_hit).unwrap();
        let requests_after_first = server.ranges.lock().unwrap().len();

        // Cache hit: no new request is sent.
        download_apps(&session, &[&manifest_str], &cache_hit).unwrap();
        assert_eq!(
            server.ranges.lock().unwrap().len(),
            requests_after_first,
            "cache hit must not re-download"
        );

        // Force: the file is downloaded again.
        let force = DownloadOptions {
            force: true,
            check_hash: true,
        };
        download_apps(&session, &[&manifest_str], &force).unwrap();
        server.shutdown();

        assert!(
            server.ranges.lock().unwrap().len() > requests_after_first,
            "force must re-download"
        );
        assert_eq!(cache_files(&session).len(), 1);
    }
}
