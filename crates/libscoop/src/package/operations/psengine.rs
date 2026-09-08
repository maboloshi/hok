//! A reusable persistent PowerShell engine for package scripts.
//!
//! Each package script stage normally spawns a fresh pwsh (~0.7 s cold start).
//! This module reuses one long-lived pwsh worker (server side in
//! asset_scripts/engine_server.ps1) across a package's script stages (and, in
//! the install path, across packages in one run) to amortise that cost.
//!
//! # Protocol
//!
//! Each submission writes one frame line to the worker:
//!
//!   RUN <json>   # {t: token, p: script path, env: {NAME: value, ...}}
//!
//! The worker applies env, runs the script at p, and streams the script's
//! output (all streams merged) to the worker's stdout *as it is produced*.
//! hok drains that stdout continuously and forwards every non-terminator
//! line to the per-call sink, until a single terminator
//!
//!   DONE_<token>:<exitcode>
//!
//! arrives when the script finishes. Only one submission runs at a time (the
//! engine is held behind a single-submitter mutex), so every line between a
//! RUN and its DONE belongs to that submission and needs no token split.
//!
//! # Lifecycle
//!
//! - [PsEngine::new] spawns the worker and keeps it alive across calls.
//! - [PsEngine::run_file_cancelable] runs one script file, draining output
//!   line by line to the supplied sink until its DONE line arrives, then
//!   returns the exit code. It observes a cooperative cancellation token
//!   *while* the submission runs: a script runs in-process inside the worker,
//!   so the only way to interrupt it mid-run is to kill the worker itself.
//!   When the token fires, the worker is killed (terminating the in-flight
//!   script), the outcome is flagged cancelled, and the engine transparently
//!   respawns on the next call. Lines already streamed before the kill are
//!   delivered to the sink — there is no capture file to read back afterwards,
//!   so output produced after a cancellation is lost (a known trade-off of
//!   streaming vs the old batch file).
//! - A script that calls exit N terminates the whole worker; the exit code is
//!   surfaced and the engine transparently respawns on the next call.
//!
//! Not thread-safe: one submitter at a time. In an async context, callers run
//! [PsEngine::run_file_cancelable] on a blocking worker and await it.

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

/// Outcome of one [PsEngine] submission.
pub struct PsRunOutcome {
    /// The script's exit code (worker-side `$LASTEXITCODE` / thrown-error 1).
    pub code: i32,
    /// `true` when the run was cut short by cancellation (worker killed).
    pub cancelled: bool,
}

/// A reusable, persistent PowerShell worker.
pub struct PsEngine {
    /// Running pwsh/powershell process running [SERVER_PS1].
    child: Child,
    /// Sink for submission frames (one RUN <json> line per call).
    stdin: ChildStdin,
    /// Worker stdout: the streamed script output lines followed by one
    /// `DONE_<token>:<code>` terminator line per submission. `Option` so a
    /// submission's completion watcher can temporarily own the reader while
    /// the engine observes cancellation on the same thread.
    stdout: Option<BufReader<ChildStdout>>,
    /// Temp path holding the server script; removed on drop.
    server_path: PathBuf,
    /// Per-engine unique token counter (also terminates its DONE line).
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
            // Server output (streamed script lines + one DONE line per
            // submission) all goes to stdout; stderr is never read and can be
            // null.
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

    /// Run a script file in the engine, returning its exit code and merged
    /// output lines (all streams). Test helper; blocks until the submission
    /// finishes and collects the streamed lines (no live sink).
    #[cfg(test)]
    pub fn run_file_captured(
        &mut self,
        path: &Path,
        env: &HashMap<String, String>,
    ) -> io::Result<(i32, Vec<String>)> {
        let mut lines = Vec::new();
        let outcome = self.run_submission(path, env, None, |line| lines.push(line))?;
        Ok((outcome.code, lines))
    }

    /// Run a script file in the engine while observing a cooperative
    /// cancellation token, streaming each merged output line to `on_line` as
    /// it is produced. Blocks until the submission finishes *or* the token
    /// fires. When the token fires mid-run, the worker is killed — terminating
    /// the in-flight script (scripts run in-process inside the worker, so this
    /// is the only way to interrupt them) — and the returned outcome is flagged
    /// [`PsRunOutcome::cancelled`]. The engine respawns on the next call.
    pub fn run_file_cancelable(
        &mut self,
        path: &Path,
        env: &HashMap<String, String>,
        token: &CancellationToken,
        on_line: impl FnMut(String),
    ) -> io::Result<PsRunOutcome> {
        self.run_submission(path, env, Some(token), on_line)
    }

    /// Shared submission core. Writes one RUN frame, drains the worker's
    /// stdout line by line, forwarding each non-terminator line to `on_line`
    /// until the `DONE_<token>` terminator arrives (optionally racing it
    /// against a cancellation token, which kills the worker), restores the
    /// stdout reader, and assembles the outcome.
    ///
    /// The reader *continuously* drains stdout (a watcher thread when a token
    /// must be observed concurrently, inline otherwise) so a chatty script can
    /// never fill the pipe and deadlock the worker against our wait.
    fn run_submission<F>(
        &mut self,
        path: &Path,
        env: &HashMap<String, String>,
        token: Option<&CancellationToken>,
        mut on_line: F,
    ) -> io::Result<PsRunOutcome>
    where
        F: FnMut(String),
    {
        self.ensure_alive()?;
        let stoken = self.next_token;
        self.next_token = self.next_token.wrapping_add(1);

        let frame = serde_json::json!({
            "t": stoken.to_string(),
            "p": path.to_string_lossy(),
            "env": env,
        });
        let frame = format!("RUN {frame}\n");
        self.stdin.write_all(frame.as_bytes())?;
        self.stdin.flush()?;

        let prefix = format!("DONE_{stoken}:");

        // ── sync path (no token): drain inline on this thread ──
        if token.is_none() {
            let reader = self.stdout.as_mut().expect("engine stdout present");
            let done_code = drain_until_done(reader, &prefix, &mut on_line)?;
            let code = match done_code {
                Some(c) => c,
                // EOF: the worker died (e.g. a script called exit N). Surface
                // its exit status; the next call respawns.
                None => self.child.wait()?.code().unwrap_or(-1),
            };
            return Ok(PsRunOutcome {
                code,
                cancelled: false,
            });
        }

        // ── cancelable path: a watcher thread owns & drains the reader while
        //    this thread polls the token, forwarding lines as they arrive ──
        let mut reader = self.stdout.take().expect("engine stdout present");
        let (tx, rx) = mpsc::channel();
        let prefix_c = prefix.clone();
        let watcher = std::thread::spawn(move || {
            let done_code = drain_until_done(&mut reader, &prefix_c, &mut |line| {
                // The channel is unbounded, so this only fails once the
                // receiver has been dropped (main loop gave up on cancel);
                // drain_until_done then stops and the reader comes back via End.
                let _ = tx.send(ReaderMsg::Line(line));
            })
            .ok()
            .flatten();
            // Hand the reader back for the next submission.
            let _ = tx.send(ReaderMsg::End { done_code, reader });
        });

        let mut cancelled = false;
        let mut code = 0;
        loop {
            if let Some(tok) = token
                && tok.is_cancelled()
            {
                cancelled = true;
                let _ = self.child.kill();
                break;
            }
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(ReaderMsg::Line(line)) => on_line(line),
                Ok(ReaderMsg::End { done_code, reader }) => {
                    self.stdout = Some(reader);
                    code = match done_code {
                        Some(c) => c,
                        // EOF: worker died (exit N / killed). Surface its status.
                        None => self.child.wait()?.code().unwrap_or(-1),
                    };
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                // Watcher ended without an End (should not happen); treat as
                // EOF so the engine stays usable via respawn.
                Err(_) => {
                    code = self.child.wait()?.code().unwrap_or(-1);
                    break;
                }
            }
        }

        // If the token fired, the watcher sees EOF right after the kill; make
        // sure it returned the reader so the engine stays consistent.
        if cancelled && self.stdout.is_none() {
            let _ = watcher.join();
            while let Ok(msg) = rx.recv() {
                if let ReaderMsg::End { reader, .. } = msg {
                    self.stdout = Some(reader);
                    break;
                }
            }
            code = self.child.wait()?.code().unwrap_or(-1);
        }

        Ok(PsRunOutcome { code, cancelled })
    }
}

/// A message the cancelable-path watcher thread sends back to the draining
/// loop: a forwarded output line, or the terminal DONE/EOF result plus the
/// (returned) stdout reader.
enum ReaderMsg {
    /// A non-terminator output line forwarded from the worker.
    Line(String),
    /// The submission ended: its exit code, or `None` for EOF (worker died /
    /// killed). Carries the reader back for the next submission.
    End {
        done_code: Option<i32>,
        reader: BufReader<ChildStdout>,
    },
}

/// Read lines from `reader`, invoking `on_line` for each non-empty
/// non-terminator line, until this submission's `DONE_<token>:<code>`
/// terminator (returning its exit code) or EOF (returning `None`). Shared by
/// the sync and cancelable-watcher drain loops; keeps reading continuously so
/// a chatty worker can never fill the pipe while we wait.
fn drain_until_done<R, F>(
    reader: &mut BufReader<R>,
    prefix: &str,
    on_line: &mut F,
) -> io::Result<Option<i32>>
where
    R: std::io::Read,
    F: FnMut(String),
{
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => return Ok(None), // EOF
            Ok(_) => {
                if let Some(code) = parse_done(prefix, &buf) {
                    return Ok(Some(code));
                }
                if let Some(line) = clean_output_line(&buf) {
                    on_line(line);
                }
            }
            Err(e) => return Err(e),
        }
    }
}

/// Parse a worker terminator `DONE_<token>:<code>` from raw line bytes
/// (trailing newline optional) into its exit code; `None` when the line is not
/// *this* token's terminator (and is therefore ordinary script output).
fn parse_done(prefix: &str, buf: &[u8]) -> Option<i32> {
    let line = String::from_utf8_lossy(buf);
    let line = line.trim_end_matches(['\r', '\n']);
    line.strip_prefix(prefix)
        .and_then(|rest| rest.trim().parse::<i32>().ok())
}

/// Trim a raw output line (lossy UTF-8, leading BOM and trailing newline
/// stripped) into an owned line; `None` when it is empty after trimming.
fn clean_output_line(buf: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(buf);
    let s = s
        .trim_start_matches('\u{feff}')
        .trim_end_matches(['\r', '\n']);
    if s.is_empty() {
        None
    } else {
        Some(s.to_owned())
    }
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

    /// Output (all streams merged) is streamed out of the worker rather than
    /// written to a capture file: the sync test helper collects the same lines
    /// a live sink would receive, including non-ASCII (the worker emits UTF-8).
    #[test]
    fn engine_streams_merged_output_lines() {
        if !shell_available() {
            return;
        }
        let dir = crate::test_utils::tmpdir("psengine_stream_merge");
        let script = write_script(
            &dir,
            "s.ps1",
            "Write-Output 'alpha-line'\nWrite-Host 'beta-世界'\nWrite-Output 'gamma-line'",
        );
        let mut engine = PsEngine::new().unwrap();
        let (code, lines) = engine.run_file_captured(&script, &HashMap::new()).unwrap();
        assert_eq!(code, 0);
        let joined = lines.join("\n");
        assert!(joined.contains("alpha-line"), "got: {joined:?}");
        assert!(joined.contains("gamma-line"), "got: {joined:?}");
        assert!(
            joined.contains("beta-世界"),
            "merged Write-Host (info stream) must survive UTF-8, got: {joined:?}"
        );
    }

    /// Streaming really is line-by-line, not batch-at-end: a script that
    /// writes 'first', sleeps, then writes 'second' must deliver 'first' to the
    /// sink well before the run returns (a batch model would deliver it only at
    /// completion). Exercises the cancelable/watcher drain path with a
    /// never-cancelled token.
    #[test]
    fn engine_streams_lines_before_completion() {
        if !shell_available() {
            return;
        }
        let dir = crate::test_utils::tmpdir("psengine_stream_timing");
        let script = write_script(
            &dir,
            "s.ps1",
            "Write-Output 'first-A'\nStart-Sleep -Milliseconds 600\nWrite-Output 'second-B-世界'",
        );
        let mut engine = PsEngine::new().unwrap();
        let token = tokio_util::sync::CancellationToken::new();
        let mut seen: Vec<(std::time::Instant, String)> = Vec::new();
        engine
            .run_file_cancelable(&script, &HashMap::new(), &token, |line| {
                seen.push((std::time::Instant::now(), line));
            })
            .unwrap();
        let run_end = std::time::Instant::now();

        assert!(
            seen.len() >= 2,
            "expected at least first/second lines, got: {seen:?}"
        );
        assert!(seen[0].1.contains("first-A"), "got: {:?}", seen[0].1);
        assert!(seen[1].1.contains("second-B-世界"), "got: {:?}", seen[1].1);
        // 'first-A' must have arrived well before the whole run returned —
        // i.e. it was delivered while the script was still sleeping, proving
        // the output is streamed live rather than held until completion.
        let first_arrival = run_end.duration_since(seen[0].0);
        assert!(
            first_arrival >= std::time::Duration::from_millis(300),
            "first line should stream ~600ms before completion, arrived only {first_arrival:?} early"
        );
        // And 'second' arrived after the 600ms sleep separated it from 'first'.
        let gap = seen[1].0.duration_since(seen[0].0);
        assert!(
            gap >= std::time::Duration::from_millis(300),
            "expected ~600ms gap between first and second, got {gap:?}"
        );
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
