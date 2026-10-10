//! PowerShell script execution & installer-variable expansion primitives.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::Stdio;
use tokio::sync::mpsc;
use tracing::debug;

use super::psengine::{PsChannel, PsEngine};
use crate::constant::ISOLATED_PACKAGE_BUCKET;
use crate::package::Package;
use crate::{Error, Event, Session, error::Fallible, internal};

/// Which OS pipe a fresh-spawn reader drains. The spawn path maps its two real
/// pipes onto the two semantic events; warnings under `pwsh -File` land on
/// stderr (host rendering), so it cannot produce a distinct warning event.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SpawnStream {
    Stdout,
    Stderr,
}

/// Read a captured child pipe line by line (lossy UTF-8) and forward every
/// non-empty line to the event bus as a [`Event::ScriptStdout`] (stdout) or
/// [`Event::ScriptStderr`] (stderr) event, so the frontend renders script
/// output with the same indentation as its own lines. Runs on a worker thread
/// (see `run_script`) so a chatty script cannot deadlock the child against the
/// wait loop on a full pipe buffer.
fn forward_script_pipe(
    mut reader: BufReader<impl Read>,
    tx: Option<mpsc::Sender<Event>>,
    which: SpawnStream,
) {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => {
                let cow = String::from_utf8_lossy(&buf);
                let line = internal::string::strip_bom(&cow).trim_end_matches(['\r', '\n']);
                if line.is_empty() {
                    continue;
                }
                if let Some(tx) = tx.as_ref() {
                    let ev = match which {
                        SpawnStream::Stdout => Event::ScriptStdout(line.to_owned()),
                        SpawnStream::Stderr => Event::ScriptStderr(line.to_owned()),
                    };
                    let _ = tx.try_send(ev);
                }
            }
            Err(_) => break,
        }
    }
}

/// Translate a failure to *launch* the PowerShell process into a clear,
/// actionable error.
///
/// Every package script runs on pwsh/powershell (both the fresh-spawn and the
/// resident-engine path use [`internal::os::ps_command`]), so a launch failure
/// here means no usable PowerShell could be started — it is missing, not on
/// PATH, or a present-but-broken binary cannot start. There is deliberately no
/// fallback between the two execution paths (both need the same shell), so the
/// error names the requirement and the stage/package rather than surfacing a
/// cryptic spawn `io::Error`.
fn ps_launch_error(stage: &str, name: &str, err: std::io::Error) -> Error {
    Error::Custom(format!(
        "cannot run {stage} script for '{name}': could not start PowerShell \
         (pwsh.exe or powershell.exe is missing or broken): {err}"
    ))
}

/// Compute a manifest's install-time environment additions.
///
/// Mirrors Scoop's `env_add_path` + `env_set` hooks (lib/install.ps1:62-63),
/// which run *just before* `post_install` (lib/install.ps1:69) and write into
/// the current session so the script — and its own child processes — inherit
/// them (e.g. a `post_install` running `initdb` reads `$env:PGDATA`).
/// Only the child's environment is touched; the host process env is unchanged.
///
/// Only the `post_install` stage gets these, matching Scoop's ordering where
/// `pre_install`/installer run before the env hooks.
fn install_env_additions(
    env: &mut std::collections::HashMap<String, String>,
    session: &Session,
    package: &Package,
    working_dir: &Path,
    cmd: &str,
) {
    let manifest = package.manifest();
    // env_set: set each declared variable (Scoop's env_set also writes to the
    // session env, `Set-Content env:$name`).
    if let Some(env_set) = manifest.env_set() {
        for (key, value) in env_set {
            let expanded = expand_scoop_vars(&[value.as_str()], session, package, working_dir, cmd);
            env.insert(key.clone(), expanded[0].clone());
        }
    }
    // env_add_path: prepend the bin dirs to PATH so post_install can invoke
    // freshly-installed binaries by name (Scoop's Add-Path updates the session
    // PATH). Only the child's PATH is affected — the host PATH is untouched.
    if let Some(add_path) = manifest.env_add_path() {
        let add_paths: Vec<String> = add_path
            .into_iter()
            .filter(|p| !p.is_empty())
            .map(|p| {
                internal::path::normalize_path(working_dir.join(p))
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        if !add_paths.is_empty() {
            let mut parts: Vec<String> =
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
            // Prepend each path once, case-insensitively, keeping manifest
            // order (mirrors Scoop's Add-Path current-session update).
            for p in add_paths {
                let lower = p.to_lowercase();
                if parts.iter().any(|x| x.to_lowercase() == lower) {
                    continue;
                }
                parts.insert(0, p);
            }
            if let Ok(path) = std::env::join_paths(parts) {
                env.insert("PATH".to_string(), path.to_string_lossy().into_owned());
            }
        }
    }
}

/// The value a package script sees as `$bucket`.
///
/// Upstream sets `$bucket = $install.bucket` (uninstall/update) and leaves it
/// `$null` for an app installed from a URL or local path, so an isolated
/// package's script must see an **empty string** — not hok's internal
/// [`ISOLATED_PACKAGE_BUCKET`] sentinel, which is a storage detail of the
/// installed record and the installed manifest.
fn script_bucket(package: &Package) -> &str {
    let bucket = package.bucket();
    if bucket == ISOLATED_PACKAGE_BUCKET {
        ""
    } else {
        bucket
    }
}

/// Embedded core helpers (available to every package script).
const CORE_PS1: &str = include_str!("../../../../../asset_scripts/core.ps1");

/// Embedded decompress helpers. These only emit `[[HOK_EXTRACT]]` markers;
/// Rust performs the extraction (see `super::extract_markers`).
const DECOMPRESS_PS1: &str = include_str!("../../../../../asset_scripts/decompress.ps1");

/// Execute a PowerShell script defined in a package manifest.
///
/// `script_lines` is an array of PowerShell command lines that will be joined
/// and executed. The function is a no-op if `script_lines` is `None`.
///
/// Environment variables set for the script:
/// - `SCOOP` — the Scoop root directory
/// - `SCOOP_APP_DIR` — the package's installation directory (`$dir`)
/// - `SCOOP_APP_ORIGINAL_DIR` — the real (versioned) install directory
///   (`$original_dir`), set when it differs from `$dir` (post_install runs
///   with `$dir` = the `current` junction, mirroring upstream `link_current`)
/// - `SCOOP_PACKAGE_NAME` — the package name
/// - `SCOOP_PACKAGE_VERSION` — the installed version
/// - `version` — same as SCOOP_PACKAGE_VERSION (Scoop convention)
///
/// For the `post_install` stage only, the manifest's `env_set` variables and
/// `env_add_path` directories are also applied (see
/// [`install_env_additions`]), mirroring Scoop's env hooks running right
/// before post_install (lib/install.ps1:62-63,69).
///
/// Execution path is gated by the `script_engine` config (default off):
/// - **off** (default): each stage spawns a fresh `pwsh -File`, piping its
///   stdout/stderr separately to `ScriptStdout` / `ScriptStderr` events with
///   prompt Ctrl-C cancellation.
/// - **on**: the stage runs through the session's resident [PsEngine], reusing
///   one long-lived pwsh worker across stages; output is streamed live,
///   split into stdout / warning / stderr events, and Ctrl-C kills the worker
///   (respawned on the next stage).
pub async fn run_script(
    session: &Session,
    package: &Package,
    working_dir: &Path,
    original_dir: Option<&Path>,
    stage: &str,
    cmd: &str,
    script_lines: Option<Vec<&str>>,
) -> Fallible<()> {
    let lines = match script_lines {
        Some(l) if !l.is_empty() => l,
        _ => return Ok(()),
    };

    debug!(
        "run_script: {} stage={} ({} lines)",
        package.name(),
        stage,
        lines.len()
    );

    let script = lines.join("\r\n");

    // `$global` keeps its boolean type (is_global is a real bool); the
    // preamble converts it via [bool]::Parse. A bare `false` would be parsed
    // as a command name by PowerShell, tripping the `[[HOK_MISSING_HELPER]]`
    // trap on every script run. Upstream sets `$global` the same way.
    let is_global = session.is_global();

    // `$fname`: remote filename of the first download URL (Scoop semantics,
    // install.ps1 `$Name[0]`). Package scripts commonly reference
    // `"$dir\$fname"` (e.g. `Expand-InnoArchive` on an installer exe);
    // `copy_downloaded_files` places the file in `$dir` under exactly this
    // name, so the variable must resolve to it.
    let fname = first_download_fname(&package.manifest().url());

    // Embed PS helper scripts so package scripts can use functions
    // like Expand-InnoArchive, Expand-7zipArchive, Get-HelperPath, etc.
    let preamble = format!(
        r#"
# Force UTF-8 so the captured stdout/stderr (re-emitted by run_script)
# decodes correctly regardless of the console codepage.
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8

# Hok embedded helpers (always available)
{core}
{decompress}
# Fallback: load Scoop lib if installed
$hokScoopLib = Join-Path $env:SCOOP "apps\scoop\current\lib"
if (Test-Path $hokScoopLib) {{
    Get-ChildItem $hokScoopLib -Filter *.ps1 | ForEach-Object {{ . $_.FullName -ErrorAction SilentlyContinue }}
}}

# Notify hok about undefined commands (missing helpers). A bare `continue`
# would swallow *every* terminating error (a genuine throw/abort included),
# silently letting a failed stage "succeed". So only the CommandNotFound case
# (a helper hok cannot provide) is notified and continued; every other
# terminating error is rethrown (throw $_) so the stage fails and the real
# reason reaches hok — the engine's structured-error capture or the spawn
# path's nonzero exit.
trap {{
    if ($_.Exception -is [System.Management.Automation.CommandNotFoundException]) {{
        Write-Host "[[HOK_MISSING_HELPER]]$($_.Exception.CommandName)"
        continue
    }}
    throw $_
}}

# Scoop-compatible variables for package scripts
$dir = $env:SCOOP_APP_DIR
$original_dir = if ($env:SCOOP_APP_ORIGINAL_DIR) {{ $env:SCOOP_APP_ORIGINAL_DIR }} else {{ $dir }}
$scoopdir = $env:SCOOP
$bucketsdir = Join-Path $scoopdir "buckets"
$persist_dir = Join-Path $scoopdir "persist" $env:SCOOP_PACKAGE_NAME
$version = $env:SCOOP_PACKAGE_VERSION
$app = $env:SCOOP_PACKAGE_NAME
$bucket = $env:SCOOP_PACKAGE_BUCKET
$architecture = "{arch}"
$global = [bool]::Parse('{is_global}')
$cmd = $env:SCOOP_PACKAGE_CMD
$fname = '{fname}'
"#,
        core = CORE_PS1,
        decompress = DECOMPRESS_PS1,
        arch = crate::internal::arch::Arch::current().name(),
        is_global = is_global,
        fname = fname.replace('\'', "''")
    );
    let full_script = format!("{preamble}\r\n{script}");

    // Write script to a temp file in the working dir
    let script_path = working_dir.join(format!("{}.ps1", stage));
    if let Some(parent) = script_path.parent() {
        internal::fs::ensure_dir(parent)?;
    }
    std::fs::write(&script_path, &full_script)?;

    // Build environment variables
    let root_path = session.effective_root_path();
    let pkg_dir = working_dir.to_path_buf(); // $dir = version dir (not current)
    let original_dir = original_dir.unwrap_or(working_dir);

    let version = package.effective_version();

    // Create marker file for P2 extraction routing
    let marker_path = working_dir.join("hok_extract_markers.txt");
    let _ = std::fs::remove_file(&marker_path); // clean from previous runs

    // Ensure both temp files are cleaned up on return, even via `?`
    let _guard = TempFileGuard::new(vec![script_path.clone(), marker_path.clone()]);

    // Shared process-env map for both the fresh-spawn and resident-engine
    // paths, so the two cannot drift.
    let mut env = HashMap::new();
    env.insert("SCOOP".into(), root_path.to_string_lossy().into_owned());
    env.insert(
        "SCOOP_APP_DIR".into(),
        pkg_dir.to_string_lossy().into_owned(),
    );
    env.insert(
        "SCOOP_APP_ORIGINAL_DIR".into(),
        original_dir.to_string_lossy().into_owned(),
    );
    env.insert("SCOOP_PACKAGE_NAME".into(), package.name().to_owned());
    env.insert("SCOOP_PACKAGE_VERSION".into(), version.clone());
    env.insert(
        "SCOOP_PACKAGE_BUCKET".into(),
        script_bucket(package).to_owned(),
    );
    env.insert("SCOOP_PACKAGE_CMD".into(), cmd.to_owned());
    env.insert("version".into(), version);
    env.insert(
        "HOK_EXTRACT_FILE".into(),
        marker_path.to_string_lossy().into_owned(),
    );

    // The `post_install` stage runs after Scoop's `env_add_path` + `env_set`
    // hooks (install.ps1:62-63,69), so hand it the manifest's install-time
    // environment — post_install scripts (e.g. initdb reading `$env:PGDATA`)
    // and their child processes inherit it. Other stages run before the env
    // hooks in Scoop, so they do not.
    if stage == "post_install" {
        install_env_additions(&mut env, session, package, working_dir, cmd);
    }

    // Every package script is a PowerShell script, so both the fresh-spawn and
    // the resident-engine path need a working PowerShell. There is deliberately
    // no availability probe here: each path fails at its own spawn point, and
    // that failure is translated into a clear message there (see
    // [run_script_spawn] and [run_script_engine]). A "downgrade" between the
    // two paths would be meaningless anyway — both run on pwsh/powershell.

    // Process extraction markers (P2: Rust native extraction) once the stage
    // body has run successfully; shared by both execution paths.
    let executed = if session.config().script_engine() {
        run_script_engine(session, package, &script_path, &env, stage).await
    } else {
        run_script_spawn(session, package, &script_path, &env, stage).await
    };
    executed?;
    super::extract_markers(session, working_dir).await;

    Ok(())
}

/// Execute a staged script via a fresh `pwsh -File` spawn (the default path).
///
/// Pipes stdout/stderr separately and forwards every non-empty line as a
/// `ScriptStdout` (stdout) or `ScriptStderr` (stderr) event so the frontend
/// renders script output with its own indentation. Cooperative Ctrl-C kills the
/// child (a lone powershell lingers otherwise); no hard timeout — Scoop scripts
/// may legitimately run for minutes.
async fn run_script_spawn(
    session: &Session,
    package: &Package,
    script_path: &Path,
    env: &HashMap<String, String>,
    stage: &str,
) -> Fallible<()> {
    // Prefer pwsh.exe (PowerShell Core, faster startup)
    let mut ps = crate::internal::os::ps_command();
    ps.arg("-File").arg(script_path).envs(env);

    // Pipe stdout/stderr so the script output is captured and re-emitted as
    // ScriptStdout / ScriptStderr events — a bare inherited stdout would print
    // at column 0 and break the frontend's indented alignment.
    ps.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = ps
        .spawn()
        .map_err(|e| ps_launch_error(stage, package.name(), e))?;

    // Drain the pipes on worker threads while the child runs; without this a
    // chatty script could fill a pipe buffer and deadlock the child against
    // our wait loop. Every non-empty line is forwarded to the event bus.
    let emitter = session.emitter();
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out_reader = if let Some(out) = stdout.take() {
        let tx = emitter.clone();
        Some(std::thread::spawn(move || {
            forward_script_pipe(BufReader::new(out), tx, SpawnStream::Stdout)
        }))
    } else {
        None
    };
    let err_reader = if let Some(err) = stderr.take() {
        let tx = emitter.clone();
        Some(std::thread::spawn(move || {
            forward_script_pipe(BufReader::new(err), tx, SpawnStream::Stderr)
        }))
    } else {
        None
    };

    // Cooperative Ctrl-C cancellation: a cancelled token kills the child so
    // no stray powershell lingers. There is deliberately no hard timeout here
    // — Scoop install scripts may legitimately run for minutes, so adding one
    // would change observable behavior.
    let token = session.cancellation_token();
    let status = loop {
        if token.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Custom(format!(
                "{} script for '{}' cancelled",
                stage,
                package.name()
            )));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                // Async sleep so the runtime is not blocked; race it against
                // cancellation so a Ctrl-C is noticed promptly.
                tokio::select! {
                    _ = token.cancelled() => {}
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
                }
            }
            Err(e) => {
                return Err(Error::Custom(format!(
                    "failed to wait for {} script for '{}': {}",
                    stage,
                    package.name(),
                    e
                )));
            }
        }
    };

    // Child finished — join the readers so every captured script-output line
    // is flushed into the event bus before the caller emits the next event
    // (e.g. the package closure).
    if let Some(t) = out_reader {
        let _ = t.join();
    }
    if let Some(t) = err_reader {
        let _ = t.join();
    }

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        return Err(Error::Custom(format!(
            "{} script for '{}' exited with code {}",
            stage,
            package.name(),
            code
        )));
    }

    Ok(())
}

/// Execute a staged script through the session's resident [PsEngine].
///
/// The engine is taken out of the session slot, runs the script on a blocking
/// worker (so the async runtime is not starved), and is returned for reuse
/// across stages/packages. The engine streams the script's classified output
/// (stdout / warning / stderr, from the worker's stream tags) to the event bus
/// *while it runs* (the same live-forwarding contract as the fresh-spawn
/// path), not in a batch after completion.
/// Cancellation observed mid-run kills the worker (terminating the in-flight
/// script); the engine respawns on its next use, mirroring the fresh-spawn
/// path's prompt abort.
async fn run_script_engine(
    session: &Session,
    package: &Package,
    script_path: &Path,
    env: &HashMap<String, String>,
    stage: &str,
) -> Fallible<()> {
    let slot = session.script_engine_slot().clone();
    let token = session.cancellation_token();
    let path = script_path.to_path_buf();
    let env = env.clone();
    // Clones captured by the 'static blocking closure; the originals stay for
    // post-await error formatting.
    let stage_in = stage.to_owned();
    let name_in = package.name().to_owned();
    // Forward each streamed line inside the blocking closure so a running
    // script's output is delivered to the event bus as it is produced.
    let tx = session.emitter();

    let outcome =
        tokio::task::spawn_blocking(move || -> Fallible<super::psengine::PsRunOutcome> {
            let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
            // Lazily spawn the engine on first use; a killed/crashed worker is
            // transparently respawned by the engine itself on its next call.
            let mut engine = match guard.take() {
                Some(e) => e,
                None => PsEngine::new().map_err(|e| ps_launch_error(&stage_in, &name_in, e))?,
            };
            let sink = {
                let tx = tx.clone();
                move |ch: PsChannel, line: String| {
                    if let Some(tx) = tx.as_ref() {
                        let ev = match ch {
                            PsChannel::Stdout => Event::ScriptStdout(line),
                            PsChannel::Warning => Event::ScriptWarning(line),
                            PsChannel::Error => Event::ScriptStderr(line),
                        };
                        let _ = tx.try_send(ev);
                    }
                }
            };
            let res = engine
                .run_file_cancelable(&path, &env, &token, sink)
                .map_err(|e| {
                    Error::Custom(format!(
                        "failed to run {stage_in} script for '{name_in}': {e}"
                    ))
                });
            // On cancellation the worker was killed in place; drop the engine
            // (a fresh one respawns on the next stage) so no dead worker lingers
            // in the slot across a cancelled run.
            let cancelled = res.as_ref().map(|o| o.cancelled).unwrap_or(false);
            if !cancelled {
                *guard = Some(engine);
            }
            res
        })
        .await
        .map_err(|e| {
            Error::Custom(format!(
                "failed to run {stage} script for '{}': {e}",
                package.name()
            ))
        })??;

    // Output was already streamed live during the run; only the terminal state
    // (code / cancellation) is checked here.
    if outcome.cancelled {
        return Err(Error::Custom(format!(
            "{} script for '{}' cancelled",
            stage,
            package.name()
        )));
    }
    if outcome.code != 0 {
        // A thrown error's message is preferred over a bare exit code so the
        // caller sees *why* the script failed; otherwise fall back to the code
        // (e.g. a script that called exit N and produced no throw message).
        let msg = if let Some(e) = &outcome.error {
            format!("{} script for '{}' failed: {}", stage, package.name(), e)
        } else {
            format!(
                "{} script for '{}' exited with code {}",
                stage,
                package.name(),
                outcome.code
            )
        };
        return Err(Error::Custom(msg));
    }
    Ok(())
}

/// Remote filename of the first download URL (Scoop `$fname` semantics):
/// the name `copy_downloaded_files` gives the file inside `$dir`.
fn first_download_fname(urls: &[&str]) -> String {
    urls.first()
        .map(|u| {
            let stripped = internal::url::strip_url_query(u);
            stripped.rsplit('/').next().unwrap_or_default().to_string()
        })
        .unwrap_or_default()
}

/// Expand Scoop-style variables (`$dir`, `$scoopdir`, `$persist_dir`, etc.)
/// in installer/uninstaller args and `env_set` values, replacing them with
/// the actual filesystem paths.
///
/// This mirrors the variable definitions in `run_script`'s PowerShell preamble,
/// so that `installer.file` and `uninstaller.file` (which run via `run_gui`
/// rather than through PowerShell) get equivalent variable expansion.
pub fn expand_scoop_vars(
    args: &[&str],
    session: &Session,
    pkg: &Package,
    working_dir: &Path,
    cmd: &str,
) -> Vec<String> {
    args.iter()
        .map(|arg| expand_scoop_str(arg, session, pkg, working_dir, cmd))
        .collect()
}

/// Expand Scoop-style variables in a single string (see [`expand_scoop_vars`]).
pub fn expand_scoop_str(
    s: &str,
    session: &Session,
    pkg: &Package,
    working_dir: &Path,
    cmd: &str,
) -> String {
    let root_path = session.effective_root_path();
    let persist_dir = session.persist_dir(pkg.name());
    let buckets_dir = session.buckets_dir();
    let version = pkg.version();
    let app = pkg.name();
    let bucket = script_bucket(pkg);

    let working_dir_str = working_dir.to_string_lossy().to_string();
    let root_path_str = root_path.to_string_lossy().to_string();
    let persist_dir_str = persist_dir.to_string_lossy().to_string();
    let buckets_dir_str = buckets_dir.to_string_lossy().to_string();

    let mut s = s.to_string();
    // Longer/replace more specific patterns first to avoid partial overlap
    s = s.replace("$original_dir", &working_dir_str);
    s = s.replace("$persist_dir", &persist_dir_str);
    s = s.replace("$bucketsdir", &buckets_dir_str);
    s = s.replace("$scoopdir", &root_path_str);
    s = s.replace(
        "$architecture",
        crate::internal::arch::Arch::current().name(),
    );
    s = s.replace("$version", version);
    s = s.replace("$app", app);
    s = s.replace("$bucket", bucket);
    s = s.replace(
        "$global",
        if session.is_global() { "true" } else { "false" },
    );
    s = s.replace("$cmd", cmd);
    s = s.replace("$dir", &working_dir_str);
    s
}

/// Run a manifest `installer.file` / `uninstaller.file` executable via
/// `run_gui`, with installer-variable-expanded arguments (see
/// [`expand_scoop_vars`]).
///
/// Mirrors Scoop's `Invoke-InstallerFile` / `Invoke-UninstallerFile`: the
/// binary runs detached (GUI-style) with `working_dir` as its working
/// directory. `stage` (`"installer"` / `"uninstaller"`) is used in error
/// messages; `cmd` (`"install"` / `"uninstall"`) is substituted for `$cmd`.
pub fn run_installer_file(
    session: &Session,
    package: &Package,
    working_dir: &Path,
    stage: &str,
    cmd: &str,
    file: &str,
    raw_args: &[&str],
) -> Fallible<()> {
    debug!("run_installer_file: {} - {}.file", package.name(), stage);
    let exe_path = working_dir.join(file);
    let expanded = expand_scoop_vars(raw_args, session, package, working_dir, cmd);
    let args: Vec<&str> = expanded.iter().map(|s| s.as_str()).collect();
    let exit_code = if file.to_ascii_lowercase().ends_with(".ps1") {
        // Upstream runs .ps1 installer/uninstaller files in-process
        // (`& $progName @fnArgs`, lib/install.ps1:127-128) — ShellExecute
        // would just open the script in an editor. Run via PowerShell.
        run_ps1_file(&exe_path, &args, session, package, working_dir, cmd).map_err(|e| {
            Error::Custom(format!(
                "failed to run {} '{}' for '{}': {}",
                stage,
                file,
                package.name(),
                e
            ))
        })?
    } else {
        internal::os::run_gui(&exe_path, &args, Some(working_dir)).map_err(|e| {
            Error::Custom(format!(
                "failed to run {} '{}' for '{}': {}",
                stage,
                file,
                package.name(),
                e
            ))
        })?
    };
    // Upstream `Invoke-ExternalCommand` treats a non-zero exit as failure
    // and aborts ("Installation aborted." / "Uninstallation aborted.").
    if exit_code != 0 {
        return Err(Error::Custom(format!(
            "{} '{}' for '{}' exited with code {}",
            stage,
            file,
            package.name(),
            exit_code
        )));
    }
    Ok(())
}

/// Run a manifest `installer`/`uninstaller` hook: the `*.file` executable
/// (or the first download URL's filename when only `*.args` is given),
/// followed by the `*.script` hook — mirroring upstream `Invoke-Installer`
/// / `Invoke-Uninstaller` (lib/install.ps1:110-115, 144, 163-166).
///
/// Shared by the install and remove pipelines so the hook semantics
/// cannot drift. `hook` (`"installer"` / `"uninstaller"`) and `cmd`
/// (`"install"` / `"uninstall"`) are passed through to
/// [`run_installer_file`] / [`run_script`] and used in debug/error
/// messages and `$cmd` expansion.
// 9 args is deliberate: one shared entry point for both hook stages
// (file + script) across the install and remove pipelines; a struct
// would just move the same fields behind one more indirection.
#[allow(clippy::too_many_arguments)]
pub async fn run_hook(
    session: &Session,
    package: &Package,
    dir: &Path,
    hook: &str,
    cmd: &str,
    file: Option<&str>,
    args: Option<Vec<&str>>,
    keep: bool,
    script: Option<Vec<&str>>,
) -> Fallible<()> {
    // 1. Hook file. Runs when `*.file` is set *or* when only `*.args` is
    //    given — upstream falls back to the first download URL's filename.
    let raw_args = args.unwrap_or_default();
    if file.is_some() || !raw_args.is_empty() {
        let resolved = match file {
            Some(f) => f.to_owned(),
            None => {
                let first_url = package.manifest().url().first().copied().unwrap_or("");
                internal::url::url_filename(first_url).to_owned()
            }
        };
        debug!(
            "run_hook: {} - {}.file ({})",
            package.name(),
            hook,
            resolved
        );
        run_installer_file(session, package, dir, hook, cmd, &resolved, &raw_args)?;
        // Don't remove the hook file when `keep` is set (Scoop order:
        // `!$installer.keep` -> Remove-Item, lib/install.ps1).
        if !keep {
            let hook_path = dir.join(&resolved);
            if hook_path.exists() {
                std::fs::remove_file(&hook_path)?;
            }
        }
    }

    // 2. `*.script` runs regardless of the file step (upstream
    //    Invoke-HookScript is called after Invoke-Installer,
    //    lib/install.ps1:144,163-166).
    if let Some(script) = script {
        debug!("run_hook: {} - {}.script", package.name(), hook);
        run_script(session, package, dir, None, hook, cmd, Some(script)).await?;
    }

    Ok(())
}

/// Run a `.ps1` installer/uninstaller file via PowerShell, mirroring
/// upstream's in-process `& $progName @fnArgs` (lib/install.ps1:127-128).
/// The same `SCOOP_*` environment as [`run_script`] is provided so `$dir`
/// and friends resolve inside the script. Returns the process exit code.
fn run_ps1_file(
    exe_path: &Path,
    args: &[&str],
    session: &Session,
    package: &Package,
    working_dir: &Path,
    cmd: &str,
) -> std::io::Result<i32> {
    let root_path = session.effective_root_path();
    let mut ps = crate::internal::os::ps_command();
    ps.arg("-File")
        .arg(exe_path)
        .env("SCOOP", root_path.as_os_str())
        .env("SCOOP_APP_DIR", working_dir.as_os_str())
        .env("SCOOP_PACKAGE_NAME", package.name())
        .env("SCOOP_PACKAGE_VERSION", package.version())
        .env("SCOOP_PACKAGE_BUCKET", script_bucket(package))
        .env("SCOOP_PACKAGE_CMD", cmd);
    ps.args(args);
    let status = ps.status()?;
    Ok(status.code().unwrap_or(-1))
}

/// Removes the given file paths when dropped. Ensures cleanup even when
/// the calling function returns early via `?`.
struct TempFileGuard(Vec<std::path::PathBuf>);

impl TempFileGuard {
    fn new(paths: Vec<std::path::PathBuf>) -> Self {
        Self(paths)
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dot-sources the embedded `decompress.ps1` and checks each helper name is
    /// bound to the intended body (see the test below). Exits non-zero if any
    /// name is missing or bound to the wrong definition.
    const DECOMPRESS_PROBE_PS1: &str = r#"
. "$PSScriptRoot\decompress.ps1"
$script:fail = 0
function Assert-Def($Name, $Needle) {
    $d = (Get-Command $Name -CommandType Function -ErrorAction SilentlyContinue).Definition
    if (-not $d) { Write-Host "MISSING:$Name"; $script:fail = 1; return }
    if ($d -notlike "*$Needle*") { Write-Host "BAD:$Name=$d"; $script:fail = 1; return }
    Write-Host "OK:$Name"
}
Assert-Def 'Expand-7zipArchive' 'Write-ExtractMarker "7z"'
Assert-Def 'Expand-MsiArchive' 'Write-ExtractMarker "msi"'
Assert-Def 'Expand-InnoArchive' 'Write-ExtractMarker "innosetup"'
Assert-Def 'Expand-Msi' 'Expand-MsiArchive'
Assert-Def 'Expand-ZipArchive' 'Expand-7zipArchive'
# `-Switches` / `-Overwrite` must stay declared parameters: without them
# PowerShell feeds `-Overwrite` to `ValueFromRemainingArguments`
# (`$Switches`) and the value is silently dropped.
function Assert-Param($Name, $Param) {
    $p = (Get-Command $Name -CommandType Function -ErrorAction SilentlyContinue).Parameters
    if (-not $p -or -not $p.ContainsKey($Param)) {
        Write-Host "MISSINGPARAM:${Name}:${Param}"; $script:fail = 1; return
    }
    Write-Host "PARAM-OK:${Name}:${Param}"
}
Assert-Param 'Expand-7zipArchive' 'Switches'
Assert-Param 'Expand-7zipArchive' 'Overwrite'
Assert-Param 'Expand-7zipArchive' 'Removal'
Assert-Param 'Expand-InnoArchive' 'Switches'
Assert-Param 'Expand-MsiArchive' 'Switches'
# X5b: only `Path`/`DestinationPath` may be positional, exactly like official
# Scoop (`lib/decompress.ps1:66-71,172-185,229-242,306-316`). PowerShell makes a
# parameter positional only while no parameter declares a `Position`, so once
# `ExtractDir` lost its `Position = 2` the leftovers must flow to `$Switches`.
function Assert-NamedOnly($Name, $Param) {
    $p = (Get-Command $Name -CommandType Function -ErrorAction SilentlyContinue).Parameters
    if (-not $p -or -not $p.ContainsKey($Param)) {
        Write-Host "MISSINGPARAM:${Name}:${Param}"; $script:fail = 1; return
    }
    $positional = $p[$Param].Attributes |
        Where-Object { $_ -is [System.Management.Automation.ParameterAttribute] -and $_.Position -ge 0 }
    if ($positional) {
        Write-Host "BAD:PositionalParam:${Name}:${Param}=$($positional.Position -join ',')"
        $script:fail = 1
        return
    }
    Write-Host "PARAM-OK:${Name}:${Param}:named-only"
}
function Assert-Position($Name, $Param, $Expected) {
    $p = (Get-Command $Name -CommandType Function -ErrorAction SilentlyContinue).Parameters
    $positional = @($p[$Param].Attributes |
        Where-Object { $_ -is [System.Management.Automation.ParameterAttribute] -and $_.Position -ge 0 })
    if ($positional.Count -ne 1 -or $positional[0].Position -ne $Expected) {
        Write-Host "BAD:Position:${Name}:${Param}"
        $script:fail = 1
        return
    }
    Write-Host "PARAM-OK:${Name}:${Param}:Position=$Expected"
}
foreach ($fn in 'Expand-7zipArchive', 'Expand-InnoArchive', 'Expand-MsiArchive', 'Expand-DarkArchive') {
    Assert-Position $fn 'Path' 0
    Assert-Position $fn 'DestinationPath' 1
    Assert-NamedOnly $fn 'ExtractDir'
    Assert-NamedOnly $fn 'Switches'
}
Assert-NamedOnly 'Expand-7zipArchive' 'Overwrite'
# Behavioral proof of the same rule: a third positional argument must be
# recorded as `switches`, not as `extract_dir`.
$env:HOK_EXTRACT_FILE = Join-Path $PSScriptRoot 'marker.txt'
function Assert-Marker($Command, $Expected) {
    Remove-Item $env:HOK_EXTRACT_FILE -Force -ErrorAction SilentlyContinue
    Invoke-Expression $Command
    $line = if (Test-Path $env:HOK_EXTRACT_FILE) { (Get-Content $env:HOK_EXTRACT_FILE -Raw).Trim() } else { '' }
    if ($line -ne $Expected) {
        Write-Host "BAD:Marker:$Command -> $line"
        $script:fail = 1
        return
    }
    Write-Host "PARAM-OK:Marker:$Command"
}
Assert-Marker "Expand-7zipArchive 'a' 'b' 'c'" '[[HOK_EXTRACT]]7z|a|b|||c|'
Assert-Marker "Expand-7zipArchive 'a' 'b' 'c' 'd'" '[[HOK_EXTRACT]]7z|a|b|||c d|'
Assert-Marker "Expand-7zipArchive 'a' 'b' -ExtractDir 'x'" '[[HOK_EXTRACT]]7z|a|b|x|||'
Assert-Marker "Expand-MsiArchive 'a' 'b' '-s'" '[[HOK_EXTRACT]]msi|a|b|||-s|'
Assert-Marker "Expand-ZipArchive 'a' 'b' 'c'" '[[HOK_EXTRACT]]7z|a|b|||c|'
Remove-Item $env:HOK_EXTRACT_FILE -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path $PSScriptRoot 'marker.txt') -Force -ErrorAction SilentlyContinue
$env:HOK_EXTRACT_FILE = $null
# `-Overwrite` is a validated set, like official (lib/decompress.ps1:77-79).
$ov = (Get-Command 'Expand-7zipArchive').Parameters['Overwrite'].Attributes |
    Where-Object { $_ -is [System.Management.Automation.ValidateSetAttribute] }
if (-not $ov -or ($ov.ValidValues -join ',') -ne 'All,Skip,Rename') {
    Write-Host "BAD:OverwriteValidateSet"; $script:fail = 1
} else {
    Write-Host 'PARAM-OK:Expand-7zipArchive:OverwriteValidateSet'
}
if ($fail) { exit 1 }
Write-Host 'ALIAS-OK'
"#;

    /// Create a small real zip archive containing `app/hello.txt` and
    /// `app/sub/deep.txt`.
    fn create_zip(path: &std::path::Path) {
        use std::io::Write;
        use zip::CompressionMethod;
        use zip::ZipWriter;
        use zip::write::FileOptions;

        let file = std::fs::File::create(path).unwrap();
        let mut zip = ZipWriter::new(file);
        let opts: FileOptions<'_, ()> =
            FileOptions::default().compression_method(CompressionMethod::Stored);
        zip.start_file("app/hello.txt", opts).unwrap();
        zip.write_all(b"Hello, World!").unwrap();
        zip.start_file("app/sub/deep.txt", opts).unwrap();
        zip.write_all(b"Deep content").unwrap();
        zip.finish().unwrap();
    }

    /// Regression guard for the deleted
    /// `function Expand-7ZipArchive { Expand-7zipArchive @args }`.
    ///
    /// PowerShell names are case-insensitive, so that line did not alias
    /// `Expand-7zipArchive` — it replaced it with a body calling itself, and
    /// every 7z-family script stage died with "call depth overflow". This test
    /// binds the names for real (dot-source + `Get-Command ….Definition`), so a
    /// case-variant collision introduced by any future edit fails it; a
    /// substring check on the embedded text would not.
    #[test]
    fn embedded_decompress_helpers_are_not_self_recursive() {
        if !pwsh_available() {
            return;
        }
        let root = crate::test_utils::tmpdir("decompress_alias_binding");
        std::fs::write(root.join("decompress.ps1"), DECOMPRESS_PS1).unwrap();
        std::fs::write(root.join("probe.ps1"), DECOMPRESS_PROBE_PS1).unwrap();

        let mut cmd = crate::internal::os::ps_command();
        cmd.arg("-File").arg(root.join("probe.ps1"));
        let out = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "decompress helper binding probe failed (exit {:?}):\n{stdout}\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(stdout.contains("ALIAS-OK"), "probe output:\n{stdout}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `Expand-7zipArchive -ExtractDir` must produce Scoop's layout: only the
    /// `ExtractDir` subtree is extracted, then hoisted into the destination.
    /// For an archive containing `app/hello.txt`, `-ExtractDir app` therefore
    /// yields `hello.txt` directly in the destination rather than
    /// `app/app/hello.txt` (the old pre-joined destination bug). Also covers
    /// `-Removal`: the source archive is gone afterwards.
    #[tokio::test]
    async fn expand_7zip_archive_extract_dir_matches_scoop_layout() {
        if !pwsh_available() {
            return;
        }
        let root = crate::test_utils::tmpdir("decompress_extract_dir_e2e");
        let session = crate::test_utils::test_session(&root);
        let pkg = crate::package::Package::from(
            "test-pkg",
            "main",
            crate::package::Manifest::from_json(
                "test-pkg",
                r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
            )
            .unwrap(),
        );
        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();
        create_zip(&working_dir.join("payload.zip"));

        run_script(
            &session,
            &pkg,
            &working_dir,
            None,
            "installer",
            "install",
            Some(vec![
                "Expand-7zipArchive \"$dir\\payload.zip\" \"$dir\" -ExtractDir 'app' -Removal",
            ]),
        )
        .await
        .unwrap();

        assert!(
            working_dir.join("hello.txt").exists(),
            "extract_dir subtree must be hoisted into the destination"
        );
        assert!(working_dir.join("sub").join("deep.txt").exists());
        assert!(
            !working_dir.join("app").exists(),
            "the extract_dir subtree must not remain nested under the destination"
        );
        assert!(
            !working_dir.join("payload.zip").exists(),
            "-Removal must delete the source archive after a successful extraction"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `7z.exe` availability for the `-Switches`/`-Overwrite` end-to-end tests
    /// (`HOK_7Z_EXE`, then Scoop's shim, then `PATH`).
    fn seven_zip_available() -> bool {
        if let Some(exe) = std::env::var_os("HOK_7Z_EXE") {
            return std::path::Path::new(&exe).is_file();
        }
        let scoop = std::env::var("SCOOP").unwrap_or_default();
        if std::path::Path::new(&scoop)
            .join("shims")
            .join("7z.exe")
            .is_file()
        {
            return true;
        }
        std::process::Command::new("7z.exe")
            .arg("i")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
    }

    /// `lessmsi.exe` availability for the `-Switches` MSI end-to-end test
    /// (Scoop's shim, then `PATH`). `false` skips the test, like
    /// [`seven_zip_available`] does when 7-Zip is absent.
    fn less_msi_available() -> bool {
        let scoop = std::env::var("SCOOP").unwrap_or_default();
        if std::path::Path::new(&scoop)
            .join("shims")
            .join("lessmsi.exe")
            .is_file()
        {
            return true;
        }
        std::process::Command::new("lessmsi.exe")
            .arg("h")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
    }

    /// A manifest package with a trivial body, for the script tests.
    fn test_pkg() -> crate::package::Package {
        crate::package::Package::from(
            "test-pkg",
            "main",
            crate::package::Manifest::from_json(
                "test-pkg",
                r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
            )
            .unwrap(),
        )
    }

    /// X5 end-to-end: a real package script's `-Switches` must survive the
    /// PS → marker → Rust round trip and change what lands on disk. `-xr!`
    /// removes a member the archive really contains, so the assertion cannot
    /// pass if the switch was dropped.
    #[tokio::test]
    async fn expand_7zip_archive_switches_take_effect() {
        if !pwsh_available() || !seven_zip_available() {
            eprintln!("skipping: needs pwsh + 7z.exe");
            return;
        }
        let root = crate::test_utils::tmpdir("decompress_switches_e2e");
        let session = crate::test_utils::test_session(&root);
        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();
        create_zip(&working_dir.join("payload.zip"));

        run_script(
            &session,
            &test_pkg(),
            &working_dir,
            None,
            "installer",
            "install",
            Some(vec![
                "Expand-7zipArchive \"$dir\\payload.zip\" \"$dir\" -Switches '-xr!deep.txt'",
            ]),
        )
        .await
        .unwrap();

        assert!(
            working_dir.join("app").join("hello.txt").exists(),
            "the non-excluded member must be extracted"
        );
        assert!(
            !working_dir
                .join("app")
                .join("sub")
                .join("deep.txt")
                .exists(),
            "-Switches '-xr!deep.txt' must keep deep.txt out of the tree"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// X5 end-to-end: `-Overwrite` must no longer be swallowed by
    /// `ValueFromRemainingArguments`. `Skip` keeps a pre-existing file
    /// (7-Zip `-aos`) while the rest of the archive still extracts.
    #[tokio::test]
    async fn expand_7zip_archive_overwrite_skip_is_honoured() {
        if !pwsh_available() || !seven_zip_available() {
            eprintln!("skipping: needs pwsh + 7z.exe");
            return;
        }
        let root = crate::test_utils::tmpdir("decompress_overwrite_e2e");
        let session = crate::test_utils::test_session(&root);
        let working_dir = root.join("work");
        std::fs::create_dir_all(working_dir.join("app")).unwrap();
        std::fs::write(working_dir.join("app").join("hello.txt"), b"STALE").unwrap();
        create_zip(&working_dir.join("payload.zip"));

        run_script(
            &session,
            &test_pkg(),
            &working_dir,
            None,
            "installer",
            "install",
            Some(vec![
                "Expand-7zipArchive \"$dir\\payload.zip\" \"$dir\" -Overwrite 'Skip'",
            ]),
        )
        .await
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(working_dir.join("app").join("hello.txt")).unwrap(),
            "STALE",
            "-Overwrite Skip must not replace the existing file"
        );
        assert!(
            working_dir
                .join("app")
                .join("sub")
                .join("deep.txt")
                .exists(),
            "the non-conflicting member must still be extracted"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// X5b end-to-end against a **real MSI**: `Expand-MsiArchive -Switches`
    /// must survive PS → marker → Rust and reach the `lessmsi.exe` command
    /// line. lessmsi treats a trailing bare argument as a file-name filter
    /// (`lessmsi <command> [options] <msi_name> [<path_to_extract\>]
    /// [file_names]`), so `-Switches 'other.txt'` is observable on disk: only
    /// `other.txt` comes out of an archive that also contains `hello.txt`.
    /// The no-switch control in the same test extracts both, so the assertion
    /// cannot pass if the switch was dropped — nor if the fixture stopped
    /// carrying both files.
    ///
    /// The fixture is generated by
    /// `tests/fixtures/msi/gen-fixture.ps1` (WindowsInstaller COM +
    /// in-box `makecab.exe`, no network); see the README next to it.
    #[tokio::test]
    async fn expand_msi_archive_switches_reach_lessmsi() {
        if !pwsh_available() || !less_msi_available() {
            eprintln!("skipping: needs pwsh + lessmsi.exe");
            return;
        }
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("msi")
            .join("x5b-fixture.msi");
        assert!(
            fixture.is_file(),
            "missing MSI fixture: {}",
            fixture.display()
        );

        let root = crate::test_utils::tmpdir("decompress_msi_switches_e2e");
        let session = crate::test_utils::test_session(&root);

        // Control: no `-Switches`, so lessmsi extracts everything.
        let full_dir = root.join("full");
        std::fs::create_dir_all(&full_dir).unwrap();
        std::fs::copy(&fixture, full_dir.join("x5b-fixture.msi")).unwrap();
        run_script(
            &session,
            &test_pkg(),
            &full_dir,
            None,
            "installer",
            "install",
            Some(vec!["Expand-MsiArchive \"$dir\\x5b-fixture.msi\" \"$dir\""]),
        )
        .await
        .unwrap();
        assert!(
            full_dir.join("X5bFixture").join("hello.txt").exists(),
            "the fixture must extract hello.txt when no -Switches is given"
        );
        assert!(
            full_dir.join("X5bFixture").join("other.txt").exists(),
            "the fixture must extract other.txt when no -Switches is given"
        );

        // `-Switches 'other.txt'` must reach lessmsi and filter hello.txt out.
        let filtered_dir = root.join("filtered");
        std::fs::create_dir_all(&filtered_dir).unwrap();
        std::fs::copy(&fixture, filtered_dir.join("x5b-fixture.msi")).unwrap();
        run_script(
            &session,
            &test_pkg(),
            &filtered_dir,
            None,
            "installer",
            "install",
            Some(vec![
                "Expand-MsiArchive \"$dir\\x5b-fixture.msi\" \"$dir\" -Switches 'other.txt'",
            ]),
        )
        .await
        .unwrap();
        assert!(
            filtered_dir.join("X5bFixture").join("other.txt").exists(),
            "the selected member must still be extracted"
        );
        assert!(
            !filtered_dir.join("X5bFixture").join("hello.txt").exists(),
            "-Switches 'other.txt' must reach lessmsi's file-name filter"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A call that omits `-DestinationPath` must extract next to the archive
    /// (official Scoop default `(Split-Path $Path)`), not relative to hok's
    /// own current directory. `run_script` does not set a `current_dir` on the
    /// child, so an empty destination would fall through to the process CWD.
    #[tokio::test]
    async fn expand_7zip_archive_defaults_destination_to_archive_dir() {
        if !pwsh_available() {
            return;
        }
        let root = crate::test_utils::tmpdir("decompress_default_dest");
        let session = crate::test_utils::test_session(&root);
        let pkg = crate::package::Package::from(
            "test-pkg",
            "main",
            crate::package::Manifest::from_json(
                "test-pkg",
                r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
            )
            .unwrap(),
        );
        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();
        create_zip(&working_dir.join("payload.zip"));

        // Record whether the host CWD already holds the leaked tree, so the
        // assertion detects leakage without depending on the machine's
        // starting state. An empty destination makes the archive's own members
        // (`app/hello.txt`) land relative to the CWD, so probe that path.
        let cwd_leak = std::env::current_dir()
            .unwrap()
            .join("app")
            .join("hello.txt");
        let leaked_before = cwd_leak.exists();

        run_script(
            &session,
            &pkg,
            &working_dir,
            None,
            "installer",
            "install",
            Some(vec!["Expand-7zipArchive -Path \"$dir\\payload.zip\""]),
        )
        .await
        .unwrap();

        // No extract_dir, so the whole archive lands in the archive's dir.
        assert!(
            working_dir.join("app").join("hello.txt").exists(),
            "an omitted -DestinationPath must default to the archive's directory"
        );
        assert!(
            working_dir
                .join("app")
                .join("sub")
                .join("deep.txt")
                .exists()
        );
        assert_eq!(
            cwd_leak.exists(),
            leaked_before,
            "extraction must not leak into the process CWD: {}",
            cwd_leak.display()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A script's stdout (here a Chinese Write-Host line) is captured and
    /// re-emitted as ScriptStdout events, so the frontend renders it with
    /// the same indentation as its own lines instead of a bare column-0 print.
    /// Also guards the UTF-8 preamble setting so non-ASCII survives the pipe.
    #[tokio::test]
    async fn run_script_forwards_script_output() {
        let root = crate::test_utils::tmpdir("script_output_forward");
        let session = crate::test_utils::test_session(&root);
        let bus = session.event_bus();
        let mut rx = bus.receiver().expect("fresh event bus receiver");

        let pkg = crate::package::Package::from(
            "test-pkg",
            "main",
            crate::package::Manifest::from_json(
                "test-pkg",
                r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
            )
            .unwrap(),
        );
        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        let script = Some(vec!["Write-Host 'hello-世界'", "Write-Host 'done-ok'"]);
        run_script(
            &session,
            &pkg,
            &working_dir,
            None,
            "install",
            "install",
            script,
        )
        .await
        .unwrap();

        let mut lines = vec![];
        while let Ok(ev) = rx.try_recv() {
            if let Event::ScriptStdout(line) = ev {
                lines.push(line);
            }
        }
        assert!(
            lines.iter().any(|l| l.contains("hello-世界")),
            "expected UTF-8 script output, got {:?}",
            lines
        );
        assert!(lines.iter().any(|l| l.contains("done-ok")));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The `post_install` stage inherits the manifest's `env_set` variables
    /// and `env_add_path` directories (mirroring Scoop's env hooks running
    /// right before post_install, install.ps1:62-63,69) — so a script like
    /// initdb reading `$env:PGDATA` gets the data dir. Mirrors the
    /// postgresql manifest's post_install initdb call.
    #[tokio::test]
    async fn post_install_inherits_env_set_and_add_path() {
        let root = crate::test_utils::tmpdir("script_post_install_env");
        let session = crate::test_utils::test_session(&root);
        let working_dir = root.join("apps").join("test-pkg").join("1.0.0");
        std::fs::create_dir_all(&working_dir).unwrap();

        let manifest = crate::package::Manifest::from_json(
            "test-pkg",
            r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT",
                "env_set": {"PGDATA_TEST": "$dir\\data"},
                "env_add_path": ["bin"]}"#,
        )
        .unwrap();
        let pkg = crate::package::Package::from("test-pkg", "main", manifest);

        let bus = session.event_bus();
        let mut rx = bus.receiver().expect("fresh event bus receiver");
        run_script(
            &session,
            &pkg,
            &working_dir,
            None,
            "post_install",
            "install",
            Some(vec![
                "Write-Host \"PGDATA=$env:PGDATA_TEST\"",
                "Write-Host \"PATH=$env:PATH\"",
            ]),
        )
        .await
        .unwrap();

        let mut lines = vec![];
        while let Ok(ev) = rx.try_recv() {
            if let Event::ScriptStdout(line) = ev {
                lines.push(line);
            }
        }
        let joined = lines.join("\n");
        assert!(
            joined.contains(&format!("PGDATA={}\\data", working_dir.to_string_lossy())),
            "post_install must inherit env_set PGDATA_TEST, got: {joined}"
        );
        let bin = internal::path::normalize_path(working_dir.join("bin"))
            .to_string_lossy()
            .into_owned();
        assert!(
            joined.contains(&bin),
            "post_install PATH must contain the env_add_path bin dir, got: {joined}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Stages that run *before* Scoop's env hooks (`pre_install`) must NOT see
    /// `env_set` variables — only post_install gets the install-time env.
    #[tokio::test]
    async fn pre_install_does_not_inherit_env_set() {
        let root = crate::test_utils::tmpdir("script_pre_install_env");
        let session = crate::test_utils::test_session(&root);
        let working_dir = root.join("apps").join("test-pkg").join("1.0.0");
        std::fs::create_dir_all(&working_dir).unwrap();

        let manifest = crate::package::Manifest::from_json(
            "test-pkg",
            r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT",
                "env_set": {"PGDATA_TEST": "$dir\\data"}}"#,
        )
        .unwrap();
        let pkg = crate::package::Package::from("test-pkg", "main", manifest);

        let bus = session.event_bus();
        let mut rx = bus.receiver().expect("fresh event bus receiver");
        run_script(
            &session,
            &pkg,
            &working_dir,
            None,
            "pre_install",
            "install",
            Some(vec!["Write-Host \"PGDATA=[$env:PGDATA_TEST]\""]),
        )
        .await
        .unwrap();

        let mut lines = vec![];
        while let Ok(ev) = rx.try_recv() {
            if let Event::ScriptStdout(line) = ev {
                lines.push(line);
            }
        }
        assert!(
            lines.iter().any(|l| l.contains("PGDATA=[]")),
            "pre_install must not inherit env_set, got: {:?}",
            lines
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// In-flight cancellation: with `script_engine` on, cancelling the token
    /// *while* a script runs must kill the engine worker (the only way to
    /// interrupt an in-process script) and make run_script return promptly.
    #[tokio::test]
    async fn engine_path_cancel_mid_run_kills_worker() {
        if !pwsh_available() {
            return;
        }
        let (session, root) = engine_session("script_engine_cancel_mid");
        let token = tokio_util::sync::CancellationToken::new();
        session.set_cancellation_token(token.clone()).unwrap();

        let pkg = crate::package::Package::from(
            "test-pkg",
            "main",
            crate::package::Manifest::from_json(
                "test-pkg",
                r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
            )
            .unwrap(),
        );
        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        // Fire the token shortly after the long script begins running.
        let cancel_token = token.clone();
        let canceller = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            cancel_token.cancel();
        });

        let start = std::time::Instant::now();
        let err = run_script(
            &session,
            &pkg,
            &working_dir,
            None,
            "install",
            "install",
            Some(vec!["Start-Sleep -Seconds 300"]),
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("cancelled"),
            "expected a cancellation error, got: {err}"
        );
        // Aborts well before the 300s script would finish on its own.
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "engine cancel must abort mid-run promptly, took {:?}",
            start.elapsed()
        );
        let _ = canceller.await;

        let _ = std::fs::remove_dir_all(&root);
    }
    /// in a throwaway temp dir (config written with the flag so the resident
    /// engine path is selected).
    fn engine_session(name: &str) -> (crate::Session, std::path::PathBuf) {
        let root = crate::test_utils::tmpdir(name);
        let config_path = root.join("hok.json");
        let root_escaped = root.to_string_lossy().replace('\\', "\\\\");
        let cache_escaped = root.join("cache").to_string_lossy().replace('\\', "\\\\");
        std::fs::write(
            &config_path,
            format!(
                r#"{{"root_path": "{}", "cache_path": "{}", "script_engine": true}}"#,
                root_escaped, cache_escaped
            ),
        )
        .unwrap();
        (crate::Session::new_with(&config_path).unwrap(), root)
    }

    fn pwsh_available() -> bool {
        crate::internal::os::is_pwsh_available()
            || std::process::Command::new("powershell.exe")
                .arg("-NoProfile")
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
    }

    /// With `script_engine` on, `run_script` runs the stage through the
    /// resident engine; its captured output is forwarded as ScriptStdout
    /// events (same frontend contract as the fresh-spawn path).
    #[tokio::test]
    async fn engine_path_forwards_script_output() {
        if !pwsh_available() {
            return;
        }
        let (session, root) = engine_session("script_engine_output");
        let bus = session.event_bus();
        let mut rx = bus.receiver().expect("fresh event bus receiver");

        let pkg = crate::package::Package::from(
            "test-pkg",
            "main",
            crate::package::Manifest::from_json(
                "test-pkg",
                r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
            )
            .unwrap(),
        );
        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        run_script(
            &session,
            &pkg,
            &working_dir,
            None,
            "install",
            "install",
            Some(vec!["Write-Host 'engine-世界'", "Write-Host 'engine-done'"]),
        )
        .await
        .unwrap();

        let mut lines = vec![];
        while let Ok(ev) = rx.try_recv() {
            if let Event::ScriptStdout(line) = ev {
                lines.push(line);
            }
        }
        assert!(
            lines.iter().any(|l| l.contains("engine-世界")),
            "engine path must forward UTF-8 script output, got {:?}",
            lines
        );
        assert!(lines.iter().any(|l| l.contains("engine-done")));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// On the engine path, each PowerShell stream is routed to its own event
    /// (the worker's `O:`/`W:`/`E:` tags are stripped and classified), so the
    /// frontend can render stdout / warning / stderr with distinct colours.
    #[tokio::test]
    async fn engine_path_routes_stdout_warning_stderr() {
        if !pwsh_available() {
            return;
        }
        let (session, root) = engine_session("script_engine_stream_split");
        let bus = session.event_bus();
        let mut rx = bus.receiver().expect("fresh event bus receiver");

        let pkg = crate::package::Package::from(
            "test-pkg",
            "main",
            crate::package::Manifest::from_json(
                "test-pkg",
                r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
            )
            .unwrap(),
        );
        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        // Write-Error / Write-Warning are non-terminating, so the run itself
        // succeeds; only their routing is under test.
        let _ = run_script(
            &session,
            &pkg,
            &working_dir,
            None,
            "install",
            "install",
            Some(vec![
                "Write-Output 'stdout-x'",
                "Write-Warning 'warn-y'",
                "Write-Error 'err-z'",
            ]),
        )
        .await;

        let mut out = vec![];
        let mut warns = vec![];
        let mut errs = vec![];
        while let Ok(ev) = rx.try_recv() {
            match ev {
                Event::ScriptStdout(l) => out.push(l),
                Event::ScriptWarning(l) => warns.push(l),
                Event::ScriptStderr(l) => errs.push(l),
                _ => {}
            }
        }
        assert!(
            out.iter().any(|l| l.contains("stdout-x")),
            "stdout must route to ScriptStdout, got: {out:?} | {warns:?} | {errs:?}"
        );
        assert!(
            warns.iter().any(|l| l.contains("warn-y")),
            "warning must route to ScriptWarning, got: {out:?} | {warns:?} | {errs:?}"
        );
        assert!(
            errs.iter().any(|l| l.contains("err-z")),
            "error must route to ScriptStderr, got: {out:?} | {warns:?} | {errs:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The fresh-spawn path maps its real stderr pipe onto `ScriptStderr`, so a
    /// script's `Write-Error` is distinguishable from normal stdout output.
    #[tokio::test]
    async fn spawn_path_routes_stderr() {
        if !pwsh_available() {
            return;
        }
        let root = crate::test_utils::tmpdir("script_spawn_stderr");
        let session = crate::test_utils::test_session(&root);
        let bus = session.event_bus();
        let mut rx = bus.receiver().expect("fresh event bus receiver");

        let pkg = crate::package::Package::from(
            "test-pkg",
            "main",
            crate::package::Manifest::from_json(
                "test-pkg",
                r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
            )
            .unwrap(),
        );
        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        // Non-terminating Write-Error → stderr pipe; the run still succeeds.
        let _ = run_script(
            &session,
            &pkg,
            &working_dir,
            None,
            "install",
            "install",
            Some(vec![
                "Write-Output 'spawn-out'",
                "Write-Error 'spawn-err-line'",
            ]),
        )
        .await;

        let mut out = vec![];
        let mut errs = vec![];
        while let Ok(ev) = rx.try_recv() {
            match ev {
                Event::ScriptStdout(l) => out.push(l),
                Event::ScriptStderr(l) => errs.push(l),
                _ => {}
            }
        }
        assert!(
            out.iter().any(|l| l.contains("spawn-out")),
            "stdout must route to ScriptStdout, got: {out:?} | {errs:?}"
        );
        assert!(
            errs.iter().any(|l| l.contains("spawn-err-line")),
            "stderr must route to ScriptStderr, got: {out:?} | {errs:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A package script that throws must fail the stage on the engine path and
    /// surface the *real* reason in the returned error (not be swallowed by the
    /// preamble's missing-helper trap). Guards the trap fix end to end.
    #[tokio::test]
    async fn engine_path_throw_fails_stage_with_reason() {
        if !pwsh_available() {
            return;
        }
        let (session, root) = engine_session("script_engine_throw");
        let pkg = crate::package::Package::from(
            "test-pkg",
            "main",
            crate::package::Manifest::from_json(
                "test-pkg",
                r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
            )
            .unwrap(),
        );
        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        let err = run_script(
            &session,
            &pkg,
            &working_dir,
            None,
            "install",
            "install",
            Some(vec!["throw 'boom-real-msg'"]),
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("boom-real-msg"),
            "the real throw reason must surface, got: {err}"
        );
        assert!(
            err.to_string().contains("failed"),
            "expected a failed-with-reason error, got: {err}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Same trap fix on the fresh-spawn path: a throw makes `pwsh -File` exit
    /// nonzero, so the stage reports an exit-code failure instead of silently
    /// succeeding.
    #[tokio::test]
    async fn spawn_path_throw_fails_stage() {
        if !pwsh_available() {
            return;
        }
        let root = crate::test_utils::tmpdir("script_spawn_throw");
        let session = crate::test_utils::test_session(&root);
        let pkg = crate::package::Package::from(
            "test-pkg",
            "main",
            crate::package::Manifest::from_json(
                "test-pkg",
                r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
            )
            .unwrap(),
        );
        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        let err = run_script(
            &session,
            &pkg,
            &working_dir,
            None,
            "install",
            "install",
            Some(vec!["throw 'boom'"]),
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("exited with code"),
            "a throw must fail the spawn stage via a nonzero exit, got: {err}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// True streaming on the engine path: a script that writes a first line
    /// then sleeps must deliver that line to the event bus *while the run is
    /// still in flight* (well before the sleep finishes), not batched until
    /// completion. Guards against regressing to the old capture-file model.
    #[tokio::test]
    async fn engine_path_streams_output_live() {
        if !pwsh_available() {
            return;
        }
        let (session, root) = engine_session("script_engine_stream_live");
        let bus = session.event_bus();
        let mut rx = bus.receiver().expect("fresh event bus receiver");

        let pkg = crate::package::Package::from(
            "test-pkg",
            "main",
            crate::package::Manifest::from_json(
                "test-pkg",
                r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
            )
            .unwrap(),
        );
        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        // Background bus watcher: timestamp when 'live-first' and 'live-last'
        // arrive on the event bus, stopping once 'live-last' (the script's end)
        // is seen. Only holds the receiver, so it is safe to spawn.
        let watcher = tokio::spawn(async move {
            let mut first: Option<std::time::Instant> = None;
            let mut last: Option<std::time::Instant> = None;
            while let Some(ev) = rx.recv().await {
                if let Event::ScriptStdout(l) = ev {
                    let now = std::time::Instant::now();
                    if l.contains("live-first-世界") && first.is_none() {
                        first = Some(now);
                    }
                    if l.contains("live-last") {
                        last = Some(now);
                        break;
                    }
                }
            }
            (first, last)
        });

        // A long tail after 'first' lets the watcher observe 'first' arriving
        // on the bus while the script is still sleeping.
        run_script(
            &session,
            &pkg,
            &working_dir,
            None,
            "install",
            "install",
            Some(vec![
                "Write-Output 'live-first-世界'",
                "Start-Sleep -Milliseconds 2500",
                "Write-Output 'live-last'",
            ]),
        )
        .await
        .unwrap();

        let (first, last) = watcher.await.unwrap();
        let first = first.expect("live-first should stream to the bus");
        let last = last.expect("live-last should stream to the bus");
        let gap = last.duration_since(first);
        assert!(
            gap >= std::time::Duration::from_millis(1500),
            "expected ~2.5s gap between live-first and live-last if streamed live, \
             got {gap:?} (a batch-at-end engine would deliver both ~simultaneously)"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// script by killing the worker: run_script returns a cancellation error
    /// and the (dead) worker is not left lingering behind a next stage.
    #[tokio::test]
    async fn engine_path_cancel_kills_in_flight_script() {
        if !pwsh_available() {
            return;
        }
        let (session, root) = engine_session("script_engine_cancel");
        let token = tokio_util::sync::CancellationToken::new();
        session.set_cancellation_token(token.clone()).unwrap();
        // Cancel *before* the run starts; the engine must observe it at the
        // submission boundary and abort (killing the worker) instead of
        // running the script.
        token.cancel();

        let pkg = crate::package::Package::from(
            "test-pkg",
            "main",
            crate::package::Manifest::from_json(
                "test-pkg",
                r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
            )
            .unwrap(),
        );
        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        let err = run_script(
            &session,
            &pkg,
            &working_dir,
            None,
            "install",
            "install",
            Some(vec!["Start-Sleep -Seconds 300"]),
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("cancelled"),
            "expected a cancellation error, got: {err}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Test that TempFileGuard removes files on Drop.
    #[test]
    fn test_temp_file_guard_cleanup() {
        let dir = crate::test_utils::tmpdir("temp_file_guard");
        let path1 = dir.join("test1.txt");
        let path2 = dir.join("test2.txt");

        std::fs::write(&path1, b"hello").unwrap();
        std::fs::write(&path2, b"world").unwrap();
        assert!(path1.exists());
        assert!(path2.exists());

        {
            let _guard = TempFileGuard::new(vec![path1.clone(), path2.clone()]);
            // Guard is alive — files should still exist
            assert!(path1.exists());
            assert!(path2.exists());
        }
        // Guard dropped — files should be removed
        assert!(!path1.exists());
        assert!(!path2.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Helper to create a test environment for expand_scoop_vars tests.
    /// Cleans up the temp dir on drop via the returned guard.
    struct TestDirGuard(std::path::PathBuf);
    impl Drop for TestDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn setup_expand_scoop_vars_test(
        test_name: &str,
    ) -> (crate::Session, Package, std::path::PathBuf, TestDirGuard) {
        let tmp = crate::test_utils::tmpdir(&format!("expand_scoop_vars_{}", test_name));
        let guard = TestDirGuard(tmp.clone());
        let root = &tmp;

        // Write minimal hok config
        let config_path = root.join("hok.json");
        let root_escaped = root.to_string_lossy().replace('\\', "\\\\");
        std::fs::write(
            &config_path,
            format!(r#"{{"root_path": "{}"}}"#, root_escaped),
        )
        .unwrap();

        let session = crate::Session::new_with(&config_path).unwrap();
        let manifest = crate::package::Manifest::from_json(
            "test-pkg",
            r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
        )
        .unwrap();
        let pkg = Package::from("test-pkg", "test-bucket", manifest);
        let working_dir = root.join("apps").join("test-pkg").join("1.0.0");

        (session, pkg, working_dir, guard)
    }

    /// Test that $dir is expanded to the working directory path.
    #[test]
    fn test_expand_dir_var() {
        let (session, pkg, working_dir, _tmp) = setup_expand_scoop_vars_test("dir");
        let args = vec!["/DIR=\"$dir\""];
        let expanded = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded.len(), 1);
        assert_eq!(
            expanded[0],
            format!("/DIR=\"{}\"", working_dir.to_string_lossy())
        );
    }

    /// Test that $scoopdir is expanded to the Scoop root path.
    #[test]
    fn test_expand_scoopdir_var() {
        let (session, pkg, working_dir, tmp) = setup_expand_scoop_vars_test("scoopdir");
        let root = &tmp.0;
        let args = vec!["$scoopdir"];
        let expanded = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0], root.to_string_lossy().to_string());
    }

    /// Test that $persist_dir is expanded correctly.
    #[test]
    fn test_expand_persist_dir_var() {
        let (session, pkg, working_dir, tmp) = setup_expand_scoop_vars_test("persist");
        let root = &tmp.0;
        let expected = root.join("persist").join("test-pkg");
        let args = vec!["$persist_dir"];
        let expanded = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0], expected.to_string_lossy().to_string());
    }

    /// Test that $version, $app, and $bucket are expanded.
    #[test]
    fn test_expand_identity_vars() {
        let (session, pkg, working_dir, _tmp) = setup_expand_scoop_vars_test("identity");
        let args = vec!["$version", "$app", "$bucket"];
        let expanded = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded[0], "1.0.0");
        assert_eq!(expanded[1], "test-pkg");
        assert_eq!(expanded[2], "test-bucket");
    }

    /// An isolated (URL / local-path) install must expose an **empty** `$bucket`,
    /// not the internal `__isolated__` sentinel: upstream reads `$bucket` from
    /// `install.json`, which carries no bucket for such installs. Both the
    /// PowerShell-preamble env and the `expand_scoop_vars` string path must agree.
    #[test]
    fn test_isolated_package_bucket_is_empty() {
        let (session, _pkg, working_dir, _tmp) = setup_expand_scoop_vars_test("isolated_bucket");
        let manifest = crate::package::Manifest::from_json(
            "iso-pkg",
            r#"{"version": "1.0.0", "homepage": "https://example.com"}"#,
        )
        .unwrap();
        let pkg = Package::from(
            "iso-pkg",
            crate::constant::ISOLATED_PACKAGE_BUCKET,
            manifest,
        );

        assert_eq!(script_bucket(&pkg), "");

        let expanded = expand_scoop_vars(&["$bucket"], &session, &pkg, &working_dir, "uninstall");
        assert_eq!(expanded[0], "");
    }

    /// Test that $cmd is expanded to "install" or "uninstall" accordingly.
    #[test]
    fn test_expand_cmd_var() {
        let (session, pkg, working_dir, _tmp) = setup_expand_scoop_vars_test("cmd");
        let args = vec!["$cmd"];
        let expanded_install = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        let expanded_uninstall =
            expand_scoop_vars(&args, &session, &pkg, &working_dir, "uninstall");
        assert_eq!(expanded_install[0], "install");
        assert_eq!(expanded_uninstall[0], "uninstall");
    }

    /// `$fname` resolves to the remote filename of the first download URL.
    #[test]
    fn test_first_download_fname() {
        // plain URL → remote filename
        assert_eq!(
            first_download_fname(&[
                "https://github.com/zed-industries/zed/releases/download/v1.15.0/Zed-x86_64.exe"
            ]),
            "Zed-x86_64.exe"
        );
        // Scoop `#/rename.ext` fragment wins (copy_downloaded_files uses the
        // same stripped name)
        assert_eq!(
            first_download_fname(&["https://example.com/installer.bin#/setup.exe"]),
            "setup.exe"
        );
        // query stripped
        assert_eq!(
            first_download_fname(&["https://example.com/app.exe?download=1"]),
            "app.exe"
        );
        // no URLs → empty
        let none: &[&str] = &[];
        assert_eq!(first_download_fname(none), "");
    }

    /// Test that all variables together in a realistic installer arg string are expanded.
    #[test]
    fn test_expand_all_vars_in_args() {
        let (session, pkg, working_dir, tmp) = setup_expand_scoop_vars_test("all_vars");
        let root = &tmp.0;
        let args = vec![
            "/VERYSILENT",
            "/DIR=\"$dir\"",
            "/D=\"$persist_dir\"",
            "/SCOOP=\"$scoopdir\"",
        ];
        let expanded = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded.len(), 4);
        assert_eq!(expanded[0], "/VERYSILENT");
        assert_eq!(
            expanded[1],
            format!("/DIR=\"{}\"", working_dir.to_string_lossy())
        );
        assert_eq!(
            expanded[2],
            format!(
                "/D=\"{}\"",
                root.join("persist").join("test-pkg").to_string_lossy()
            )
        );
        assert_eq!(
            expanded[3],
            format!("/SCOOP=\"{}\"", root.to_string_lossy())
        );
    }

    /// Test that $original_dir produces the same value as $dir.
    #[test]
    fn test_expand_original_dir_var() {
        let (session, pkg, working_dir, _tmp) = setup_expand_scoop_vars_test("original_dir");
        let args = vec!["$original_dir"];
        let expanded = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded[0], working_dir.to_string_lossy().to_string());
    }

    /// Test that $architecture is expanded (not empty, one of the known values).
    #[test]
    fn test_expand_architecture_var() {
        let (session, pkg, working_dir, _tmp) = setup_expand_scoop_vars_test("arch");
        let args = vec!["$architecture"];
        let expanded = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        assert!(
            expanded[0] == "64bit" || expanded[0] == "32bit" || expanded[0] == "arm64",
            "expected 64bit/32bit/arm64, got {}",
            expanded[0]
        );
    }

    /// Test that variable names inside longer words do NOT get replaced (false positive).
    #[test]
    fn test_expand_no_false_positive() {
        let (session, pkg, working_dir, _tmp) = setup_expand_scoop_vars_test("false_positive");
        // "directory" contains "dir" but should not be replaced
        let args = vec!["directory", "scoopdirectory"];
        let expanded = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded[0], "directory");
        assert_eq!(expanded[1], "scoopdirectory");
    }

    /// Notes-style expansion: a post-install note expands the full Scoop
    /// variable set (mirror Scoop's `show_notes` → `substitute`, which
    /// replaces all params, not just the three path vars).
    #[test]
    fn test_expand_note_style() {
        let (session, pkg, working_dir, tmp) = setup_expand_scoop_vars_test("note");
        let root = &tmp.0;
        let note =
            "Run \"$dir\\Setup-UVEnv.ps1 $persist_dir\" v$version ($app from $bucket, $scoopdir)";
        let expanded = expand_scoop_str(note, &session, &pkg, &working_dir, "install");
        assert_eq!(
            expanded,
            format!(
                "Run \"{}\\Setup-UVEnv.ps1 {}\" v1.0.0 (test-pkg from test-bucket, {})",
                working_dir.to_string_lossy(),
                root.join("persist").join("test-pkg").to_string_lossy(),
                root.to_string_lossy()
            )
        );
    }
}
