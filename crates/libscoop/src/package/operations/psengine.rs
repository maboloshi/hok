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
//! - [PsEngine::run_file] blocks until its own DONE line arrives, then reads
//!   the capture file (returned as output lines) and removes it.
//! - A script that calls exit N terminates the whole worker; the exit code is
//!   surfaced and the engine transparently respawns on the next call.
//! - [PsEngine::cancel] kills the worker (used for cooperative Ctrl-C); the
//!   engine respawns on the next call.
//!
//! Not thread-safe: one submitter at a time. In an async context, callers run
//! [PsEngine::run_file_captured] on a blocking worker and await it.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// Server script executed by the worker (see module docs).
const SERVER_PS1: &str = include_str!("../../../../../asset_scripts/engine_server.ps1");

static SERVER_SEQ: AtomicU64 = AtomicU64::new(0);

/// Globally-unique per-submission counter, so capture file names cannot
/// collide across engines or parallel test threads in one process.
static CAPTURE_SEQ: AtomicU64 = AtomicU64::new(0);

/// A reusable, persistent PowerShell worker.
pub struct PsEngine {
    /// Running pwsh/powershell process running [SERVER_PS1].
    child: Child,
    /// Sink for submission frames (one RUN <json> line per call).
    stdin: ChildStdin,
    /// Worker stdout: the DONE_<token>:<code> completion line per submission.
    stdout: BufReader<ChildStdout>,
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
            stdout: BufReader::new(stdout),
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
    /// returning its exit code (dropping captured output).
    pub fn run_file(&mut self, path: &Path, env: &HashMap<String, String>) -> io::Result<i32> {
        self.run_file_captured(path, env).map(|(code, _)| code)
    }

    /// Run a script file in the engine, returning its exit code and captured
    /// output lines (all streams merged). Blocks until the submission finishes.
    pub fn run_file_captured(
        &mut self,
        path: &Path,
        env: &HashMap<String, String>,
    ) -> io::Result<(i32, Vec<String>)> {
        self.ensure_alive()?;
        let token = self.next_token;
        self.next_token = self.next_token.wrapping_add(1);

        // Per-submission capture file for this script's merged output.
        let cseq = CAPTURE_SEQ.fetch_add(1, Ordering::Relaxed);
        let capture = std::env::temp_dir().join(format!(
            "hok-psengine-out-{}-{}.txt",
            std::process::id(),
            cseq
        ));
        let frame = serde_json::json!({
            "t": token.to_string(),
            "p": path.to_string_lossy(),
            "env": env,
            "f": capture.to_string_lossy(),
        });
        let frame = format!("RUN {frame}\n");
        self.stdin.write_all(frame.as_bytes())?;
        self.stdin.flush()?;

        let prefix = format!("DONE_{token}:");
        let mut line = String::new();
        let code = match self.stdout.read_line(&mut line) {
            // EOF: the worker died (e.g. a script called exit N). Surface its
            // exit status; the next call respawns.
            Ok(0) => self.child.wait()?.code().unwrap_or(-1),
            Ok(_) => {
                let stripped = line.trim_end_matches(['\r', '\n']);
                stripped
                    .strip_prefix(&prefix)
                    .and_then(|rest| rest.trim().parse::<i32>().ok())
                    .unwrap_or(1)
            }
            Err(e) => return Err(e),
        };

        // Read back the captured output, then drop the capture file.
        let output = std::fs::read_to_string(&capture)
            .map(|text| {
                text.lines()
                    .map(str::trim_end)
                    .filter(|l| !l.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let _ = std::fs::remove_file(&capture);

        Ok((code, output))
    }

    /// Kill the worker. Used for cooperative cancellation (Ctrl-C): a running
    /// script cannot be interrupted in place, so we drop the whole worker and
    /// let [PsEngine::run_file] respawn it on the next call.
    pub fn cancel(&mut self) -> io::Result<()> {
        let _ = self.child.kill();
        self.child.wait()?;
        Ok(())
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
