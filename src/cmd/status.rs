//! Show status and check for new package versions.

use clap::Parser;
use crossterm::style::Stylize;
use libscoop::{Session, package};

use crate::format::{pad_visual, visual_width};
use crate::{Result, output};

/// Show the status of all installed apps
#[derive(Debug, Parser)]
pub struct Args {
    /// Also show up-to-date apps (default: only apps needing attention)
    #[arg(short = 'a', long, action = clap::ArgAction::SetTrue)]
    all: bool,
}

/// A single rendered status row.
struct Row {
    name: String,
    installed: String,
    latest: String,
    info: String,
}

/// Render a ScoopStatus-style table:
/// Name | Installed Version | Latest Version | Info.
fn render(rows: &[Row]) {
    let hdr_name = rust_i18n::t!("cmd.status_header_name").to_string();
    let hdr_installed = rust_i18n::t!("cmd.status_header_installed").to_string();
    let hdr_latest = rust_i18n::t!("cmd.status_header_latest").to_string();
    let hdr_info = rust_i18n::t!("cmd.status_header_info").to_string();

    let w_info = visual_width(&hdr_info);
    let mut w_name = visual_width(&hdr_name);
    let mut w_installed = visual_width(&hdr_installed);
    let mut w_latest = visual_width(&hdr_latest);
    for row in rows {
        w_name = w_name.max(visual_width(&row.name));
        w_installed = w_installed.max(visual_width(&row.installed));
        w_latest = w_latest.max(visual_width(&row.latest));
    }

    println!(
        "  {}  {}  {}  {}",
        pad_visual(&hdr_name, w_name).bold(),
        pad_visual(&hdr_installed, w_installed).bold(),
        pad_visual(&hdr_latest, w_latest).bold(),
        hdr_info.bold(),
    );
    println!(
        "  {}  {}  {}  {}",
        "-".repeat(w_name).dark_grey(),
        "-".repeat(w_installed).dark_grey(),
        "-".repeat(w_latest).dark_grey(),
        "-".repeat(w_info).dark_grey(),
    );

    for row in rows {
        let name = pad_visual(&row.name, w_name);
        let installed = pad_visual(&row.installed, w_installed);
        let latest = pad_visual(&row.latest, w_latest);
        println!(
            "  {}  {}  {}  {}",
            name.dark_cyan(),
            installed,
            latest.green(),
            row.info.clone().red(),
        );
    }
}

/// Assemble the per-app info column, mirroring Scoop's app_status Info:
/// comma-joined Install failed / Held package / Deprecated / Manifest removed
/// plus the missing-dependency names.
fn info_for(s: &package::status::AppStatus) -> String {
    let mut parts = Vec::new();
    if s.failed {
        parts.push(rust_i18n::t!("cmd.status_install_failed").to_string());
    }
    if s.held {
        parts.push(rust_i18n::t!("cmd.status_held").to_string());
    }
    if s.deprecated {
        parts.push(rust_i18n::t!("cmd.status_deprecated").to_string());
    }
    if s.removed {
        parts.push(rust_i18n::t!("cmd.status_removed").to_string());
    }
    if !s.missing_deps.is_empty() {
        parts.push(s.missing_deps.join(", "));
    }
    parts.join(", ")
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let all = package::status::status(session)?;

    // Default: list apps that need attention (outdated / failed / deprecated /
    // removed / missing deps), matching `scoop status`. `-a/--all` (hok
    // extension) also lists healthy, up-to-date apps.
    let shown: Vec<&package::status::AppStatus> = all
        .iter()
        .filter(|s| args.all || s.needs_attention())
        .collect();

    if shown.is_empty() {
        output::info(rust_i18n::t!("cmd.install_ok"));
        return Ok(());
    }

    output::header(rust_i18n::t!("cmd.header_status"));
    let rows = shown
        .iter()
        .map(|s| {
            let installed = s.installed_version.clone().unwrap_or_default();
            let latest = s.latest_version.clone().unwrap_or_default();
            Row {
                name: format!("{}/{}", s.bucket, s.name),
                installed,
                latest,
                info: info_for(s),
            }
        })
        .collect::<Vec<_>>();
    render(&rows);

    // Summary footer (Scoop does not print one; hok adds a compact count).
    let outdated = shown.iter().filter(|s| s.outdated).count();
    println!();
    output::info(rust_i18n::t!(
        "cmd.status_summary",
        count = shown.len(),
        outdated = outdated,
    ));
    Ok(())
}
