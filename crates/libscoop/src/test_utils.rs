//! Test utilities shared across libscoop test modules.
//!
//! Only compiled under `#[cfg(test)]`.

/// Create a temporary directory for testing, automatically cleaned up.
pub fn tmpdir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("hok_test_{}", name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write a minimal-but-parseable PE image with the given subsystem into
/// `path` (creating parent dirs). Used to exercise the shim-variant
/// selection against GUI (2) vs console (3) targets.
pub fn write_fake_pe(path: &std::path::Path, subsystem: u16) {
    use std::io::Write;
    let mut data = vec![0u8; 0x100];
    data[0] = b'M';
    data[1] = b'Z';
    // e_lfanew = 0x80
    data[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    // PE signature
    data[0x80..0x84].copy_from_slice(b"PE\0\0");
    // Subsystem at PE + 0x5C = 0x80 + 0x5C = 0xDC
    data[0xDC..0xDE].copy_from_slice(&subsystem.to_le_bytes());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(&data).unwrap();
}

/// Create a [`Session`][1] rooted at `root`: writes a minimal `hok.json`
/// (with `root_path` and `cache_path` pointing at `root`) and loads it via
/// `Session::new_with`.
///
/// `cache_path` is pinned to `<root>/cache`: without it, the config falls
/// back to the global default cache dir (`~/scoop/cache`), which tests
/// running in parallel would clobber (same download filenames → same files).
///
/// [1]: crate::Session
pub fn test_session(root: &std::path::Path) -> crate::Session {
    let config_path = root.join("hok.json");
    let root_escaped = root.to_string_lossy().replace('\\', "\\\\");
    let cache_escaped = root.join("cache").to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        &config_path,
        format!(
            r#"{{"root_path": "{}", "cache_path": "{}"}}"#,
            root_escaped, cache_escaped
        ),
    )
    .unwrap();
    crate::Session::new_with(&config_path).unwrap()
}

/// Write a Scoop manifest into `<root>/buckets/<bucket>/bucket/<name>.json`,
/// making it visible to `query_synced` / `query_installed`.
pub fn write_bucket_manifest(root: &std::path::Path, bucket: &str, name: &str, json: &str) {
    let dir = root.join("buckets").join(bucket).join("bucket");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{}.json", name)), json).unwrap();
}

/// Mark `name` as installed under `root` by creating
/// `<root>/apps/<name>/current/{manifest.json, install.json}`.
pub fn mark_installed(
    root: &std::path::Path,
    name: &str,
    bucket: &str,
    manifest_json: &str,
    held: bool,
) {
    let current = root.join("apps").join(name).join("current");
    std::fs::create_dir_all(&current).unwrap();
    std::fs::write(current.join("manifest.json"), manifest_json).unwrap();
    let install_info = format!(
        r#"{{"architecture": "64bit", "bucket": "{}", "hold": {}}}"#,
        bucket, held
    );
    std::fs::write(current.join("install.json"), install_info).unwrap();
}
// ─── In-memory network fake ────────────────────────────────────────────────

use crate::error::{Error, Fallible};
use crate::internal::git::Git;
use crate::internal::network::{HeadResult, Network, RangeError, RangeResponse, RequestOptions};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};

/// An in-memory [`Network`] fake for offline tests.
///
/// Serves a per-URL body for `download` / `download_stream` / `head`, and a
/// range server for `range_get` that mirrors the knobs of the legacy
/// `spawn_range_server` (fail-first, ignore-range, truncate-first,
/// range-conditioned failure) so the fragmented-download logic runs entirely
/// off the network. Records received ranges and response statuses for
/// assertions.
#[derive(Debug)]
pub struct FakeNetwork {
    /// URL → full body.
    bodies: Mutex<HashMap<String, Vec<u8>>>,
    /// Fail the first `fail_first` range requests with `fail_status`.
    fail_first: AtomicU32,
    fail_status: AtomicU16,
    /// Always answer 200 with the full body (server ignores `Range`).
    ignore_range: AtomicBool,
    /// Truncate the first 206 response to half the requested slice.
    truncate_first: AtomicBool,
    /// Fail every request whose range overlaps `[s, e]` for the first `n`.
    fail_range: Mutex<Option<(u64, u64, u32)>>,
    count: AtomicU32,
    fail_range_count: AtomicU32,
    /// `Range` header values received by `range_get`, in request order.
    pub ranges: Mutex<Vec<String>>,
    /// HTTP status codes sent in response order.
    pub responses: Mutex<Vec<u16>>,
}

impl FakeNetwork {
    /// Create an empty fake.
    pub fn new() -> Self {
        Self {
            bodies: Mutex::new(HashMap::new()),
            fail_first: AtomicU32::new(0),
            fail_status: AtomicU16::new(500),
            ignore_range: AtomicBool::new(false),
            truncate_first: AtomicBool::new(false),
            fail_range: Mutex::new(None),
            count: AtomicU32::new(0),
            fail_range_count: AtomicU32::new(0),
            ranges: Mutex::new(Vec::new()),
            responses: Mutex::new(Vec::new()),
        }
    }

    /// Serve `body` for `url`.
    pub fn serve(&self, url: &str, body: Vec<u8>) {
        self.bodies.lock().unwrap().insert(url.to_owned(), body);
    }

    /// Configure range-server behaviour mirroring the legacy server.
    pub fn range_behavior(
        &self,
        fail_first: u32,
        fail_status: u16,
        ignore_range: bool,
        truncate_first: bool,
        fail_range: Option<(u64, u64, u32)>,
    ) {
        self.fail_first.store(fail_first, Ordering::SeqCst);
        self.fail_status.store(fail_status, Ordering::SeqCst);
        self.ignore_range.store(ignore_range, Ordering::SeqCst);
        self.truncate_first.store(truncate_first, Ordering::SeqCst);
        *self.fail_range.lock().unwrap() = fail_range;
    }

    fn body(&self, url: &str) -> Option<Vec<u8>> {
        self.bodies.lock().unwrap().get(url).cloned()
    }
}

impl Default for FakeNetwork {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Network for FakeNetwork {
    async fn head(&self, url: &str, _opts: &RequestOptions) -> HeadResult {
        self.responses.lock().unwrap().push(200);
        let body = self.body(url);
        HeadResult {
            url: url.to_owned(),
            status_code: 200,
            error: None,
            content_length: body.map(|b| b.len() as u64),
        }
    }

    async fn download(&self, url: &str, _opts: &RequestOptions) -> Result<Vec<u8>, String> {
        self.body(url)
            .ok_or_else(|| format!("no canned body for {url}"))
    }

    async fn range_get(
        &self,
        url: &str,
        start: u64,
        end: u64,
        _opts: &RequestOptions,
    ) -> Result<RangeResponse, RangeError> {
        self.ranges
            .lock()
            .unwrap()
            .push(format!("bytes={start}-{end}"));
        let n = self.count.fetch_add(1, Ordering::SeqCst) + 1;

        if n <= self.fail_first.load(Ordering::SeqCst) {
            let status = self.fail_status.load(Ordering::SeqCst);
            self.responses.lock().unwrap().push(status);
            if (400..500).contains(&status) && status != 408 && status != 429 {
                return Err(RangeError::Final(format!("HTTP {status} for {url}")));
            }
            return Err(RangeError::Transient(format!("HTTP {status}")));
        }

        if self.ignore_range.load(Ordering::SeqCst) {
            self.responses.lock().unwrap().push(200);
            return Err(RangeError::Final(
                "expected 206 Partial Content, got 200 (server ignored Range request)".to_string(),
            ));
        }

        let body = self
            .body(url)
            .ok_or_else(|| RangeError::Transient(format!("no canned body for {url}")))?;
        let slice = &body[start as usize..=end as usize];

        if let Some((fs, fe, attempts)) = *self.fail_range.lock().unwrap()
            && start <= fe
            && fs <= end
        {
            let m = self.fail_range_count.fetch_add(1, Ordering::SeqCst) + 1;
            if m <= attempts {
                let status = self.fail_status.load(Ordering::SeqCst);
                self.responses.lock().unwrap().push(status);
                return Err(RangeError::Transient(format!("HTTP {status}")));
            }
        }

        if self.truncate_first.load(Ordering::SeqCst) && n == 1 {
            let half = slice.len() / 2;
            self.responses.lock().unwrap().push(206);
            return Ok(RangeResponse {
                status: 206,
                content_range_start: Some(start),
                bytes: slice[..half].to_vec(),
            });
        }

        self.responses.lock().unwrap().push(206);
        Ok(RangeResponse {
            status: 206,
            content_range_start: Some(start),
            bytes: slice.to_vec(),
        })
    }

    async fn download_stream(
        &self,
        url: &str,
        opts: &RequestOptions,
        dest: &Path,
        on_progress: &mut (dyn FnMut(u64) + Send),
    ) -> Result<(), String> {
        let body = self
            .body(url)
            .ok_or_else(|| format!("no canned body for {url}"))?;
        let resume = opts.range_start as usize;
        self.ranges.lock().unwrap().push(if resume > 0 {
            format!("bytes={resume}-")
        } else {
            String::new()
        });
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Mirror the real client: append the tail when resuming, otherwise
        // (re)write the whole body.
        let slice = if resume < body.len() {
            &body[resume..]
        } else {
            &[]
        };
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dest)
            .map_err(|e| e.to_string())?;
        use std::io::Write as _;
        file.write_all(slice).map_err(|e| e.to_string())?;
        let total = dest.metadata().map(|m| m.len()).unwrap_or(0);
        on_progress(total);
        Ok(())
    }
}

// ─── In-memory git fake ────────────────────────────────────────────────────

/// An in-memory [`Git`] fake for offline tests.
///
/// Records `clone_repo` / `pull` calls instead of touching real repositories.
/// `pull` is a no-op success unless [`FakeGit::fail_pull`] is set.
#[derive(Debug, Default)]
pub struct FakeGit {
    /// Record of `(remote_url, path)` clone calls.
    pub clones: Mutex<Vec<(String, std::path::PathBuf)>>,
    /// Record of pulled repository paths.
    pub pulls: Mutex<Vec<std::path::PathBuf>>,
    /// When set, `pull` returns an error.
    pub fail_pull: AtomicBool,
}

#[async_trait::async_trait]
impl Git for FakeGit {
    async fn clone_repo(
        &self,
        remote_url: &str,
        path: &Path,
        _proxy: Option<&str>,
    ) -> Fallible<()> {
        self.clones
            .lock()
            .unwrap()
            .push((remote_url.to_owned(), path.to_owned()));
        Ok(())
    }

    async fn pull(&self, path: &Path, _proxy: Option<&str>) -> Fallible<()> {
        if self.fail_pull.load(Ordering::SeqCst) {
            return Err(Error::Custom("fake pull failed".to_string()));
        }
        self.pulls.lock().unwrap().push(path.to_owned());
        Ok(())
    }

    fn remote_url_of(&self, _repo_path: &Path, remote: &str) -> Fallible<Option<String>> {
        Ok(Some(format!("fake://{remote}")))
    }

    fn head_commit_time(&self, _path: &Path) -> Option<i64> {
        Some(0)
    }
}
