//! Manage the download cache.

use clap::{ArgAction, Parser, Subcommand};
use libscoop::{Session, cache};

use crate::format::{pad_visual, pad_visual_right, visual_width};
use crate::output::style;
use crate::{Result, format, output};

/// A rendered cache row: the three columns the table shows.
struct Row {
    name: String,
    version: String,
    size: String,
}

/// Package cache management
#[derive(Debug, Parser)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List download caches
    #[clap(alias = "ls", alias = "show")]
    List {
        /// List caches matching the query
        query: Option<String>,
    },
    /// Remove download caches
    #[clap(alias = "rm")]
    #[clap(arg_required_else_help = true)]
    Remove {
        /// Remove caches matching the query
        query: Option<String>,
        /// Remove all caches
        #[arg(short, long, action = ArgAction::SetTrue, conflicts_with = "query")]
        all: bool,
    },
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    match args.command {
        Command::List { query } => {
            let query = query.unwrap_or("*".to_string());
            let files = cache::list(session, query.as_str())?;
            let mut total_size: u64 = 0;
            let mut rows = Vec::new();

            for f in files.into_iter() {
                // Skip entries deleted concurrently (NotFound) instead of
                // aborting the whole listing; other errors still surface.
                let metadata = match f.path().metadata() {
                    Ok(m) => m,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e.into()),
                };
                let size = metadata.len();
                total_size += size;

                rows.push(Row {
                    name: f.package_name().to_owned(),
                    version: f.version().to_owned(),
                    size: format::humansize(size, true),
                });
            }

            output::header(hok_i18n::t!("cmd.header_cache"));

            if rows.is_empty() {
                output::info(hok_i18n::t!("cmd.cache_empty"));
                return Ok(());
            }

            render(&rows);

            // The totals line sits *below* the rows it aggregates — the shape
            // upstream `scoop cache show` uses, and the placement rule every
            // other hok table follows (title on top, statistics at the bottom).
            println!();
            output::info(format!(
                "{} {}",
                hok_i18n::t!("cmd.cache_total_label"),
                hok_i18n::t!(
                    "cmd.cache_total",
                    count = rows.len(),
                    size = format::humansize(total_size, true)
                )
            ));

            Ok(())
        }
        Command::Remove { query, all } => {
            if all {
                match cache::remove(session, "*") {
                    Ok(_) => {
                        output::info(hok_i18n::t!("cmd.cache_all_removed"));
                        return Ok(());
                    }
                    Err(e) => return Err(e.into()),
                }
            }

            if let Some(query) = query {
                match cache::remove(session, query.as_str()) {
                    Ok(_) => {
                        if query == "*" {
                            output::info(hok_i18n::t!("cmd.cache_all_removed"));
                        } else {
                            output::info(hok_i18n::t!("cmd.cache_matching_removed", query = query));
                        }
                    }
                    Err(e) => return Err(e.into()),
                }
            }

            Ok(())
        }
    }
}

/// Lay out the cache table: an emphasized header, a dim dashed rule, then one
/// line per file.
///
/// `size` is a numeric column and is right-aligned; the last column is never
/// padded, so no rendered line carries trailing spaces.
fn layout(rows: &[Row], headers: [&str; 3]) -> Vec<String> {
    let [hdr_name, hdr_version, hdr_size] = headers;

    let w_name = visual_width(hdr_name).max(
        rows.iter()
            .map(|r| visual_width(&r.name))
            .max()
            .unwrap_or(0),
    );
    let w_version = visual_width(hdr_version).max(
        rows.iter()
            .map(|r| visual_width(&r.version))
            .max()
            .unwrap_or(0),
    );
    let w_size = visual_width(hdr_size).max(
        rows.iter()
            .map(|r| visual_width(&r.size))
            .max()
            .unwrap_or(0),
    );

    let mut lines = vec![
        format!(
            "  {}  {}  {}",
            style::emphasis(pad_visual(hdr_name, w_name)),
            style::emphasis(pad_visual(hdr_version, w_version)),
            style::emphasis(pad_visual_right(hdr_size, w_size)),
        ),
        format!(
            "  {}  {}  {}",
            style::dim("-".repeat(w_name)),
            style::dim("-".repeat(w_version)),
            style::dim("-".repeat(w_size)),
        ),
    ];

    for row in rows {
        lines.push(format!(
            "  {}  {}  {}",
            style::emphasis(pad_visual(&row.name, w_name)),
            pad_visual(&row.version, w_version),
            pad_visual_right(&row.size, w_size),
        ));
    }

    lines
}

/// Print the cache table with the localized headers.
fn render(rows: &[Row]) {
    let name = hok_i18n::t!("cmd.cache_header_name");
    let version = hok_i18n::t!("cmd.cache_header_version");
    let size = hok_i18n::t!("cmd.cache_header_size");

    for line in layout(rows, [name.as_str(), version.as_str(), size.as_str()]) {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, version: &str, size: &str) -> Row {
        Row {
            name: name.to_owned(),
            version: version.to_owned(),
            size: size.to_owned(),
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

    /// The visual column at which `value` starts in a rendered `line`.
    fn value_column(line: &str, value: &str) -> usize {
        let line = plain(line);
        let at = line.find(value).expect("the value is on the line");
        visual_width(&line[..at])
    }

    /// Covers the whole table: header, rule and data rows all end at the same
    /// visual width, CJK names do not shift the version column, and no line
    /// carries trailing spaces.
    #[test]
    fn layout_aligns_columns_with_cjk_names() {
        let rows = vec![
            row("7zip", "26.03", "2 MB"),
            row("directoryopus-cn", "13.25.3", "1011 KB"),
            row("中文包", "1.0", "512 KB"),
        ];
        let lines = layout(&rows, ["名称", "版本", "大小"]);
        let plain_lines = lines.iter().map(|l| plain(l)).collect::<Vec<_>>();

        let widths = plain_lines
            .iter()
            .map(|l| visual_width(l))
            .collect::<Vec<_>>();
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "uneven line widths: {widths:?}\n{plain_lines:#?}"
        );

        // Version values all start one gutter after the widest name column.
        let expected = 2 + visual_width("directoryopus-cn") + 2;
        for (line, version) in plain_lines[2..].iter().zip(["26.03", "13.25.3", "1.0"]) {
            assert_eq!(value_column(line, version), expected, "on `{line}`");
        }

        assert!(
            plain_lines.iter().all(|l| !l.ends_with(' ')),
            "trailing spaces: {plain_lines:#?}"
        );
    }

    /// The size column (header included) is right-aligned on a shared edge.
    #[test]
    fn layout_right_aligns_the_size_column() {
        let rows = vec![
            row("7zip", "26.03", "2 MB"),
            row("zstd", "1.5.7", "1011 KB"),
        ];
        let lines = layout(&rows, ["Name", "Version", "Size"])
            .iter()
            .map(|l| plain(l))
            .collect::<Vec<_>>();

        // Widest size is `1011 KB` (7 columns); `2 MB` (4) is padded on the left.
        assert!(lines[0].ends_with("   Size"), "header: {}", lines[0]);
        assert!(lines[1].ends_with("-------"), "rule: {}", lines[1]);
        assert!(lines[2].ends_with("   2 MB"), "row: {}", lines[2]);
        assert!(lines[3].ends_with("1011 KB"), "row: {}", lines[3]);
    }

    /// An empty listing still lays out the header and the rule.
    #[test]
    fn layout_of_no_rows_is_header_and_rule_only() {
        let lines = layout(&[], ["Name", "Version", "Size"]);
        assert_eq!(lines.len(), 2);
    }
}
