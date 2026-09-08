//! PowerShell script execution & installer-variable expansion primitives.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::Stdio;
use tokio::sync::mpsc;
use tracing::debug;

use super::psengine::PsEngine;
use crate::package::Package;
use crate::{Error, Event, Session, error::Fallible, internal};

/// Read a captured child pipe line by line (lossy UTF-8) and forward every
/// non-empty line to the event bus as `ScriptOutput`, so the frontend renders
/// script output with the same indentation as its own lines. Runs on a worker
/// thread (see `run_script`) so a chatty script cannot deadlock the child
/// against the wait loop on a full pipe buffer.
fn forward_script_pipe(mut reader: BufReader<impl Read>, tx: Option<mpsc::Sender<Event>>) {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => {
                let cow = String::from_utf8_lossy(&buf);
                let line = cow
                    .trim_start_matches('\u{feff}')
                    .trim_end_matches(['\r', '\n']);
                if line.is_empty() {
                    continue;
                }
                if let Some(tx) = tx.as_ref() {
                    let _ = tx.try_send(Event::ScriptOutput(line.to_owned()));
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
/// Execution path is gated by the `script-engine` config (default off):
/// - **off** (default): each stage spawns a fresh `pwsh -File`, piping output
///   to `ScriptOutput` events with prompt Ctrl-C cancellation.
/// - **on**: the stage runs through the session's resident [PsEngine], reusing
///   one long-lived pwsh worker across stages; output is captured and
///   re-emitted as `ScriptOutput`, and Ctrl-C kills the worker (respawned on
///   the next stage).
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
    const CORE_PS1: &str = include_str!("../../../../../asset_scripts/core.ps1");
    const DECOMPRESS_PS1: &str = include_str!("../../../../../asset_scripts/decompress.ps1");
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

# Notify hok about undefined commands (missing helpers)
trap {{
    if ($_.Exception -is [System.Management.Automation.CommandNotFoundException]) {{
        Write-Host "[[HOK_MISSING_HELPER]]$($_.Exception.CommandName)"
    }}
    continue
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
    env.insert("SCOOP_PACKAGE_BUCKET".into(), package.bucket().to_owned());
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
/// Pipes stdout/stderr and forwards every non-empty line as a `ScriptOutput`
/// event so the frontend renders script output with its own indentation.
/// Cooperative Ctrl-C kills the child (a lone powershell lingers otherwise);
/// no hard timeout — Scoop scripts may legitimately run for minutes.
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
    // `ScriptOutput` events — a bare inherited stdout would print at column 0
    // and break the frontend's indented alignment.
    ps.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = ps
        .spawn()
        .map_err(|e| ps_launch_error(stage, package.name(), e))?;

    // Drain the pipes on worker threads while the child runs; without this a
    // chatty script could fill a pipe buffer and deadlock the child against
    // our wait loop. Every non-empty line is forwarded as `ScriptOutput`.
    let emitter = session.emitter();
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out_reader = if let Some(out) = stdout.take() {
        let tx = emitter.clone();
        Some(std::thread::spawn(move || {
            forward_script_pipe(BufReader::new(out), tx)
        }))
    } else {
        None
    };
    let err_reader = if let Some(err) = stderr.take() {
        let tx = emitter.clone();
        Some(std::thread::spawn(move || {
            forward_script_pipe(BufReader::new(err), tx)
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

    // Child finished — join the readers so every captured `ScriptOutput` line
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
/// across stages/packages. Captured output lines (the engine captures all
/// streams to a per-submission file) are forwarded as `ScriptOutput` events
/// after the submission finishes. Cancellation observed mid-run kills the
/// worker (terminating the in-flight script); the engine respawns on its next
/// use, mirroring the fresh-spawn path's prompt abort.
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

    let outcome =
        tokio::task::spawn_blocking(move || -> Fallible<super::psengine::PsRunOutcome> {
            let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
            // Lazily spawn the engine on first use; a killed/crashed worker is
            // transparently respawned by the engine itself on its next call.
            let mut engine = match guard.take() {
                Some(e) => e,
                None => PsEngine::new().map_err(|e| ps_launch_error(&stage_in, &name_in, e))?,
            };
            let res = engine
                .run_file_captured_cancelable(&path, &env, &token)
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

    // Forward the captured output as ScriptOutput events (after completion —
    // a resident engine captures to a file rather than streaming live).
    let emitter = session.emitter();
    if let Some(tx) = emitter.as_ref() {
        for line in &outcome.output {
            let _ = tx.try_send(Event::ScriptOutput(line.clone()));
        }
    }

    if outcome.cancelled {
        return Err(Error::Custom(format!(
            "{} script for '{}' cancelled",
            stage,
            package.name()
        )));
    }
    if outcome.code != 0 {
        return Err(Error::Custom(format!(
            "{} script for '{}' exited with code {}",
            stage,
            package.name(),
            outcome.code
        )));
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
    let bucket = pkg.bucket();

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
        .env("SCOOP_PACKAGE_BUCKET", package.bucket())
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

    /// A script's stdout (here a Chinese Write-Host line) is captured and
    /// re-emitted as ScriptOutput events, so the frontend renders it with
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
            if let Event::ScriptOutput(line) = ev {
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
            if let Event::ScriptOutput(line) = ev {
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
            if let Event::ScriptOutput(line) = ev {
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

    /// In-flight cancellation: with `script-engine` on, cancelling the token
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
                r#"{{"root_path": "{}", "cache_path": "{}", "script-engine": true}}"#,
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

    /// With `script-engine` on, `run_script` runs the stage through the
    /// resident engine; its captured output is forwarded as ScriptOutput
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
            if let Event::ScriptOutput(line) = ev {
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

    /// With `script-engine` on, a cancelled token aborts a *running* engine
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
