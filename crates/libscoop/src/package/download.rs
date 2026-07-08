use curl::easy::{Easy, List};
use curl::multi::Multi;
use flume::Sender;
use once_cell::unsync::OnceCell;
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::Write,
    time::Duration,
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
    /// Associated libscoop session.
    session: &'a Session,

    /// Packages with intent to download.
    pub packages: &'a [&'a Package],

    /// Multi handle for curl.
    multi: Multi,

    caches: OnceCell<HashMap<String, PackageCache<'a>>>,

    /// Whether to reuse cached files.
    reuse_cache: bool,
}

/// Stores download information of a file.
struct FileDownloadInfo<'a> {
    /// Download URL.
    url: &'a str,

    /// Local cached file size.
    local_size: u64,

    /// Remote file size.
    remote_size: u64,

    /// Whether the remote file size is estimated.
    estimated: bool,
}

/// Possible cache state of a package.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CacheMaybeValid {
    /// All files are cached and valid.
    Full,

    /// Some files are cached and valid.
    Partial,

    /// No valid cache.
    None,
}

/// Local cache information of a package.
struct PackageCache<'a> {
    /// Associated package.
    package: &'a Package,

    /// Whether the cache is valid.
    valid: CacheMaybeValid,

    /// Inner details of the package cache.
    ///
    /// Since a package may have multiple files to download, the inner hashmap
    /// stores the download information of each file.
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

        if cnt == self.inner.len() {
            self.valid = CacheMaybeValid::Full;
        } else if cnt > 0 {
            self.valid = CacheMaybeValid::Partial;
        } else {
            self.valid = CacheMaybeValid::None;
        }
    }
}

impl<'a> PackageSet<'a> {
    pub fn new(
        session: &'a Session,
        packages: &'a [&Package],
        reuse_cache: bool,
    ) -> Fallible<PackageSet<'a>> {
        let mut multi = Multi::new();

        let max_conn = session.config().aria2_max_connection_per_server() as usize;
        multi.set_max_total_connections(max_conn * 2)?;
        multi.set_max_host_connections(max_conn)?;
        multi.pipelining(false, true)?;

        Ok(PackageSet {
            session,
            packages,
            multi,
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
            // if the package is upgradable, use the upgradable reference instead
            let pkg = pkg.upgradable().unwrap_or(pkg);

            let urls = pkg.download_urls();
            let filenames = pkg.download_filenames();

            let mut pacakge_cache = PackageCache {
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

                let dlinfo = FileDownloadInfo {
                    url,
                    local_size,
                    remote_size,
                    estimated: false,
                };

                pacakge_cache.inner.insert(filename.to_owned(), dlinfo);
            }

            if self.reuse_cache {
                if file_cached_count == urls.len() {
                    pacakge_cache.valid = CacheMaybeValid::Full;
                } else if file_cached_count > 0 {
                    pacakge_cache.valid = CacheMaybeValid::Partial;
                }
            }

            caches.insert(pkg.ident(), pacakge_cache);
        }

        let _ = self.caches.set(caches);
    }

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

        let mut handles = HashMap::new();
        let mut token_ctx = HashMap::new();
        let package_caches = self.caches.get_mut().unwrap();

        // Maps final file path → list of chunk temp paths (for reassembly)
        let mut chunk_file_map: HashMap<std::path::PathBuf, Vec<std::path::PathBuf>> = HashMap::new();
        // Simple non-chunked files: (tmp, final)
        let mut filepaths: Vec<(std::path::PathBuf, std::path::PathBuf)> = vec![];

        internal::fs::ensure_dir(&cache_root)?;

        // Read fragmentation settings from aria2 config (reuse existing user config)
        let fragmentation_enabled = config.aria2_enabled();
        let chunk_count = config
            .aria2_split()
            .min(config.aria2_max_connection_per_server()) as u64;
        let min_split_size = config.aria2_min_split_size();

        for (pidx, (_, cache)) in package_caches.iter().enumerate() {
            if self.reuse_cache && cache.valid == CacheMaybeValid::Full {
                continue;
            }

            let cookie = cache.package.cookie().unwrap_or_default();

            for (uidx, (filename, dlinfo)) in cache.inner.iter().enumerate() {
                if self.reuse_cache
                    && dlinfo.local_size > 0
                    && dlinfo.local_size == dlinfo.remote_size
                {
                    continue;
                }

                // Decide whether to use fragmented download
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
                    let mut part_paths = Vec::new();

                    for chunk_idx in 0..chunk_count {
                        let start = chunk_idx * chunk_size;
                        let end = if chunk_idx == chunk_count - 1 {
                            dlinfo.remote_size - 1
                        } else {
                            (chunk_idx + 1) * chunk_size - 1
                        };

                        let part_path = part_dir.join(format!("part.{}", chunk_idx));
                        part_paths.push(part_path.clone());

                        let mut easy = Easy::new();
                        easy.get(true)?;
                        easy.url(dlinfo.url)?;
                        easy.follow_location(true)?;
                        easy.useragent(user_agent)?;
                        easy.fail_on_error(true)?;
                        if let Some(proxy) = proxy {
                            easy.proxy(proxy)?;
                        }
                        set_cookie(&mut easy, &cookie)?;

                        // Set Range header for this chunk
                        let range = format!("bytes={}-{}", start, end);
                        let mut list = List::new();
                        list.append(&range)?;
                        easy.http_headers(list)?;

                        if let Some(tx) = self.session.emitter() {
                            let ident = cache.package.ident();
                            let url = dlinfo.url.to_owned();
                            let fname = filename.to_owned();
                            easy.progress(true)?;
                            easy.progress_function(move |dltotal, dlnow, _, _| {
                                progress(
                                    tx.clone(),
                                    ident.to_owned(),
                                    url.to_owned(),
                                    fname.to_owned(),
                                    dltotal,
                                    dlnow,
                                )
                            })?;
                        }

                        // Remove existing chunk file
                        let _ = std::fs::remove_file(&part_path);
                        let mut file = OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&part_path)?;
                        easy.write_function(move |data| {
                            file.write_all(data).unwrap();
                            Ok(data.len())
                        })?;

                        let mut easyhandle = self.multi.add(easy)?;
                        let token = pidx * 10000 + uidx * 100 + chunk_idx as usize;
                        let _ = easyhandle.set_token(token);
                        handles.insert(token, easyhandle);
                        token_ctx.insert(token, (cache.package.ident(), filename.to_owned()));
                    }

                    chunk_file_map.insert(path.clone(), part_paths);
                } else {
                    // Single-threaded download (original behavior)
                    let mut easy = Easy::new();
                    easy.get(true)?;
                    easy.url(dlinfo.url)?;
                    easy.follow_location(true)?;
                    easy.useragent(user_agent)?;
                    easy.fail_on_error(true)?;
                    if let Some(proxy) = proxy {
                        easy.proxy(proxy)?;
                    }
                    set_cookie(&mut easy, &cookie)?;

                    if let Some(tx) = self.session.emitter() {
                        let ident = cache.package.ident();
                        let url = dlinfo.url.to_owned();
                        let fname = filename.to_owned();
                        easy.progress(true)?;
                        easy.progress_function(move |dltotal, dlnow, _, _| {
                            progress(
                                tx.clone(),
                                ident.to_owned(),
                                url.to_owned(),
                                fname.to_owned(),
                                dltotal,
                                dlnow,
                            )
                        })?;
                    }

                    let path = cache_root.join(filename);
                    let tmp = cache_root.join(format!("{}.download", filename));

                    let _ = std::fs::remove_file(&path);
                    let _ = std::fs::remove_file(&tmp);

                    filepaths.push((tmp.clone(), path.clone()));

                    let mut file = OpenOptions::new().create(true).append(true).open(&tmp)?;
                    easy.write_function(move |data| {
                        file.write_all(data).unwrap();
                        Ok(data.len())
                    })?;

                    let mut easyhandle = self.multi.add(easy)?;
                    let token = pidx * 100 + uidx;
                    let _ = easyhandle.set_token(token);
                    handles.insert(token, easyhandle);
                    token_ctx.insert(token, (cache.package.ident(), filename.to_owned()));
                }
            }
        }

        let mut alive = true;
        while alive {
            alive = self.multi.perform()? > 0;

            let mut handle_err = None;

            self.multi.messages(|message| {
                let token = message.token().expect("failed to get token");
                let handle = handles.get_mut(&token).expect("failed to get handle");

                if let Some(Err(e)) = message.result_for(handle) {
                    handle_err = Some(e);
                }
            });

            if let Some(err) = handle_err {
                return Err(err.into());
            }

            if alive {
                self.multi.wait(&mut [], Duration::from_secs(5))?;
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
            // Remove parts directory
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

    /// Calculate download size.
    ///
    /// This function is actually a pre-download process, which will try to
    /// fetch the remote file size of each package file.
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

        let mut handles = HashMap::new();
        let mut token_ctx = HashMap::new();
        let package_caches = self.caches.get_mut().unwrap();

        for (pidx, &pkg) in self.packages.iter().enumerate() {
            // if the package is upgradable, use the upgradable reference instead
            let pkg = pkg.upgradable().unwrap_or(pkg);

            let urls = pkg.download_urls();
            let filenames = pkg.download_filenames();
            let cookie = pkg.cookie().unwrap_or_default();

            for (uidx, (url, filename)) in urls.iter().zip(filenames.iter()).enumerate() {
                let mut easy = Easy::new();
                easy.get(true)?;
                easy.url(url)?;
                easy.follow_location(true)?;
                easy.nobody(true)?;
                easy.useragent(user_agent)?;
                if let Some(proxy) = proxy {
                    easy.proxy(proxy)?;
                }
                set_cookie(&mut easy, &cookie)?;

                let mut easyhandle = self.multi.add(easy)?;
                let token = pidx * 100 + uidx;
                let _ = easyhandle.set_token(token);
                handles.insert(token, easyhandle);

                token_ctx.insert(token, (pkg.ident(), url.to_string(), filename.to_owned()));
            }
        }

        let mut total = 0;
        let mut estimated = false;

        let mut alive = true;
        while alive {
            alive = self.multi.perform()? > 0;

            let mut handle_err = None;

            self.multi.messages(|message| {
                let token = message.token().expect("failed to get token");
                let handle = handles.get_mut(&token).expect("failed to get handle");

                if let Some(handle_ret) = message.result_for(handle) {
                    match handle_ret {
                        Err(e) => handle_err = Some(e),
                        Ok(_) => {
                            let (ident, url, filename) = token_ctx.get(&token).unwrap();
                            let package_cache = package_caches.get_mut(ident).unwrap();
                            let info = package_cache
                                .inner
                                .get_mut(filename)
                                .expect("failed to get cache info");

                            if let Ok(code) = handle.response_code() {
                                let mut content_length = 0u64;
                                if code == 200 {
                                    content_length =
                                        handle.content_length_download().unwrap_or(0f64) as u64;
                                    info.remote_size = content_length;
                                    if content_length != info.local_size {
                                        total += content_length;
                                    }
                                } else {
                                    debug!("code: {}, ident: {}, url: {}", code, ident, url)
                                }

                                if content_length == 0 {
                                    info.estimated = true;
                                    estimated = true;
                                }

                                package_cache.update_valid_state();
                            } else {
                                debug!("failed to get response code for {}", url);
                            }
                        }
                    }
                }
            });

            if let Some(err) = handle_err {
                return Err(err.into());
            }

            if alive {
                self.multi.wait(&mut [], Duration::from_secs(5))?;
            }
        }

        Ok(DownloadSize { total, estimated })
    }
}

fn set_cookie(easy: &mut Easy, cookie: &[(&str, &str)]) -> Fallible<()> {
    if !cookie.is_empty() {
        let mut header_cookie = String::from("Cookie: ");
        header_cookie.push_str(
            &cookie
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join("; "),
        );
        let mut list = List::new();
        list.append(&header_cookie)?;
        easy.http_headers(list)?;
    }

    Ok(())
}

/// Progress context for package download.
#[derive(Clone, Debug)]
pub struct PackageDownloadProgressContext {
    /// Package identifier.
    pub ident: String,

    /// Download URL.
    pub url: String,

    /// Download filename.
    pub filename: String,

    /// Total bytes to download.
    pub dltotal: u64,

    /// Downloaded bytes.
    pub dlnow: u64,
}

/// Report package download progress.
fn progress(
    tx: Sender<Event>,
    ident: String,
    url: String,
    filename: String,
    dltotal: f64,
    dlnow: f64,
) -> bool {
    let ctx = PackageDownloadProgressContext {
        ident,
        url,
        filename,
        dltotal: dltotal as u64,
        dlnow: dlnow as u64,
    };

    tx.send(Event::PackageDownloadProgress(ctx)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

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
