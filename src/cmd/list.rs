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
use crossterm::style::Stylize;
use libscoop::{QueryOption, Session, package};

use crate::cmd::status_info_word;
use crate::format::{pad_visual, visual_width};
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

        let info = if words.is_empty() {
            String::new()
        } else {
            words.join(" · ").red().to_string()
        };

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
                rows.push(continuation("-", rust_i18n::t!("cmd.list_no_versions")));
            }

            for ver in retained {
                stats.retained += 1;
                rows.push(continuation(
                    &ver.version,
                    rust_i18n::t!("cmd.list_version_old"),
                ));
            }
        }
    }

    output::header(rust_i18n::t!("cmd.header_installed_apps"));

    if rows.is_empty() {
        output::info(rust_i18n::t!("cmd.list_no_match"));
        return Ok(());
    }

    render(&rows);

    println!();
    output::info(rust_i18n::t!(
        "cmd.list_summary",
        count = stats.shown,
        held = stats.held,
        upgradable = stats.upgradable,
        broken = stats.broken,
        orphaned = stats.orphaned,
        isolated = stats.isolated,
    ));
    if args.versions && stats.retained > 0 {
        output::info(rust_i18n::t!(
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
        info: note.as_ref().to_string().dark_grey().to_string(),
        dimmed: true,
    }
}

/// Render the table: bold header, dashed rule, then one line per row.
fn render(rows: &[Row]) {
    let hdr_name = rust_i18n::t!("cmd.list_header_name").to_string();
    let hdr_ver = rust_i18n::t!("cmd.list_header_version").to_string();
    let hdr_src = rust_i18n::t!("cmd.list_header_source").to_string();
    let hdr_info = rust_i18n::t!("cmd.list_header_info").to_string();

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
        pad_visual(&hdr_name, w_name).bold(),
        pad_visual(&hdr_ver, w_ver).bold(),
        pad_visual(&hdr_src, w_src).bold(),
        hdr_info.bold(),
    );
    println!(
        "  {}  {}  {}  {}",
        "-".repeat(w_name).dark_grey(),
        "-".repeat(w_ver).dark_grey(),
        "-".repeat(w_src).dark_grey(),
        "-".repeat(w_info).dark_grey(),
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
            (
                name.dark_grey().to_string(),
                version.dark_grey().to_string(),
                source.dark_grey().to_string(),
            )
        } else {
            (name.dark_cyan().to_string(), version, source)
        };

        if row.info.is_empty() {
            println!("  {}  {}  {}", name, version, source);
        } else {
            println!("  {}  {}  {}  {}", name, version, source, row.info);
        }
    }
}
