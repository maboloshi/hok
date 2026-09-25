//! `hok download` must survive resolving and downloading a valid local
//! manifest.
//!
//! Regression test for a main-thread stack overflow (STATUS_STACK_OVERFLOW,
//! `0xC00000FD`): deserializing a manifest that carries a `hash` peaks above
//! the 1 MiB stack Windows reserves for the process's main thread (PE header
//! `SizeOfStackReserve`), so `hok download <local manifest>` died before it
//! opened a single connection. `build.rs` now reserves a bigger stack.
//!
//! Only the real binary can show this: it runs on the PE-header main thread,
//! while libscoop's own tests run on libtest threads, which get their own
//! (larger) stack.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Body served by the loopback server. Small on purpose: the fragmentation
/// threshold defaults to 20 MiB, so the transfer is a single GET.
const BODY: &[u8] = b"hok-download-stack-overflow-body";

/// SHA-256 hex of `data`, the form a manifest `hash` field wants.
fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = libscoop::hash::ChecksumBuilder::new().sha256().build();
    hasher.consume(data);
    hasher.finalize()
}

/// A throwaway Scoop root plus the environment overrides that keep the test
/// away from the real installation and the user's `config.json`.
struct Env {
    root: PathBuf,
    home: PathBuf,
    /// An empty working directory: the manifest lives here so nothing is read
    /// from (or written to) the source checkout.
    cwd: PathBuf,
}

impl Env {
    fn new(name: &str) -> Env {
        let base = std::env::temp_dir().join(format!("hok_download_stack_{name}"));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("scoop");
        let home = base.join("home");
        let cwd = base.join("cwd");
        std::fs::create_dir_all(&root).expect("create test root");
        std::fs::create_dir_all(&home).expect("create test home");
        std::fs::create_dir_all(&cwd).expect("create test cwd");
        Env { root, home, cwd }
    }

    /// Run `hok <args>` against this environment; returns the exit code and
    /// everything the process printed.
    fn run(&self, args: &[&str]) -> (i32, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_hok"))
            .args(args)
            .current_dir(&self.cwd)
            .env("SCOOP", &self.root)
            .env("SCOOP_GLOBAL", self.root.join("global"))
            .env("USERPROFILE", &self.home)
            .env("HOME", &self.home)
            .env("APPDATA", self.home.join("AppData"))
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env_remove("HOK_LOG_LEVEL")
            .output()
            .expect("run hok");
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        (out.status.code().unwrap_or(-1), text)
    }
}

/// Files directly inside `dir`, sorted by name.
fn cache_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
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

/// A minimal loopback HTTP server: answers HEAD (sizing) and GET (the body)
/// until it is dropped. The download under test is one small file, so there is
/// no range, keep-alive or chunked-encoding support to worry about.
struct Server {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn start(body: Vec<u8>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => serve(stream, &body),
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        // The listening socket is gone: nothing left to serve.
                        Err(_) => break,
                    }
                }
            })
        };
        Server {
            addr,
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Answer one request: `Content-Length` plus the body for a GET, headers only
/// for a HEAD.
fn serve(mut stream: TcpStream, body: &[u8]) {
    // The accepted socket inherits the listener's non-blocking mode (Windows
    // included), but this handler is synchronous: without switching back, a
    // request that has not arrived yet reads as `WouldBlock` and the connection
    // is closed without an answer.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));

    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    // Drain the headers so the peer is not left mid-write.
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) if line == "\r\n" || line == "\n" => break,
            Ok(_) => {}
        }
    }

    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    if !request_line.starts_with("HEAD") {
        let _ = stream.write_all(body);
    }
    let _ = stream.flush();
}

/// A local manifest with a `hash` used to overflow the main thread's 1 MiB
/// stack before `hok download` could do any work; downloading it must now
/// succeed end to end (and a second run must take the cache path).
#[test]
fn download_of_a_local_manifest_does_not_overflow_the_stack() {
    let env = Env::new("local_manifest");
    let server = Server::start(BODY.to_vec());
    let url = format!("http://{}/app.bin", server.addr);

    let manifest = env.cwd.join("stackapp.json");
    std::fs::write(
        &manifest,
        format!(
            r#"{{"version": "1.0", "url": "{}", "hash": "{}"}}"#,
            url,
            sha256_hex(BODY)
        ),
    )
    .expect("write manifest");
    let manifest = manifest.to_str().unwrap();

    let (code, text) = env.run(&["download", manifest]);
    assert!(
        !text.contains("overflowed its stack"),
        "the main thread overflowed: {text}"
    );
    assert_eq!(code, 0, "{text}");

    let cached = cache_files(&env.root.join("cache"));
    assert_eq!(
        cached.len(),
        1,
        "the downloaded file must land in the cache: {cached:?}\n{text}"
    );
    assert_eq!(std::fs::read(&cached[0]).unwrap(), BODY);

    // The second run is a cache hit: same manifest resolution, tail
    // verification instead of a transfer — and the same clean exit.
    let (code, text) = env.run(&["download", manifest]);
    assert!(
        !text.contains("overflowed its stack"),
        "the main thread overflowed: {text}"
    );
    assert_eq!(code, 0, "{text}");
    assert_eq!(
        cache_files(&env.root.join("cache")).len(),
        1,
        "the cache file must survive the cache-hit run"
    );
}
