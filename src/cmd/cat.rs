//! Inspect the manifest of a package.

use clap::Parser;
use libscoop::{Session, fs, os, package};
use std::io::Write;
use std::process::{Command, Stdio};

use crate::Result;

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

/// Pipe a JSON string through bat for syntax highlighting (Scoop's
/// scoop cat only shells out to bat when cat_style is configured).
fn run_bat(json: &str, style: &str) -> std::io::Result<()> {
    let mut child = Command::new("bat.exe")
        .args(["--no-paging", "--style", style, "--language", "json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(json.as_bytes())?;
    }
    child.wait()?;
    Ok(())
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
    // otherwise (or if bat cannot start) print the plain pretty JSON so the
    // manifest content is always shown.
    let config = session.config();
    let style = config.cat_style();
    let used_bat = !style.is_empty() && os::is_program_available("bat.exe");
    if used_bat && run_bat(&json, style).is_ok() {
        return Ok(());
    }

    println!("{json}");
    Ok(())
}
