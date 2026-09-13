//! Show status and check for new package versions.

use clap::{ArgAction, Parser};
use libscoop::{Session, package};

use crate::cmd::{InfoContext, status_info_word};
use crate::format::{pad_visual, visual_width};
use crate::output::style;
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

/// Lay out the status table: bold header, dashed rule, then one line per row.
///
/// A row's empty trailing columns are dropped, so a healthy row ends at its
/// last real value instead of leaving a run of padding behind (the same rule
/// `list` applies to its empty `Info` column). Empty cells must not be
/// coloured either — an empty coloured span would leak a stray SGR escape into
/// piped output.
fn layout(rows: &[Row], headers: [&str; 6]) -> Vec<String> {
    let [
        hdr_name,
        hdr_source,
        hdr_installed,
        hdr_latest,
        hdr_missing,
        hdr_info,
    ] = headers;

    let mut w_name = visual_width(hdr_name);
    let mut w_source = visual_width(hdr_source);
    let mut w_installed = visual_width(hdr_installed);
    let mut w_latest = visual_width(hdr_latest);
    let mut w_missing = visual_width(hdr_missing);
    let w_info = visual_width(hdr_info);
    for row in rows {
        w_name = w_name.max(visual_width(&row.name));
        w_source = w_source.max(visual_width(&row.source));
        w_installed = w_installed.max(visual_width(&row.installed));
        w_latest = w_latest.max(visual_width(&row.latest));
        w_missing = w_missing.max(visual_width(&row.missing));
    }

    let mut lines = vec![
        format!(
            "  {}  {}  {}  {}  {}  {}",
            style::accent_bold(pad_visual(hdr_name, w_name)),
            style::accent_bold(pad_visual(hdr_source, w_source)),
            style::accent_bold(pad_visual(hdr_installed, w_installed)),
            style::accent_bold(pad_visual(hdr_latest, w_latest)),
            style::accent_bold(pad_visual(hdr_missing, w_missing)),
            style::accent_bold(hdr_info),
        ),
        format!(
            "  {}  {}  {}  {}  {}  {}",
            style::dim("-".repeat(w_name)),
            style::dim("-".repeat(w_source)),
            style::dim("-".repeat(w_installed)),
            style::dim("-".repeat(w_latest)),
            style::dim("-".repeat(w_missing)),
            style::dim("-".repeat(w_info)),
        ),
    ];

    for row in rows {
        let widths = [w_name, w_source, w_installed, w_latest, w_missing, w_info];
        let raws = [
            row.name.as_str(),
            row.source.as_str(),
            row.installed.as_str(),
            row.latest.as_str(),
            row.missing.as_str(),
            row.info.as_str(),
        ];

        // Build one cell: `padded` is off for the last column a row actually
        // carries, so the row ends at its text instead of its padding. Empty
        // cells are never coloured — an empty coloured span would leak a stray
        // SGR escape into piped output.
        let cell = |index: usize, padded: bool| -> String {
            let raw = raws[index];
            let text = if padded {
                pad_visual(raw, widths[index])
            } else {
                raw.to_owned()
            };
            match index {
                0 => style::emphasis(text),
                3 if !raw.is_empty() => style::success(text),
                5 if !raw.is_empty() => style::danger(text),
                _ => text,
            }
        };

        let Some(last) = (0..raws.len()).rev().find(|&index| !raws[index].is_empty()) else {
            // A row with nothing at all still occupies a line.
            lines.push(String::new());
            continue;
        };
        let cells = (0..=last)
            .map(|index| cell(index, index < last))
            .collect::<Vec<_>>();
        lines.push(format!("  {}", cells.join("  ")));
    }

    lines
}

/// Print the status table with the localized headers.
fn render(rows: &[Row]) {
    let name = hok_i18n::t!("cmd.status_header_name");
    let source = hok_i18n::t!("cmd.status_header_source");
    let installed = hok_i18n::t!("cmd.status_header_installed");
    let latest = hok_i18n::t!("cmd.status_header_latest");
    let missing = hok_i18n::t!("cmd.status_header_missing");
    let info = hok_i18n::t!("cmd.status_header_info");

    for line in layout(
        rows,
        [
            name.as_str(),
            source.as_str(),
            installed.as_str(),
            latest.as_str(),
            missing.as_str(),
            info.as_str(),
        ],
    ) {
        println!("{line}");
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
        output::done(hok_i18n::t!("output.ok_all"));
        return Ok(());
    }

    let rows = shown
        .iter()
        .map(|s| {
            let info = s
                .info_kinds()
                .into_iter()
                .map(|kind| status_info_word(kind, InfoContext::Status))
                .collect::<Vec<_>>()
                .join(", ");
            Row {
                name: s.name.clone(),
                source: s.bucket.clone(),
                installed: s.installed_version.clone().unwrap_or_default(),
                latest: s.latest_version.clone().unwrap_or_default(),
                missing: s.missing_deps.join(" | "),
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
    output::info(hok_i18n::t!(
        "cmd.status_attention",
        count = attention,
        broken = broken,
        source = source,
        deps = deps,
        updatable = updatable,
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn row(
        name: &str,
        source: &str,
        installed: &str,
        latest: &str,
        missing: &str,
        info: &str,
    ) -> Row {
        Row {
            name: name.to_owned(),
            source: source.to_owned(),
            installed: installed.to_owned(),
            latest: latest.to_owned(),
            missing: missing.to_owned(),
            info: info.to_owned(),
        }
    }

    /// `line` without its ANSI SGR sequences, so widths can be measured.
    fn plain(line: &str) -> String {
        let mut out = String::new();
        let mut escaping = false;
        for ch in line.chars() {
            match ch {
                '\x1b' => escaping = true,
                'm' if escaping => escaping = false,
                _ if !escaping => out.push(ch),
                _ => {}
            }
        }
        out
    }

    fn value_column(line: &str, value: &str) -> usize {
        let line = plain(line);
        let at = line.find(value).expect("the value is on the line");
        visual_width(&line[..at])
    }

    /// Columns line up under the header (CJK names included), an empty tail is
    /// dropped instead of printed as padding, and empty cells in the middle
    /// keep their column so later values stay aligned.
    #[test]
    fn layout_aligns_columns_and_drops_empty_trailing_cells() {
        let rows = vec![
            row("aria2", "main", "1.37.0-1", "", "", ""),
            row("中文包", "scoop-app", "1.0", "2.0", "", "Update available"),
            row("sublime-text", "", "", "", "", "Install failed"),
        ];
        let headers = [
            "名称",
            "来源",
            "已安装版本",
            "最新版本",
            "缺少依赖项",
            "信息",
        ];
        let lines = layout(&rows, headers)
            .iter()
            .map(|l| plain(l))
            .collect::<Vec<_>>();

        // The header line is the oracle for the columns it declares.
        let source_col = value_column(&lines[0], "来源");
        let installed_col = value_column(&lines[0], "已安装版本");
        let latest_col = value_column(&lines[0], "最新版本");
        let info_col = value_column(&lines[0], "信息");

        assert_eq!(value_column(&lines[2], "main"), source_col, "row 1 source");
        assert_eq!(
            value_column(&lines[2], "1.37.0-1"),
            installed_col,
            "row 1 installed"
        );
        assert_eq!(value_column(&lines[3], "2.0"), latest_col, "row 2 latest");
        // Empty middle cells still hold their column width.
        assert_eq!(
            value_column(&lines[4], "Install failed"),
            info_col,
            "row 3 info"
        );

        // The empty tail is dropped: the healthy row ends at its version.
        assert!(lines[2].ends_with("1.37.0-1"), "row 1: {}", lines[2]);
        assert!(
            lines.iter().all(|l| !l.ends_with(' ')),
            "trailing spaces: {lines:#?}"
        );
    }
}
