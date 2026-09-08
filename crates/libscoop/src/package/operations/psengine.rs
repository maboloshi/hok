//! A reusable persistent PowerShell engine for package scripts.
//!
//! Each package script stage normally spawns a fresh pwsh (~0.7 s cold start).
//! This module reuses one long-lived pwsh worker (server side in
//! asset_scripts/engine_server.ps1) across a package's script stages (and, in
//! the install path, across packages in one run) to amortise that cost.
//!
//! # Protocol
//!
//! Each [PsEngine::run_file] call writes one frame line to the worker:
//!
//!   RUN <json>   # {t: token, p: script path, env: {NAME: value, ...}, f: out file}
//!
//! The worker applies env, runs the script at p, captures its merged output
//! into the file at f, then answers on stdout with DONE_<token>:<exitcode>.
//! Output is captured to a per-submission file (not this process's stdout)
//! so a shared worker can multiplex submissions and a chatty script can never
//! fill a pipe and deadlock it.
//!
//! # Lifecycle
//!
//! - [PsEngine::new] spawns the worker and keeps it alive across calls.
//! - [PsEngine::run_file_captured_cancelable] runs one script file, blocking
//!   until its DONE line arrives, then reads the capture file (returned as
//!   output lines) and removes it. It observes a cooperative cancellation
//!   token *while* the submission runs: a script runs in-process inside the
//!   worker, so the only way to interrupt it mid-run is to kill the worker
//!   itself. When the token fires, the worker is killed (terminating the
//!   in-flight script), the outcome is flagged cancelled, and the engine
//!   transparently respawns on the next call.
//! - A script that calls exit N terminates the whole worker; the exit code is
//!   surfaced and the engine transparently respawns on the next call.
//!
//! Not thread-safe: one submitter at a time. In an async context, callers run
//! [PsEngine::run_file_captured_cancelable] on a blocking worker and await it.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Server script executed by the worker (see module docs).
const SERVER_PS1: &str = include_str!("../../../../../asset_scripts/engine_server.ps1");

static SERVER_SEQ: AtomicU64 = AtomicU64::new(0);

/// Globally-unique per-submission counter, so capture file names cannot
/// collide across engines or parallel test threads in one process.
static CAPTURE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Outcome of one [PsEngine] submission.
pub struct PsRunOutcome {
    /// The script's exit code (worker-side `$LASTEXITCODE` / thrown-error 1).
    pub code: i32,
    /// Merged script output lines (all streams), read back from the capture file.
    pub output: Vec<String>,
    /// `true` when the run was cut short by cancellation (worker killed).
    pub cancelled: bool,
}

/// A reusable, persistent PowerShell worker.
pub struct PsEngine {
    /// Running pwsh/powershell process running [SERVER_PS1].
    child: Child,
    /// Sink for submission frames (one RUN <json> line per call).
    stdin: ChildStdin,
    /// Worker stdout: the DONE_<token>:<code> completion line per submission.
    /// `Option` so a submission's completion watcher can temporarily own the
    /// reader while the engine observes cancellation on the same thread.
    stdout: Option<BufReader<ChildStdout>>,
    /// Temp path holding the server script; removed on drop.
    server_path: PathBuf,
    /// Per-engine unique token counter (also names the capture file).
    next_token: u64,
}

impl PsEngine {
    /// Spawn a fresh worker running [SERVER_PS1].
    pub fn new() -> io::Result<PsEngine> {
        // Materialize the embedded server script once on disk (kept for the
        // worker's lifetime; reused verbatim across respawns).
        let seq = SERVER_SEQ.fetch_add(1, Ordering::Relaxed);
        let server_path =
            std::env::temp_dir().join(format!("hok-psengine-{}-{}.ps1", std::process::id(), seq));
        std::fs::write(&server_path, SERVER_PS1)?;
        Self::spawn_worker(&server_path)
    }

    /// Spawn the worker process and take ownership of its pipes.
    fn spawn_worker(server_path: &Path) -> io::Result<PsEngine> {
        let mut cmd = crate::internal::os::ps_command();
        cmd.arg("-File")
            .arg(server_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Server output is one DONE line per submission; script output
            // goes to capture files, so stderr is never read and can be null.
            .stderr(Stdio::null());
        let mut child = cmd.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or(io::Error::other("engine: missing child stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(io::Error::other("engine: missing child stdout"))?;
        Ok(PsEngine {
            child,
            stdin,
            stdout: Some(BufReader::new(stdout)),
            server_path: server_path.to_path_buf(),
            next_token: 1,
        })
    }

    /// Respawn after an unexpected exit (e.g. a script called exit N).
    fn respawn(&mut self) -> io::Result<()> {
        let fresh = Self::spawn_worker(&self.server_path)?;
        *self = fresh;
        Ok(())
    }

    /// Reuse the live worker; respawn it if the previous run killed it.
    fn ensure_alive(&mut self) -> io::Result<()> {
        if self.child.try_wait()?.is_some() {
            self.respawn()?;
        }
        Ok(())
    }

    /// Run a script file in the engine with the given process environment,
    /// returning its exit code (dropping captured output). Test helper; the
    /// production path uses the cancellation-aware variant.
    #[cfg(test)]
    pub fn run_file(&mut self, path: &Path, env: &HashMap<String, String>) -> io::Result<i32> {
        self.run_file_captured(path, env).map(|(code, _)| code)
    }

    /// Run a script file in the engine, returning its exit code and captured
    /// output lines (all streams merged). Test helper; blocks until the
    /// submission finishes.
    #[cfg(test)]
    pub fn run_file_captured(
        &mut self,
        path: &Path,
        env: &HashMap<String, String>,
    ) -> io::Result<(i32, Vec<String>)> {
        self.run_submission(path, env, None)
            .map(|o| (o.code, o.output))
    }

    /// Run a script file in the engine while observing a cooperative
    /// cancellation token. Blocks until the submission finishes *or* the token
    /// fires. When the token fires mid-run, the worker is killed — terminating
    /// the in-flight script (scripts run in-process inside the worker, so this
    /// is the only way to interrupt them) — and the returned outcome is flagged
    /// [`PsRunOutcome::cancelled`]. The engine respawns on the next call.
    pub fn run_file_captured_cancelable(
        &mut self,
        path: &Path,
        env: &HashMap<String, String>,
        token: &CancellationToken,
    ) -> io::Result<PsRunOutcome> {
        self.run_submission(path, env, Some(token))
    }

    /// Shared submission core. Writes one RUN frame, waits for the matching
    /// DONE line (optionally racing it against a cancellation token), reads
    /// back the capture file, and restores the stdout reader.
    fn run_submission(
        &mut self,
        path: &Path,
        env: &HashMap<String, String>,
        token: Option<&CancellationToken>,
    ) -> io::Result<PsRunOutcome> {
        self.ensure_alive()?;
        let stoken = self.next_token;
        self.next_token = self.next_token.wrapping_add(1);

        // Per-submission capture file for this script's merged output.
        let cseq = CAPTURE_SEQ.fetch_add(1, Ordering::Relaxed);
        let capture = std::env::temp_dir().join(format!(
            "hok-psengine-out-{}-{}.txt",
            std::process::id(),
            cseq
        ));
        let frame = serde_json::json!({
            "t": stoken.to_string(),
            "p": path.to_string_lossy(),
            "env": env,
            "f": capture.to_string_lossy(),
        });
        let frame = format!("RUN {frame}\n");
        self.stdin.write_all(frame.as_bytes())?;
        self.stdin.flush()?;

        let prefix = format!("DONE_{stoken}:");

        // Wait for the completion line. Two shapes:
        //  - no token: block on a plain synchronous read (cheap, no threads);
        //  - token:     hand the reader to a watcher thread and poll the token
        //    from here, killing the worker (and thus the in-flight script)
        //    when it fires.
        if token.is_none() {
            let reader = self.stdout.as_mut().expect("engine stdout present");
            let mut line = String::new();
            let code = match reader.read_line(&mut line) {
                // EOF: the worker died (e.g. a script called exit N). Surface
                // its exit status; the next call respawns.
                Ok(0) => self.child.wait()?.code().unwrap_or(-1),
                Ok(_) => parse_done(&prefix, &line),
                Err(e) => return Err(e),
            };
            return self.finish_submission(&capture, code, false);
        }

        // ── cancelable path: watcher thread owns the stdout reader ──
        let mut reader = self.stdout.take().expect("engine stdout present");
        let (tx, rx) = mpsc::channel();
        let prefix_c = prefix.clone();
        let watcher = std::thread::spawn(move || {
            let mut line = String::new();
            let parsed = match reader.read_line(&mut line) {
                Ok(0) => None, // EOF (worker died or was killed)
                Ok(_) => Some(parse_done(&prefix_c, &line)),
                Err(_) => None,
            };
            // Hand the reader back for the next submission.
            let _ = tx.send((parsed, reader));
        });

        let mut cancelled = false;
        let parsed = loop {
            if let Some(tok) = token
                && tok.is_cancelled()
            {
                cancelled = true;
                let _ = self.child.kill();
                break None;
            }
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok((parsed, reader)) => {
                    self.stdout = Some(reader);
                    break parsed;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                // Watcher ended without sending (should not happen); treat as
                // EOF so the engine stays usable via respawn.
                Err(_) => break None,
            }
        };

        // If the token fired, the watcher sees EOF right after the kill; make
        // sure it returned the reader so the engine stays consistent.
        if cancelled && self.stdout.is_none() {
            let _ = watcher.join();
            if let Ok((_parsed, reader)) = rx.recv() {
                self.stdout = Some(reader);
            }
        }

        let code = match parsed {
            Some(c) => c,
            // EOF: worker died (exit N / killed). Surface its exit status.
            _ => self.child.wait()?.code().unwrap_or(-1),
        };
        self.finish_submission(&capture, code, cancelled)
    }

    /// Read the submission's capture file into output lines, drop the file,
    /// and assemble the outcome.
    fn finish_submission(
        &self,
        capture: &Path,
        code: i32,
        cancelled: bool,
    ) -> io::Result<PsRunOutcome> {
        let output = std::fs::read_to_string(capture)
            .map(|text| {
                text.lines()
                    .map(str::trim_end)
                    .filter(|l| !l.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let _ = std::fs::remove_file(capture);
        Ok(PsRunOutcome {
            code,
            output,
            cancelled,
        })
    }
}

/// Parse a worker DONE line of the form `DONE_<token>:<code>` into its exit
/// code; falls back to 1 on a malformed line.
fn parse_done(prefix: &str, line: &str) -> i32 {
    line.trim_end_matches(['\r', '\n'])
        .strip_prefix(prefix)
        .and_then(|rest| rest.trim().parse::<i32>().ok())
        .unwrap_or(1)
}

impl Drop for PsEngine {
    fn drop(&mut self) {
        // Politely stop the worker, then reap it; kill only if it lingers.
        let _ = writeln!(self.stdin, "QUIT");
        let _ = self.stdin.flush();
        let _ = self.child.wait();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_file(&self.server_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    fn env(kv: &[(&str, &str)]) -> HashMap<String, String> {
        kv.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn shell_available() -> bool {
        crate::internal::os::is_pwsh_available()
            || std::process::Command::new("powershell.exe")
                .arg("-NoProfile")
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
    }

    #[test]
    fn engine_runs_script_and_forwards_env() {
        if !shell_available() {
            return;
        }
        let dir = crate::test_utils::tmpdir("psengine_env");
        let marker = dir.join("out.txt");
        let script = write_script(
            &dir,
            "s.ps1",
            &format!(
                "[System.IO.File]::WriteAllText('{}', $env:HOK_ENG_TEST)",
                marker.to_string_lossy()
            ),
        );
        let mut engine = PsEngine::new().unwrap();
        let code = engine
            .run_file(&script, &env(&[("HOK_ENG_TEST", "v1")]))
            .unwrap();
        assert_eq!(code, 0);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "v1");
        drop(engine);
        let mut e2 = PsEngine::new().unwrap();
        assert_eq!(
            e2.run_file(&script, &env(&[("HOK_ENG_TEST", "v2")]))
                .unwrap(),
            0
        );
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "v2");
    }

    #[test]
    fn engine_reuses_one_process_across_submissions() {
        if !shell_available() {
            return;
        }
        let dir = crate::test_utils::tmpdir("psengine_reuse");
        let marker = dir.join("count.txt");
        let script = write_script(
            &dir,
            "s.ps1",
            &format!(
                "Add-Content -LiteralPath '{}' -Value x",
                marker.to_string_lossy()
            ),
        );
        let mut engine = PsEngine::new().unwrap();
        let pid1 = engine.child.id();
        for _ in 0..4 {
            assert_eq!(engine.run_file(&script, &HashMap::new()).unwrap(), 0);
            assert!(engine.child.try_wait().unwrap().is_none());
            assert_eq!(engine.child.id(), pid1);
        }
        let lines = std::fs::read_to_string(&marker).unwrap();
        assert_eq!(lines.lines().count(), 4);
    }

    #[test]
    fn thrown_error_returns_code_and_keeps_engine_alive() {
        if !shell_available() {
            return;
        }
        let dir = crate::test_utils::tmpdir("psengine_throw");
        let ok = write_script(&dir, "ok.ps1", "");
        let bad = write_script(&dir, "bad.ps1", "throw 'boom'");
        let mut engine = PsEngine::new().unwrap();
        assert_eq!(engine.run_file(&bad, &HashMap::new()).unwrap(), 1);
        assert!(engine.child.try_wait().unwrap().is_none());
        assert_eq!(engine.run_file(&ok, &HashMap::new()).unwrap(), 0);
        assert!(engine.child.try_wait().unwrap().is_none());
    }

    #[test]
    fn exit_code_surfaces_and_state_does_not_leak() {
        if !shell_available() {
            return;
        }
        let dir = crate::test_utils::tmpdir("psengine_exit");
        let exit7 = write_script(&dir, "exit7.ps1", "exit 7");
        let ok = write_script(&dir, "ok.ps1", "");
        let mut engine = PsEngine::new().unwrap();
        assert_eq!(engine.run_file(&exit7, &HashMap::new()).unwrap(), 7);
        assert_eq!(engine.run_file(&ok, &HashMap::new()).unwrap(), 0);
        assert!(engine.child.try_wait().unwrap().is_none());
    }

    /// Latency contrast vs the one-shot -File spawn baseline.
    #[test]
    #[ignore = "benchmark; engine per-submit vs cold spawn, view by eye"]
    fn bench_engine_per_submit() {
        if !shell_available() {
            return;
        }
        let dir = crate::test_utils::tmpdir("psengine_bench");
        let script = write_script(&dir, "s.ps1", "Write-Output x");
        let mut engine = PsEngine::new().unwrap();
        let n = 20;
        let mut t = Vec::with_capacity(n);
        for _ in 0..n {
            let sw = std::time::Instant::now();
            engine.run_file(&script, &HashMap::new()).unwrap();
            t.push(sw.elapsed().as_secs_f64() * 1000.0);
        }
        let tail = &t[1..];
        let avg: f64 = tail.iter().sum::<f64>() / tail.len() as f64;
        eprintln!(
            "engine per-submit (excl cold): cold={:.1}ms avg={:.2}ms min={:.2}ms max={:.2}ms; baseline -File spawn ~0.7s",
            t[0],
            avg,
            tail.iter().cloned().fold(f64::MAX, f64::min),
            tail.iter().cloned().fold(0.0, f64::max)
        );
    }
}
