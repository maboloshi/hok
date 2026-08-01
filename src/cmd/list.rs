use clap::{ArgAction, Parser};
use crossterm::style::Stylize;
use libscoop::{package, QueryOption, Session};

use crate::{output, Result};

/// Approximate visual width of a string in a terminal.
/// CJK characters count as 2, everything else as 1.
fn visual_width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            if c >= '\u{1100}' && (c <= '\u{115F}' || c == '\u{2329}' || c == '\u{232A}'
                || ('\u{2E80}'..='\u{9FFF}').contains(&c)
                || ('\u{A000}'..='\u{A4CF}').contains(&c)
                || ('\u{AC00}'..='\u{D7AF}').contains(&c)
                || ('\u{F900}'..='\u{FAFF}').contains(&c)
                || ('\u{FE30}'..='\u{FE6F}').contains(&c)
                || ('\u{FF01}'..='\u{FF60}').contains(&c)
                || ('\u{FFE0}'..='\u{FFE6}').contains(&c))
            {
                2
            } else {
                1
            }
        })
        .sum()
}

/// Pad a string to a target visual width by appending spaces.
fn pad_visual(s: &str, width: usize) -> String {
    let vw = visual_width(s);
    if vw >= width {
        s.to_owned()
    } else {
        format!("{}{}", s, " ".repeat(width - vw))
    }
}

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

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let queries = args.query.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    let mut options = vec![QueryOption::UpgradableCheck];

    if args.explicit {
        options.push(QueryOption::Explicit);
    }

    if args.upgradable {
        options.push(QueryOption::Upgradable);
    }

    if args.versions {
        return list_with_versions(&queries, &options, session);
    }

    match package::query::query(session, queries, options, true) {
        Err(e) => Err(e.into()),
        Ok(packages) => {
            // Compute column widths
            let mut max_name = 4usize;
            let mut max_ver = 7usize;
            let mut max_src = 6usize;
            for pkg in &packages {
                max_name = max_name.max(pkg.name().len());
                max_ver = max_ver.max(pkg.version().len());
                max_src = max_src.max(pkg.bucket().len());
            }

            // Header
            output::header(rust_i18n::t!("cmd.header_installed_apps"));
            let hdr_name = rust_i18n::t!("cmd.list_header_name");
            let hdr_ver  = rust_i18n::t!("cmd.list_header_version");
            let hdr_src  = rust_i18n::t!("cmd.list_header_source");
            let hdr_info = rust_i18n::t!("cmd.list_header_info");
            println!(
                "  {}  {}  {}  {}",
                pad_visual(&hdr_name, max_name).bold(),
                pad_visual(&hdr_ver, max_ver).bold(),
                pad_visual(&hdr_src, max_src).bold(),
                hdr_info.bold(),
            );
            println!(
                "  {}  {}  {}  ----",
                "-".repeat(max_name),
                "-".repeat(max_ver),
                "-".repeat(max_src),
            );

            let mut shown = 0u32;
            let mut shown_held = 0u32;
            let mut shown_upgradable = 0u32;
            for pkg in &packages {
                let held = pkg.is_held();
                if args.held && !held {
                    continue;
                }
                shown += 1;

                let mut info = Vec::new();
                if held {
                    info.push("held".to_string());
                    shown_held += 1;
                }
                if let Some(upgrade) = pkg.upgradable_version() {
                    info.push(format!("→ {}", upgrade));
                    shown_upgradable += 1;
                }
                let info_str = info.join(", ");

                let info_display = if info_str.is_empty() {
                    String::new()
                } else {
                    info_str.red().to_string()
                };

                let name_col = format!("{:<1$}", pkg.name(), max_name);
                let ver_col  = format!("{:<1$}", pkg.version(), max_ver);
                let src_col  = format!("{:<1$}", pkg.bucket(), max_src);
                println!(
                    "  {}  {}  {}  {}",
                    name_col.dark_cyan(),
                    ver_col,
                    src_col,
                    info_display,
                );
            }

            // Status summary footer
            println!();
            output::info(rust_i18n::t!(
                "cmd.list_summary",
                count = shown,
                held = shown_held,
                upgradable = shown_upgradable,
            ));

            Ok(())
        }
    }
}

/// List packages with all installed versions shown.
fn list_with_versions(queries: &[&str], options: &[QueryOption], session: &Session) -> Result<()> {
    // Get current packages for name/bucket info
    let pkgs = package::query::query(session, queries.to_vec(), options.to_vec(), true)
        .unwrap_or_default();

    for pkg in &pkgs {
        output::named(pkg.name(), format!("/{}", pkg.bucket()));

        let versions = package::list::list_installed_versions(session, pkg.name())?;

        if versions.is_empty() {
            output::named("(no versions)", "(broken install)");
            continue;
        }

        for ver in &versions {
            if ver.is_current {
                output::named(ver.version.as_str(), "(current)");
            } else {
                output::status(&ver.version);
            }
        }
    }

    Ok(())
}
use crate::cmd::shared_args::Cmd;

impl Cmd for Args {
    type Args = Self;

    #[inline]
    fn execute(args: Self::Args, session: &Session) -> Result<()> {
        execute(args, session)
    }
}
