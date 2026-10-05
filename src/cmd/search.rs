//! Search available package(s).

use clap::{ArgAction, Parser};
use libscoop::{Session, package};

use crate::format::{pad_visual, visual_width};
use crate::output::style;
use crate::{Result, output};

/// Search available package(s)
///
/// Search available package(s) from synced buckets.
/// The query is performed against package names by default, use
/// --with-description or --with-binary to search through package
/// descriptions or binaries. Without a query, no search is performed.
#[derive(Debug, Parser)]
pub struct Args {
    /// The query string (regex supported by default)
    #[arg(action = ArgAction::Append)]
    query: Vec<String>,
    /// Turn regex off and use explicit matching
    #[arg(short = 'e', long, action = ArgAction::SetTrue, conflicts_with_all = &["with_binary", "with_description"])]
    explicit: bool,
    /// Search through package binaries as well
    #[arg(short = 'B', long, action = ArgAction::SetTrue)]
    with_binary: bool,
    /// Search through package descriptions as well
    #[arg(short = 'D', long, action = ArgAction::SetTrue)]
    with_description: bool,
}

/// Which optional extra columns are active for this search run.
struct Columns {
    /// `--with-binary`: show package shims.
    binary: bool,
    /// `--with-description`: show manifest description.
    description: bool,
}

impl Columns {
    fn from_args(args: &Args) -> Self {
        Columns {
            binary: args.with_binary,
            description: args.with_description,
        }
    }

    /// Return the localized header labels in column order.
    /// Base columns (always present): Name | Version | Source | Installed.
    fn headers(&self) -> Vec<&'static str> {
        let mut h = vec!["Name", "Version", "Source", "Installed"];
        if self.binary {
            h.push("Binary");
        }
        if self.description {
            h.push("Description");
        }
        h
    }
}

/// A single rendered table row for the search result.
struct SearchRow {
    name: String,
    version: String,
    source: String,
    installed: String,
    binary: String,
    description: String,
    /// When true, the installed manifest version differs from the manifest
    /// version; we show a version-change note in the Installed column.
    version_diff: Option<String>,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    // Fix for no-parameter output: if no query is given, report and exit
    // instead of performing a wildcard (`*`) search that produces thousands
    // of dim `status()` lines.
    if args.query.is_empty() {
        output::warn(hok_i18n::t!("cmd.search_no_query"));
        return Err(crate::ExitCode(1).into());
    }

    let queries = args.query.iter().map(|s| s.as_str()).collect::<Vec<_>>();

    let qargs = crate::cmd::shared_args::QueryArgs {
        explicit: args.explicit,
        with_binary: args.with_binary,
        with_description: args.with_description,
    };
    let options = qargs.to_query_options();

    let packages = package::query::query(session, queries, options, false)?;

    if packages.is_empty() {
        output::warn(hok_i18n::t!("cmd.search_no_match"));
        return Err(crate::ExitCode(1).into());
    }

    let cols = Columns::from_args(&args);

    // Build rows.
    let mut rows: Vec<SearchRow> = Vec::with_capacity(packages.len());
    for pkg in packages {
        let installed_str = if pkg.is_strictly_installed() {
            pkg.installed_version().unwrap().to_string()
        } else {
            String::new()
        };

        let manifest_version = pkg.version().to_owned();
        let version_diff = if pkg.is_strictly_installed() {
            if manifest_version != installed_str {
                Some(format!("→ {}", manifest_version))
            } else {
                None
            }
        } else {
            None
        };

        let binary_str = if args.with_binary {
            match pkg.shims() {
                None => String::new(),
                Some(shims) => shims.join(", "),
            }
        } else {
            String::new()
        };

        let desc_str = if args.with_description {
            pkg.description().unwrap_or("").to_string()
        } else {
            String::new()
        };

        rows.push(SearchRow {
            name: pkg.name().to_owned(),
            version: manifest_version,
            source: pkg.bucket().to_owned(),
            installed: installed_str,
            binary: binary_str,
            description: desc_str,
            version_diff,
        });
    }

    render_table(&cols, &rows, args.query.join(", ").as_str());
    Ok(())
}

/// Render the table, referencing `list.rs:layout` style.
fn render_table(cols: &Columns, rows: &[SearchRow], query_str: &str) {
    let headers_raw = cols.headers();
    let header_labels: Vec<String> = headers_raw.iter().map(|h| h.to_string()).collect();

    // Compute column widths: max of header and each cell.
    let mut widths: Vec<usize> = header_labels.iter().map(|h| visual_width(h)).collect();
    for row in rows {
        let cells = row.cells(cols);
        for (i, cell) in cells.iter().enumerate() {
            let vw = visual_width(cell);
            if i < widths.len() && vw > widths[i] {
                widths[i] = vw;
            }
        }
    }

    // Print result count line (reference hyperscoop style).
    println!(
        "  {}",
        style::dim(format!("{} result(s) found.", rows.len()))
    );

    // Header row.
    let header_line = build_line(&header_labels, &widths, true);
    println!("{header_line}");

    // Separator rule.
    let rule_parts: Vec<String> = header_labels
        .iter()
        .enumerate()
        .map(|(i, _)| "-".repeat(widths.get(i).copied().unwrap_or(0)))
        .collect();
    println!("{}", style::dim(format!("  {}", rule_parts.join("  "))));

    // Data rows.
    for row in rows {
        let cells = row.cells(cols);
        let line = build_line(&cells, &widths, false);
        println!("{line}");
    }

    // Bottom summary line, matching other query-class commands (`list`, etc.)
    println!();
    println!(
        "  {}",
        style::dim(hok_i18n::t!(
            "cmd.search_summary",
            count = rows.len(),
            query = query_str
        ))
    );
}

/// Build a single table line with appropriate padding and styling.
fn build_line(cells: &[String], widths: &[usize], is_header: bool) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(cells.len());
    for (i, cell) in cells.iter().enumerate() {
        let w = widths.get(i).copied().unwrap_or(visual_width(cell));
        let padded = pad_visual(cell, w);
        let styled = if is_header {
            style::accent_bold(padded.as_str())
        } else {
            // For data rows: emphasize the package name (first column),
            // leave others in default foreground.
            if i == 0 {
                style::emphasis(padded.as_str())
            } else {
                padded
            }
        };
        parts.push(styled);
    }
    format!("  {}", parts.join("  "))
}

impl SearchRow {
    /// Return cell values ordered by the active columns.
    fn cells(&self, cols: &Columns) -> Vec<String> {
        let mut c = vec![self.name.clone(), self.version.clone(), self.source.clone()];
        // Installed column: show installed version; if version differs,
        // append the change arrow in the same cell (simpler than adding
        // an extra column not selected by the user).
        let installed_display = if let Some(ref diff) = self.version_diff {
            if self.installed.is_empty() {
                // Should not happen when diff exists, but guard.
                String::new()
            } else {
                format!("{} {}", self.installed, diff)
            }
        } else {
            self.installed.clone()
        };
        c.push(installed_display);

        if cols.binary {
            c.push(self.binary.clone());
        }
        if cols.description {
            c.push(self.description.clone());
        }
        c
    }
}
