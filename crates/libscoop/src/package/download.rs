//! Concurrent package download with `ureq` (pure Rust).
//!
//! Replaced `curl` (libcurl bindings) to avoid static C compilation overhead.
//! Fragmented downloads use `std::thread::scope` instead of `curl::multi::Multi`.

use std::cell::OnceCell;
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
};
use tracing::debug;

use crate::constant::DEFAULT_USER_AGENT;
use crate::{error::Fallible, internal, Event, Session};

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
                    && chunk_count > 1;

                let result = (|| -> Fallible<()> {
                    if use_fragments {
                        let path = cache_root.join(filename);
                        let part_dir = cache_root.join(format!("{}.parts", filename));
                        internal::fs::ensure_dir(&part_dir)?;

                        let chunk_size = dlinfo.remote_size / chunk_count;
                        let mut part_paths: Vec<PathBuf> = Vec::new();

                        // Launch threads for parallel chunk downloads
                        let url_str = dlinfo.url.to_owned();
                        let cookie_clone = cookie.clone();

                        std::thread::scope(|scope| {
                            for chunk_idx in 0..chunk_count {
                                let start = chunk_idx * chunk_size;
                                let end = if chunk_idx == chunk_count - 1 {
                                    dlinfo.remote_size - 1
                                } else {
                                    (chunk_idx + 1) * chunk_size - 1
                                };

                                let part_path = part_dir.join(format!("part.{}", chunk_idx));
                                part_paths.push(part_path.clone());

                                // Don't remove — allow resume if part exists

                                let part_path = part_path.clone();
                                let url = url_str.clone();
                                let ck = cookie_clone.clone();

                                scope.spawn(move || {
                                    if let Err(e) = download_range(
                                        agent, &url, start, end, &part_path, &ck, proxy,
                                    ) {
                                        debug!("chunk download failed: {}", e);
                                    }
                                });
                            }
                        });

                        // Check all parts downloaded OK (respect resume — allow complete parts)
                        for (idx, part) in part_paths.iter().enumerate() {
                            let expected = if idx as u64 == chunk_count - 1 {
                                dlinfo.remote_size - (chunk_count - 1) * chunk_size
                            } else {
                                chunk_size
                            };
                            let actual = part.metadata().map(|m| m.len()).unwrap_or(0);
                            if actual == 0 {
                                return Err(crate::error::Error::Custom(format!(
                                    "failed to download chunk: {}",
                                    part.display()
                                )));
                            }
                            if actual < expected {
                                debug!(
                                    "chunk {} is incomplete ({} < {}), will retry",
                                    idx, actual, expected
                                );
                                return Err(crate::error::Error::Custom(format!(
                                    "incomplete chunk {}: {} < {}",
                                    idx, actual, expected
                                )));
                            }
                        }

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
                    eprintln!("failed to download '{}': {}", ident, err);
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
                    eprintln!("failed to reassemble '{}': {}", ident, e);
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
                    eprintln!("failed to move download of '{}': {}", ident, e);
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
fn download_range(
    agent: &ureq::Agent,
    url: &str,
    start: u64,
    end: u64,
    dest: &std::path::Path,
    cookie: &[(&str, &str)],
    proxy: Option<&str>,
) -> Result<(), String> {
    let _ = proxy; // proxy already baked into agent

    let mut attempt = 1u32;
    loop {
        match try_download_range(agent, url, start, end, dest, cookie) {
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
    let written = std::io::copy(&mut reader, &mut file)
        .map_err(|e| ChunkError::Transient(format!("read/write failed: {}", e)))?;

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

// ─── Old curl implementation (kept for reference) ──────────────────────────
// (see git history for the full curl-based download.rs)

#[cfg(test)]
mod tests {
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
}
