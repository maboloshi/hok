//! Concurrent package download with `ureq` (pure Rust).
//!
//! Replaced `curl` (libcurl bindings) to avoid static C compilation overhead.
//! Fragmented downloads use `std::thread::scope` instead of `curl::multi::Multi`.

use once_cell::unsync::OnceCell;
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
        for (_, cache) in self.inner.iter() {
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
        })
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
    pub fn download(&mut self) -> Fallible<()> {
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

        let mut chunk_file_map: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        let mut filepaths: Vec<(PathBuf, PathBuf)> = vec![];

        internal::fs::ensure_dir(&cache_root)?;

        // Read fragmentation settings from aria2 config (reuse existing user config)
        let fragmentation_enabled = config.aria2_enabled();
        let chunk_count = config
            .aria2_split()
            .min(config.aria2_max_connection_per_server()) as u64;
        let min_split_size = config.aria2_min_split_size();

        // Build agent once (shared for all downloads)
        let agent = build_agent(proxy, user_agent, 120);
        let agent = &agent;

        for (_, cache) in package_caches.iter() {
            if self.reuse_cache && cache.valid == CacheMaybeValid::Full {
                continue;
            }

            let cookie = cache.package.cookie().unwrap_or_default();

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

                            let _ = std::fs::remove_file(&part_path);

                            let part_path = part_path.clone();
                            let url = url_str.clone();
                            let ck = cookie_clone.clone();

                            scope.spawn(move || {
                                if let Err(e) = download_range(
                                    &agent, &url, start, end, &part_path, &ck, proxy,
                                ) {
                                    debug!("chunk download failed: {}", e);
                                }
                            });
                        }
                    });

                    // Check all parts downloaded OK
                    for part in &part_paths {
                        if !part.exists() || part.metadata().map(|m| m.len()).unwrap_or(0) == 0 {
                            return Err(crate::error::Error::Custom(format!(
                                "failed to download chunk: {}",
                                part.display()
                            )));
                        }
                    }

                    chunk_file_map.insert(path, part_paths);
                } else {
                    // Single download
                    let path = cache_root.join(filename);
                    let tmp = cache_root.join(format!("{}.download", filename));
                    let _ = std::fs::remove_file(&path);
                    let _ = std::fs::remove_file(&tmp);

                    let emitter = self.session.emitter();
                    let ident = cache.package.ident();
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

                    let mut file = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&tmp)?;

                    let mut reader = resp.into_body().into_reader();
                    let mut buf = [0u8; 32768];
                    let mut dlnow = 0u64;

                    loop {
                        let n = reader.read(&mut buf).map_err(|e| {
                            crate::error::Error::Custom(e.to_string())
                        })?;
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

                    filepaths.push((tmp, path));
                }
            }
        }

        // Reassemble fragmented files
        for (final_path, part_paths) in chunk_file_map.iter() {
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
        }

        // Rename simple downloads
        for (tmp, path) in filepaths.iter() {
            std::fs::rename(tmp, path)?;
        }

        Ok(())
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
        let agent = build_agent(proxy, user_agent, 30);

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

                // HEAD request via ureq
                let mut req = agent.head(*url);
                if !cookie.is_empty() {
                    let cookie_val = cookie
                        .iter()
                        .map(|(k, v)| format!("{}={}", k, v))
                        .collect::<Vec<_>>()
                        .join("; ");
                    req = req.header("Cookie", &cookie_val);
                }

                let code = match req.call() {
                    Ok(resp) => resp.status().as_u16(),
                    Err(e) => {
                        debug!("HEAD failed for {}: {}", url, e);
                        info.estimated = true;
                        estimated = true;
                        package_cache.update_valid_state();
                        continue;
                    }
                };

                if code == 200 {
                    info.remote_size = get_content_length_from_agent(&agent, url, &cookie)
                        .unwrap_or(0);
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

// ─── Helpers ────────────────────────────────────────────────────────────────

fn build_agent(proxy: Option<&str>, _user_agent: &str, timeout_secs: u64) -> ureq::Agent {
    let mut cfg = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(timeout_secs)));
    if let Some(proxy_url) = proxy {
        if let Ok(p) = ureq::Proxy::new(proxy_url) {
            cfg = cfg.proxy(Some(p));
        }
    }
    cfg.build().new_agent()
}

fn get_content_length_from_agent(
    agent: &ureq::Agent, url: &str, cookie: &[(&str, &str)],
) -> Option<u64> {
    let mut req = agent.head(url);
    if !cookie.is_empty() {
        let cookie_val = cookie.iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("; ");
        req = req.header("Cookie", &cookie_val);
    }
    let resp = req.call().ok()?;
    resp.headers()
        .get("Content-Length")?
        .to_str()
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
}

fn download_range(
    agent: &ureq::Agent, url: &str, start: u64, end: u64, dest: &std::path::Path,
    cookie: &[(&str, &str)], proxy: Option<&str>,
) -> Result<(), String> {
    let _ = proxy; // proxy already baked into agent
    let range = format!("bytes={}-{}", start, end);
    let mut req = agent.get(url).header("Range", &range);
    if !cookie.is_empty() {
        let cookie_val = cookie.iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("; ");
        req = req.header("Cookie", &cookie_val);
    }

    let resp = req.call().map_err(|e| e.to_string())?;
    let mut file = File::create(dest).map_err(|e| e.to_string())?;
    let mut reader = resp.into_body().into_reader();
    std::io::copy(&mut reader, &mut file).map_err(|e| e.to_string())?;
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
        let size = 100u64;
        let chunks = 5u64;
        let chunk_size = size / chunks;
        let mut covered = vec![false; size as usize];
        for i in 0..chunks {
            let start = i * chunk_size;
            let end = if i == chunks - 1 { size - 1 } else { (i + 1) * chunk_size - 1 };
            for b in start..=end {
                covered[b as usize] = true;
            }
        }
        assert!(covered.iter().all(|&c| c));
    }

    #[test]
    fn test_chunk_remainder() {
        let size = 10u64;
        let chunks = 3u64;
        let chunk_size = size / chunks;
        let mut covered = vec![false; size as usize];
        for i in 0..chunks {
            let start = i * chunk_size;
            let end = if i == chunks - 1 { size - 1 } else { (i + 1) * chunk_size - 1 };
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
