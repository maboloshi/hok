//! List installed package(s).
//!
//! Both the plain listing and the `--versions` listing render through the
//! same table: one row per package, plus dimmed continuation rows for the
//! retained (non-current) versions when `--versions` is given.
//!
//! The info column reuses `status`'s word set and order
//! (`AppStatus::info_kinds`, localized by `status_info_word`) so the two
//! commands cannot drift; `list` additionally folds the outdated "-> new"
//! arrow into Info because it has no Latest column.

use clap::{ArgAction, Parser};
use libscoop::{QueryOption, Session, package};

use crate::cmd::status_info_word;
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
    /// Pre-styled info column; it is the last column and never padded.
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
        // column, so an outdated app folds its "-> new" arrow in here; a
        // future Updated column could move that out of Info.
        // TODO: 未来加 Updated 列 后，把 outdated 的 → <new> 从 Info 挪走。
        let mut words = st
            .info_kinds()
            .into_iter()
            .map(status_info_word)
            .collect::<Vec<_>>();
        if let Some(upgrade) = pkg.upgradable_version() {
            words.push(format!("→ {}", upgrade));
            stats.upgradable += 1;
        }

        // The Info column is styled at render time (attention words danger
        // red, continuation notes dim), so it is built as plain text here.
        let info = words.join(" · ");

        rows.push(Row {
            name: pkg.name().to_owned(),
            version: pkg.version().to_owned(),
            source: st.bucket,
            info,
            dimmed: false,
        });

        if args.versions {
            let versions = package::list::list_installed_versions(session, pkg.name())?;
            let retained = versions
                .iter()
                .filter(|v| !v.is_current && v.version != pkg.version());

            if versions.is_empty() {
                rows.push(continuation("-", hok_i18n::t!("cmd.list_no_versions")));
            }

            for ver in retained {
                stats.retained += 1;
                rows.push(continuation(
                    &ver.version,
                    hok_i18n::t!("cmd.list_version_old"),
                ));
            }
        }
    }

    output::header(hok_i18n::t!("cmd.header_installed_apps"));

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
        info: note.as_ref().to_string(),
        dimmed: true,
    }
}

/// Render the table: bold header, dashed rule, then one line per row.
fn render(rows: &[Row]) {
    let hdr_name = hok_i18n::t!("cmd.list_header_name").to_string();
    let hdr_ver = hok_i18n::t!("cmd.list_header_version").to_string();
    let hdr_src = hok_i18n::t!("cmd.list_header_source").to_string();
    let hdr_info = hok_i18n::t!("cmd.list_header_info").to_string();

    let mut w_name = visual_width(&hdr_name);
    let mut w_ver = visual_width(&hdr_ver);
    let mut w_src = visual_width(&hdr_src);
    let w_info = visual_width(&hdr_info);
    for row in rows {
        w_name = w_name.max(visual_width(&row.name));
        w_ver = w_ver.max(visual_width(&row.version));
        w_src = w_src.max(visual_width(&row.source));
    }

    println!(
        "  {}  {}  {}  {}",
        style::emphasis(pad_visual(&hdr_name, w_name)),
        style::emphasis(pad_visual(&hdr_ver, w_ver)),
        style::emphasis(pad_visual(&hdr_src, w_src)),
        style::emphasis(hdr_info),
    );
    println!(
        "  {}  {}  {}  {}",
        style::dim("-".repeat(w_name)),
        style::dim("-".repeat(w_ver)),
        style::dim("-".repeat(w_src)),
        style::dim("-".repeat(w_info)),
    );

    for row in rows {
        let name = pad_visual(&row.name, w_name);
        let version = pad_visual(&row.version, w_ver);
        // Trailing padding is pointless when nothing follows the source.
        let source = if row.info.is_empty() {
            row.source.clone()
        } else {
            pad_visual(&row.source, w_src)
        };

        let (name, version, source) = if row.dimmed {
            (style::dim(name), style::dim(version), style::dim(source))
        } else {
            (style::emphasis(name), version, source)
        };
        // Attention words are danger red; a continuation row's note is dim.
        let info = if row.dimmed {
            style::dim(&row.info)
        } else {
            style::danger(&row.info)
        };

        if row.info.is_empty() {
            println!("  {}  {}  {}", name, version, source);
        } else {
            println!("  {}  {}  {}  {}", name, version, source, info);
        }
    }
}
