//! Inspect the manifest of a package.

use clap::Parser;
use libscoop::{Session, fs, os, package};
use std::io::Write;
use std::process::Stdio;

use crate::{Result, output};

/// Inspect the manifest of a package
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// Name of the package to be inspected
    package: String,
}

/// Pretty-print a resolved manifest as JSON, rendering from the in-memory
/// content so it also works when the manifest is served by the (SQLite)
/// manifest cache or came from a URL.
fn pretty_manifest(resolved: &package::GetManifest) -> Result<String> {
    let manifest = &resolved.manifest;
    // Prefer the retained raw text; else read the on-disk manifest file; else
    // (SQLite manifest cache / URL - no raw text, virtual path) render the
    // parsed spec so the content is always shown.
    let content: String = if let Some(raw) = manifest.raw() {
        raw.to_string()
    } else if manifest.path().is_file() {
        fs::read_to_string(manifest.path())?
    } else {
        manifest
            .to_pretty_json()
            .ok_or_else(|| anyhow::anyhow!(hok_i18n::t!("cmd.cat_render_failed")))?
    };
    match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(value) => Ok(serde_json::to_string_pretty(&value).unwrap_or(content)),
        Err(_) => Ok(content),
    }
}

/// Pipe a JSON string through bat for syntax highlighting; the child's exit
/// status is what tells the caller whether the manifest reached the user.
///
/// `stdout` stays inherited — bat writes the highlighted manifest straight to
/// the terminal — while `stderr` is captured, so a rejected `cat_style` can be
/// reported as hok's own warning naming the setting instead of as a bare bat
/// parser error with no hint of where the value came from.
fn run_bat(json: &str, style: &str) -> std::io::Result<std::process::Output> {
    let mut child = super::bat_command(style)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        // A style bat rejects makes it exit while parsing its arguments, before
        // it reads anything, so the pipe may already be gone. That is not a
        // failure of ours: the exit status below carries the diagnosis.
        let _ = stdin.write_all(json.as_bytes());
    }
    child.wait_with_output()
}

/// bat's own one-line diagnosis of a failed run.
///
/// A silent failure (a bat that exited without saying anything) still has to
/// produce a reason, or the warning would trail off after its colon.
fn bat_diagnosis(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        return stderr.to_string();
    }
    match output.status.code() {
        Some(code) => format!("exit code {code}"),
        None => "terminated".to_string(),
    }
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let query = args.package;

    // Source resolution (URL / UNC / local file / installed record / bucket)
    // is `Get-Manifest`, shared with `home`; the picker only kicks in for a
    // bare, uninstalled name held by several buckets.
    let resolved = super::resolve_manifest_query(
        session,
        &query,
        |count| hok_i18n::t!("cmd.found", count = count, query = query).to_string(),
        hok_i18n::t!("cmd.cat_not_found", query = query).to_string(),
        hok_i18n::t!("cmd.cat_invalid").to_string(),
    )
    .await?;

    let json = pretty_manifest(&resolved)?;

    // Only delegate to bat when a style is configured and bat is on PATH;
    // otherwise (or when bat fails) print the plain pretty JSON so the
    // manifest content is always shown.
    let config = session.config();
    let style = config.cat_style();
    if !style.is_empty() && os::is_program_available("bat.exe") {
        match run_bat(&json, style) {
            // bat rendered it: the manifest is already on the terminal.
            Ok(output) if output.status.success() => return Ok(()),
            // bat ran but rejected the invocation — `cat_style` is not a value
            // its `--style` accepts. Name the setting (bat's message alone does
            // not) and fall through to the plain JSON rather than ending the
            // command with an empty screen.
            Ok(output) => output::warn(hok_i18n::t!(
                "cmd.cat_bat_failed",
                style = style,
                reason = bat_diagnosis(&output)
            )),
            // On PATH but unusable (a broken binary, a directory placeholder).
            Err(err) => output::warn(hok_i18n::t!(
                "cmd.cat_bat_failed",
                style = style,
                reason = err.to_string()
            )),
        }
    }

    println!("{json}");
    Ok(())
}
