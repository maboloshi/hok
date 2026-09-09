//! Inspect the manifest of a package.

use clap::Parser;
use libscoop::{QueryOption, Session, fs, os, package};
use std::io::Write;
use std::process::{Command, Stdio};

use crate::{Result, output};

/// Inspect the manifest of a package
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// Name of the package to be inspected
    package: String,
}

/// Pretty-print a manifest as JSON, rendering from the in-memory content so it
/// also works when the manifest is served by the (SQLite) manifest cache where
/// the on-disk path may not exist.
fn pretty_manifest(pkg: &package::Package) -> Result<String> {
    let manifest = pkg.manifest();
    // Prefer the retained raw text; else read the on-disk manifest file; else
    // (SQLite manifest cache - no raw text, virtual path) render the parsed
    // spec so the content is always shown.
    let content: String = if let Some(raw) = manifest.raw() {
        raw.to_string()
    } else if manifest.path().is_file() {
        fs::read_to_string(manifest.path())?
    } else {
        manifest
            .to_pretty_json()
            .ok_or_else(|| anyhow::anyhow!(rust_i18n::t!("cmd.cat_render_failed")))?
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

    let mut result = package::query::query(
        session,
        vec![query.as_str()],
        vec![QueryOption::Explicit],
        false,
    )?;

    let pkg = if result.is_empty() {
        return Err(anyhow::anyhow!(rust_i18n::t!(
            "cmd.cat_not_found",
            query = query
        )));
    } else if result.len() == 1 {
        result.remove(0)
    } else {
        result.sort_by_key(|p| p.ident());
        output::info(rust_i18n::t!(
            "cmd.found",
            count = result.len(),
            query = query
        ));
        for (idx, pkg) in result.iter().enumerate() {
            println!("  {idx}. {}/{}", pkg.bucket(), pkg.name());
        }
        let index = output::prompt_select(result.len())
            .ok_or_else(|| anyhow::anyhow!(rust_i18n::t!("cmd.cat_invalid")))?;
        result.remove(index)
    };

    let json = pretty_manifest(&pkg)?;

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
