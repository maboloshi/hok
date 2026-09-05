//! Show status and check for new package versions.

use clap::{ArgAction, Parser};
use crossterm::style::Stylize;
use libscoop::{Session, package};

use crate::cmd::status_info_word;
use crate::format::{pad_visual, visual_width};
use crate::{Result, output};

/// Show the status of all installed apps
#[derive(Debug, Parser)]
pub struct Args {
    /// Also show up-to-date apps (default: only apps needing attention)
    #[arg(short = 'a', long, action = ArgAction::SetTrue)]
    all: bool,
}

/// A single rendered status row. Columns mirror the shared four-axis view:
/// Name | Source | Installed Version | Latest Version | Missing deps | Info.
struct Row {
    name: String,
    source: String,
    installed: String,
    latest: String,
    missing: String,
    info: String,
}

/// Render the status table.
fn render(rows: &[Row]) {
    let hdr_name = rust_i18n::t!("cmd.status_header_name").to_string();
    let hdr_source = rust_i18n::t!("cmd.status_header_source").to_string();
    let hdr_installed = rust_i18n::t!("cmd.status_header_installed").to_string();
    let hdr_latest = rust_i18n::t!("cmd.status_header_latest").to_string();
    let hdr_missing = rust_i18n::t!("cmd.status_header_missing").to_string();
    let hdr_info = rust_i18n::t!("cmd.status_header_info").to_string();

    let mut w_name = visual_width(&hdr_name);
    let mut w_source = visual_width(&hdr_source);
    let mut w_installed = visual_width(&hdr_installed);
    let mut w_latest = visual_width(&hdr_latest);
    let mut w_missing = visual_width(&hdr_missing);
    let w_info = visual_width(&hdr_info);
    for row in rows {
        w_name = w_name.max(visual_width(&row.name));
        w_source = w_source.max(visual_width(&row.source));
        w_installed = w_installed.max(visual_width(&row.installed));
        w_latest = w_latest.max(visual_width(&row.latest));
        w_missing = w_missing.max(visual_width(&row.missing));
    }

    println!(
        "  {}  {}  {}  {}  {}  {}",
        pad_visual(&hdr_name, w_name).bold(),
        pad_visual(&hdr_source, w_source).bold(),
        pad_visual(&hdr_installed, w_installed).bold(),
        pad_visual(&hdr_latest, w_latest).bold(),
        pad_visual(&hdr_missing, w_missing).bold(),
        hdr_info.bold(),
    );
    println!(
        "  {}  {}  {}  {}  {}  {}",
        "-".repeat(w_name).dark_grey(),
        "-".repeat(w_source).dark_grey(),
        "-".repeat(w_installed).dark_grey(),
        "-".repeat(w_latest).dark_grey(),
        "-".repeat(w_missing).dark_grey(),
        "-".repeat(w_info).dark_grey(),
    );

    for row in rows {
        println!(
            "  {}  {}  {}  {}  {}  {}",
            pad_visual(&row.name, w_name).dark_cyan(),
            pad_visual(&row.source, w_source),
            pad_visual(&row.installed, w_installed),
            pad_visual(&row.latest, w_latest).green(),
            pad_visual(&row.missing, w_missing),
            row.info.clone().red(),
        );
    }
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let all = package::status::status(session)?;

    // Default: list apps that need attention, matching `scoop status`.
    // `-a/--all` (hok extension) also lists healthy, up-to-date apps.
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
            let info = s
                .info_kinds()
                .into_iter()
                .map(status_info_word)
                .collect::<Vec<_>>()
                .join(" · ");
            Row {
                name: s.name.clone(),
                source: s.bucket.clone(),
                installed: s.installed_version.clone().unwrap_or_default(),
                latest: s.latest_version.clone().unwrap_or_default(),
                missing: s.missing_deps.join(" · "),
                info,
            }
        })
        .collect::<Vec<_>>();
    render(&rows);

    // Summary footer (B): every attention row is counted exactly once by its
    // highest-priority category (Broken > source anomaly > missing deps >
    // outdated), so the four numbers always sum to the attention count.
    // Healthy rows shown only via `-a/--all` are not part of the count.
    let attention = all.iter().filter(|s| s.needs_attention()).count();
    let broken = all.iter().filter(|s| s.broken).count();
    let source = all
        .iter()
        .filter(|s| s.attention() == package::status::Attention::Source)
        .count();
    let deps = all
        .iter()
        .filter(|s| s.attention() == package::status::Attention::MissingDeps)
        .count();
    let updatable = all
        .iter()
        .filter(|s| s.attention() == package::status::Attention::Updatable)
        .count();
    println!();
    output::info(rust_i18n::t!(
        "cmd.status_attention",
        count = attention,
        broken = broken,
        source = source,
        deps = deps,
        updatable = updatable,
    ));
    Ok(())
}
