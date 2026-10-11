//! List installed package(s).
//!
//! Both the plain listing and the `--versions` listing render through the
//! same table: one row per package, plus dimmed continuation rows for the
//! retained (non-current) versions when `--versions` is given.
//!
//! The info column takes its word set and order from `status`
//! (`AppStatus::info_kinds`, localized by `status_info_word`) so the two
//! commands cannot drift, but the wording itself is per-command: upstream says
//! `Deprecated package` in `list` and `Deprecated` in `status`, which is what
//! the `InfoContext` argument selects. `list` additionally folds the outdated
//! "-> new" arrow into Info — a hok extension, kept even though the table now
//! carries upstream's `Updated` column (upstream's `list` gives no update
//! indication at all).

use clap::{ArgAction, Parser};
use libscoop::{QueryOption, Session, package};

use crate::cmd::{InfoContext, status_info_word, version_note};
use crate::format::{pad_visual, visual_width};
use crate::output::style;
use crate::{Result, output};

/// List installed package(s)
#[derive(Debug, Parser)]
pub struct Args {
    /// The query string (regex supported by default)
    #[arg(action = ArgAction::Append)]
    query: Vec<String>,
    /// Turn regex off and use explicit matching
    #[arg(short = 'e', long, action = ArgAction::SetTrue)]
    explicit: bool,
    /// List upgradable package(s)
    #[arg(short = 'u', long, action = ArgAction::SetTrue)]
    upgradable: bool,
    /// List held package(s)
    #[arg(short = 'H', long, action = ArgAction::SetTrue)]
    held: bool,
    /// Show all installed versions (not just current)
    #[arg(short = 'V', long, action = ArgAction::SetTrue)]
    versions: bool,
}

/// A single rendered table row.
struct Row {
    name: String,
    version: String,
    source: String,
    /// `Updated` column: the `install.json` / app-dir mtime rendered as local
    /// `yyyy-MM-dd HH:mm:ss` (`package::list::update_time`); empty when it
    /// cannot be read.
    updated: String,
    /// Info column text (coloured per row kind when laid out); it is the last
    /// column and is never padded.
    info: String,
    /// Retained (non-current) version rows are dimmed as continuation rows.
    dimmed: bool,
}

/// Counters feeding the summary footer.
#[derive(Default)]
struct Stats {
    shown: u32,
    held: u32,
    upgradable: u32,
    broken: u32,
    /// Deprecated / Orphaned / BucketMissing (no longer fully managed).
    orphaned: u32,
    isolated: u32,
    retained: u32,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let queries = args.query.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    let mut options = vec![QueryOption::UpgradableCheck, QueryOption::Broken];

    if args.explicit {
        options.push(QueryOption::Explicit);
    }

    if args.upgradable {
        options.push(QueryOption::Upgradable);
    }

    let packages = package::query::query(session, queries, options, true)?;
    let sources = package::source::classify(session, &packages);

    let mut rows = Vec::new();
    let mut stats = Stats::default();

    for (idx, pkg) in packages.iter().enumerate() {
        let st = package::status::AppStatus::from_package(pkg, sources[idx]);
        let held = pkg.is_held();
        if args.held && !held {
            continue;
        }
        stats.shown += 1;

        if st.broken {
            stats.broken += 1;
        }
        match st.source {
            package::SourceState::Isolated => stats.isolated += 1,
            package::SourceState::Deprecated
            | package::SourceState::Orphaned
            | package::SourceState::BucketMissing => stats.orphaned += 1,
            package::SourceState::Synced | package::SourceState::UnknownBroken => {}
        }
        if held {
            stats.held += 1;
        }

        // Shared info words (same order as `status`). `list` has no Latest
        // column, so an outdated app folds its "-> new" arrow in here. That
        // arrow is a hok extension and deliberately stays in Info even though
        // the Upstream `Updated` column now exists — upstream's `list` shows
        // no update indication at all.
        let mut words = st
            .info_kinds()
            .into_iter()
            .map(|kind| status_info_word(kind, InfoContext::List))
            .collect::<Vec<_>>();
        if let Some(upgrade) = pkg.upgradable_version() {
            words.push(format!("→ {}", upgrade));
            stats.upgradable += 1;
        }

        // The Info column is styled at render time (attention words danger
        // red, continuation notes dim), so it is built as plain text here.
        let info = words.join(", ");

        rows.push(Row {
            name: pkg.name().to_owned(),
            version: pkg.version().to_owned(),
            source: st.bucket,
            updated: package::list::update_time(session, pkg.name(), pkg.version())
                .unwrap_or_default(),
            info,
            dimmed: false,
        });

        if args.versions {
            let versions = package::list::list_installed_versions(session, pkg.name())?;
            // Whether the shared resolver named any version as current. A
            // broken install resolves none, and then no retained row may be
            // called an old version — `cleanup --list` renders the same block
            // through this very helper.
            let has_current = versions.iter().any(|v| v.is_current);
            let retained = versions
                .iter()
                .filter(|v| !v.is_current && v.version != pkg.version())
                .collect::<Vec<_>>();

            if versions.is_empty() {
                rows.push(continuation("-", hok_i18n::t!("cmd.list_no_versions")));
            }

            for (index, ver) in retained.into_iter().enumerate() {
                // Only a version of an app whose current version resolves is a
                // retained *old* version. A broken install's dirs stay out of
                // that count and carry the unknown note instead — they are not
                // cleanable leftovers either (`cleanup` skips such an app).
                if has_current {
                    stats.retained += 1;
                }
                rows.push(continuation(
                    &ver.version,
                    version_note(index, ver.is_current, has_current).text(),
                ));
            }
        }
    }

    // Scoop distinguishes an empty installation from a query that simply
    // matched nothing: with no app installed at all it reports "There aren't
    // any apps installed." and exits 1 (`scoop-list.ps1:19-22`), while a
    // non-matching query against a populated installation exits 0. The extra
    // wildcard scan runs only when nothing was rendered.
    if rows.is_empty()
        && package::query::query(session, vec![], vec![QueryOption::Broken], true)?.is_empty()
    {
        output::info(hok_i18n::t!("cmd.list_no_apps"));
        return Err(crate::ExitCode(1).into());
    }

    // Upstream echoes the query it filtered on (`scoop-list.ps1:25`); hok
    // accepts several queries, so they are all echoed.
    if args.query.is_empty() {
        output::header(hok_i18n::t!("cmd.header_installed_apps"));
    } else {
        output::header(hok_i18n::t!(
            "cmd.header_installed_apps_matching",
            query = args.query.join("', '")
        ));
    }

    if rows.is_empty() {
        output::info(hok_i18n::t!("cmd.list_no_match"));
        return Ok(());
    }

    render(&rows);

    println!();
    output::info(hok_i18n::t!(
        "cmd.list_summary",
        count = stats.shown,
        held = stats.held,
        upgradable = stats.upgradable,
        broken = stats.broken,
        orphaned = stats.orphaned,
        isolated = stats.isolated,
    ));
    if args.versions && stats.retained > 0 {
        output::info(hok_i18n::t!(
            "cmd.list_summary_versions",
            versions = stats.retained,
            apps = stats.shown,
        ));
    }

    Ok(())
}

/// Build a dimmed continuation row carrying an extra installed version.
fn continuation(version: &str, note: impl AsRef<str>) -> Row {
    Row {
        name: String::new(),
        version: version.to_owned(),
        source: String::new(),
        updated: String::new(),
        info: note.as_ref().to_string(),
        dimmed: true,
    }
}

/// Lay out the table: bold header, dashed rule, then one line per row.
///
/// Columns are upstream's Name | Version | Source | Updated | Info
/// (`scoop-list.ps1:35-48`, `ScoopTypes.Format.ps1xml:22-28`); `info` is the
/// last column and is never padded, and a trailing all-empty Updated column is
/// trimmed too, so no rendered line carries trailing spaces.
fn layout(rows: &[Row], headers: [&str; 5]) -> Vec<String> {
    let [hdr_name, hdr_ver, hdr_src, hdr_updated, hdr_info] = headers;

    let mut w_name = visual_width(hdr_name);
    let mut w_ver = visual_width(hdr_ver);
    let mut w_src = visual_width(hdr_src);
    let mut w_updated = visual_width(hdr_updated);
    let w_info = visual_width(hdr_info);
    for row in rows {
        w_name = w_name.max(visual_width(&row.name));
        w_ver = w_ver.max(visual_width(&row.version));
        w_src = w_src.max(visual_width(&row.source));
        w_updated = w_updated.max(visual_width(&row.updated));
    }

    let mut lines = vec![
        format!(
            "  {}  {}  {}  {}  {}",
            style::accent_bold(pad_visual(hdr_name, w_name)),
            style::accent_bold(pad_visual(hdr_ver, w_ver)),
            style::accent_bold(pad_visual(hdr_src, w_src)),
            style::accent_bold(pad_visual(hdr_updated, w_updated)),
            style::accent_bold(hdr_info),
        ),
        format!(
            "  {}  {}  {}  {}  {}",
            style::dim("-".repeat(w_name)),
            style::dim("-".repeat(w_ver)),
            style::dim("-".repeat(w_src)),
            style::dim("-".repeat(w_updated)),
            style::dim("-".repeat(w_info)),
        ),
    ];

    for row in rows {
        let name = pad_visual(&row.name, w_name);
        let version = pad_visual(&row.version, w_ver);
        let has_updated = !row.updated.is_empty();
        let has_info = !row.info.is_empty();
        // Trailing padding is pointless when nothing follows the column.
        let source = if has_updated || has_info {
            pad_visual(&row.source, w_src)
        } else {
            row.source.clone()
        };
        let updated = if has_info {
            pad_visual(&row.updated, w_updated)
        } else {
            row.updated.clone()
        };

        let (name, version, source, updated) = if row.dimmed {
            (
                style::dim(name),
                style::dim(version),
                style::dim(source),
                style::dim(updated),
            )
        } else {
            (style::emphasis(name), version, source, updated)
        };
        // Attention words are danger red; a continuation row's note is dim.
        let info = if row.dimmed {
            style::dim(&row.info)
        } else {
            style::danger(&row.info)
        };

        let mut line = format!("  {}  {}  {}", name, version, source);
        if has_updated || has_info {
            line.push_str(&format!("  {}", updated));
        }
        if has_info {
            line.push_str(&format!("  {}", info));
        }
        lines.push(line);
    }

    lines
}

/// Print the table with the localized headers.
fn render(rows: &[Row]) {
    let name = hok_i18n::t!("cmd.list_header_name");
    let version = hok_i18n::t!("cmd.list_header_version");
    let source = hok_i18n::t!("cmd.list_header_source");
    let updated = hok_i18n::t!("cmd.list_header_updated");
    let info = hok_i18n::t!("cmd.list_header_info");

    for line in layout(
        rows,
        [
            name.as_str(),
            version.as_str(),
            source.as_str(),
            updated.as_str(),
            info.as_str(),
        ],
    ) {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::plain;

    fn row(name: &str, version: &str, source: &str, updated: &str, info: &str) -> Row {
        Row {
            name: name.to_owned(),
            version: version.to_owned(),
            source: source.to_owned(),
            updated: updated.to_owned(),
            info: info.to_owned(),
            dimmed: false,
        }
    }

    fn value_column(line: &str, value: &str) -> usize {
        let line = plain(line);
        let at = line.find(value).expect("the value is on the line");
        visual_width(&line[..at])
    }

    /// Covers the whole table: the header, the rule and the rows line up
    /// column by column (CJK names included), and a row whose trailing
    /// `updated`/`info` columns are empty ends at the last non-empty column
    /// instead of trailing spaces.
    #[test]
    fn layout_aligns_columns_and_trims_empty_trailing_columns() {
        let rows = vec![
            row("7zip", "26.03", "main", "2026-07-25 10:48:34", ""),
            row(
                "directoryopus-cn",
                "13.25.3",
                "scoop-app",
                "",
                "Update available",
            ),
            row(
                "中文包",
                "1.0",
                "main",
                "2026-01-02 03:04:05",
                "Held package",
            ),
        ];
        let lines = layout(&rows, ["名称", "版本", "源", "更新时间", "状态"])
            .iter()
            .map(|l| plain(l))
            .collect::<Vec<_>>();

        let version_col = 2 + visual_width("directoryopus-cn") + 2;
        let source_col = version_col + visual_width("13.25.3") + 2;
        let updated_col = source_col + visual_width("scoop-app") + 2;
        let info_col = updated_col + visual_width("2026-07-25 10:48:34") + 2;
        for (line, version) in lines[2..].iter().zip(["26.03", "13.25.3", "1.0"]) {
            assert_eq!(value_column(line, version), version_col, "on `{line}`");
        }
        for (line, source) in lines[2..].iter().zip(["main", "scoop-app", "main"]) {
            assert_eq!(value_column(line, source), source_col, "on `{line}`");
        }
        // Rows 1 and 3 carry an updated timestamp at the Updated column; row
        // 2's updated cell is empty, yet its Info still starts at `info_col`.
        assert_eq!(value_column(&lines[2], "2026-07-25 10:48:34"), updated_col);
        assert_eq!(value_column(&lines[4], "2026-01-02 03:04:05"), updated_col);
        assert_eq!(value_column(&lines[3], "Update available"), info_col);
        assert_eq!(value_column(&lines[4], "Held package"), info_col);

        // Row 1 has no info, so it ends right after `updated`; row 2 carries
        // its info after the (empty) updated cell.
        assert!(lines[2].ends_with("10:48:34"), "row 1: {}", lines[2]);
        assert!(
            lines[3].ends_with("Update available"),
            "row 2: {}",
            lines[3]
        );
        assert!(
            lines.iter().all(|l| !l.ends_with(' ')),
            "trailing spaces: {lines:#?}"
        );
    }

    /// A row with neither an updated timestamp nor info ends right after the
    /// source column (the empty-cell trimming rule, one column further).
    #[test]
    fn layout_trims_both_empty_updated_and_info() {
        let rows = vec![row("7zip", "26.03", "main", "", "")];
        let lines = layout(&rows, ["Name", "Version", "Source", "Updated", "Info"])
            .iter()
            .map(|l| plain(l))
            .collect::<Vec<_>>();

        assert!(lines[2].ends_with("main"), "row: {}", lines[2]);
        assert!(
            lines.iter().all(|l| !l.ends_with(' ')),
            "trailing spaces: {lines:#?}"
        );
    }

    /// A dimmed continuation row lays out in the same columns as a normal one.
    #[test]
    fn layout_keeps_continuation_rows_aligned() {
        let rows = vec![
            row("git", "2.51.0", "main", "2026-07-25 10:48:34", ""),
            Row {
                name: String::new(),
                version: "2.50.0".to_owned(),
                source: String::new(),
                updated: String::new(),
                info: "old".to_owned(),
                dimmed: true,
            },
        ];
        let lines = layout(&rows, ["Name", "Version", "Source", "Updated", "Info"])
            .iter()
            .map(|l| plain(l))
            .collect::<Vec<_>>();

        let version_col = 2 + visual_width("Name").max(visual_width("git")) + 2;
        assert_eq!(value_column(&lines[2], "2.51.0"), version_col);
        assert_eq!(value_column(&lines[3], "2.50.0"), version_col);
        assert!(lines[3].contains("old"), "continuation: {}", lines[3]);
    }
}
