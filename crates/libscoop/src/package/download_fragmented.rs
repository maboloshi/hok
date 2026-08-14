//! Fragmented (range-parallel) download machinery.
//!
//! Split from `package/download.rs`: chunk splitting, per-chunk retry with
//! exponential backoff, a serial fallback for exhausted ranges, and the
//! in-process TCP test server used to exercise it.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::debug;

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
pub(crate) fn download_fragmented(
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

/// In-process HTTP test server for fragmented-download tests.
///
/// Shared by the `download_fragmented` and `download_apps` test modules:
/// serves a fixed byte buffer over a single range-request connection,
/// optionally failing the first requests to exercise retry paths.
#[cfg(test)]
pub(crate) mod test_server {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use crate::internal::network::{build_agent, RequestOptions};

    /// Handle to a spawned [`spawn_range_server`].
    pub struct RangeServer {
        pub addr: std::net::SocketAddr,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
        /// `Range` header values received, in request order.
        pub ranges: Arc<Mutex<Vec<String>>>,
        /// HTTP status codes sent in response order.
        pub responses: Arc<Mutex<Vec<u16>>>,
    }

    impl RangeServer {
        /// Stop the server thread and wait for it to finish, so every
        /// received request is fully processed before assertions run.
        pub fn shutdown(&mut self) {
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
    pub fn spawn_range_server(
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

    pub fn test_agent() -> ureq::Agent {
        build_agent(&RequestOptions {
            timeout_secs: 5,
            ..RequestOptions::default()
        })
        .unwrap()
    }
}
#[cfg(test)]
mod tests {
    use super::test_server::*;
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

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
}
