//! Manage buckets.

use clap::{ArgAction, Parser, Subcommand};
use libscoop::{Session, bucket};

use crate::format::{pad_visual, pad_visual_right, visual_width};
use crate::output::style;
use crate::{Result, output};

/// A rendered local-bucket row: the four columns the table shows.
struct Row {
    name: String,
    manifests: String,
    updated: String,
    source: String,
}

/// Manage manifest buckets
#[derive(Debug, Parser)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Add a bucket
    #[clap(arg_required_else_help = true)]
    Add {
        /// The bucket name
        name: String,
        /// The bucket repository url (optional for known buckets)
        repo: Option<String>,
    },
    /// List buckets
    #[clap(alias = "ls")]
    List {
        /// List known buckets
        #[arg(short = 'k', long, action = ArgAction::SetTrue)]
        known: bool,
    },
    /// List known buckets (Scoop: `scoop bucket known`)
    Known,
    /// Remove bucket(s)
    #[clap(alias = "rm")]
    #[clap(arg_required_else_help = true)]
    Remove {
        /// The bucket name(s)
        #[arg(required = true, action = ArgAction::Append)]
        name: Vec<String>,
    },
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    match args.command {
        Command::Add { name, repo } => {
            output::progress(hok_i18n::t!("cmd.adding_bucket"), &name);
            let repo = repo.as_deref().unwrap_or_default();
            match bucket::add(session, name.as_str(), repo).await {
                Ok(..) => output::ok(),
                // Scoop exits 2 — not 1 — when the bucket is already there:
                // `lib/buckets.ps1:132` returns 2 and
                // `libexec/scoop-bucket.ps1:50` propagates it through
                // `exit $status`.
                Err(libscoop::Error::BucketAlreadyExists(name)) => {
                    output::err(hok_i18n::t!("error.bucket_already_exists", name = name));
                    return Err(crate::ExitCode(2).into());
                }
                Err(err) => return Err(err.into()),
            }
            Ok(())
        }
        Command::List { known } => {
            if known {
                render_known(&bucket::list_known());
                Ok(())
            } else {
                match bucket::list(session) {
                    Err(e) => Err(e.into()),
                    Ok(buckets) => {
                        // Scoop warns and exits 2 when nothing is added
                        // (`libexec/scoop-bucket.ps1:61-70`), which scripts use
                        // to tell "no buckets" from an ordinary listing.
                        if buckets.is_empty() {
                            output::warn(hok_i18n::t!("cmd.bucket_list_empty"));
                            return Err(crate::ExitCode(2).into());
                        }
                        let rows = buckets
                            .iter()
                            .map(|b| Row {
                                name: b.name().to_owned(),
                                manifests: b.manifest_count().to_string(),
                                // A bucket cloned without git (or added from a
                                // directory) has no commit date; that column
                                // stays empty rather than printing a placeholder.
                                updated: b.updated_at().unwrap_or_default().to_owned(),
                                source: b.source().to_owned(),
                            })
                            .collect::<Vec<_>>();

                        output::header(hok_i18n::t!("cmd.header_buckets_local"));
                        render(&rows);
                        Ok(())
                    }
                }
            }
        }
        Command::Known => {
            render_known(&bucket::list_known());
            Ok(())
        }
        Command::Remove { name } => {
            for name in name {
                output::progress(hok_i18n::t!("cmd.removing_bucket"), &name);
                match bucket::remove(session, name.as_str()) {
                    Ok(..) => output::ok(),
                    Err(err) => return Err(err.into()),
                }
            }
            Ok(())
        }
    }
}

/// Lay out the added-bucket table: an emphasized header, a dim dashed rule,
/// then one line per bucket.
///
/// `manifests` is a numeric column (right-aligned) and `source` is last, so the
/// long repository URL never pads and no rendered line carries trailing spaces.
/// The rule under the last column spans the header width only — the same rule
/// `list` applies to its `Info` column.
fn layout(rows: &[Row], headers: [&str; 4]) -> Vec<String> {
    let [hdr_name, hdr_manifests, hdr_updated, hdr_source] = headers;

    let w_name = visual_width(hdr_name).max(
        rows.iter()
            .map(|r| visual_width(&r.name))
            .max()
            .unwrap_or(0),
    );
    let w_manifests = visual_width(hdr_manifests).max(
        rows.iter()
            .map(|r| visual_width(&r.manifests))
            .max()
            .unwrap_or(0),
    );
    let w_updated = visual_width(hdr_updated).max(
        rows.iter()
            .map(|r| visual_width(&r.updated))
            .max()
            .unwrap_or(0),
    );
    let w_source = visual_width(hdr_source);

    let mut lines = vec![
        format!(
            "  {}  {}  {}  {}",
            style::accent_bold(pad_visual(hdr_name, w_name)),
            style::accent_bold(pad_visual_right(hdr_manifests, w_manifests)),
            style::accent_bold(pad_visual(hdr_updated, w_updated)),
            style::accent_bold(hdr_source),
        ),
        format!(
            "  {}  {}  {}  {}",
            style::dim("-".repeat(w_name)),
            style::dim("-".repeat(w_manifests)),
            style::dim("-".repeat(w_updated)),
            style::dim("-".repeat(w_source)),
        ),
    ];

    for row in rows {
        lines.push(format!(
            "  {}  {}  {}  {}",
            style::emphasis(pad_visual(&row.name, w_name)),
            pad_visual_right(&row.manifests, w_manifests),
            pad_visual(&row.updated, w_updated),
            row.source,
        ));
    }

    lines
}

/// Lay out the known-bucket table: name plus repository URL.
fn layout_known(buckets: &[(&str, &str)], headers: [&str; 2]) -> Vec<String> {
    let [hdr_name, hdr_repo] = headers;

    let w_name = visual_width(hdr_name).max(
        buckets
            .iter()
            .map(|(name, _)| visual_width(name))
            .max()
            .unwrap_or(0),
    );
    let w_repo = visual_width(hdr_repo);

    let mut lines = vec![
        format!(
            "  {}  {}",
            style::accent_bold(pad_visual(hdr_name, w_name)),
            style::accent_bold(hdr_repo),
        ),
        format!(
            "  {}  {}",
            style::dim("-".repeat(w_name)),
            style::dim("-".repeat(w_repo)),
        ),
    ];

    for (name, repo) in buckets {
        lines.push(format!(
            "  {}  {}",
            style::emphasis(pad_visual(name, w_name)),
            repo
        ));
    }

    lines
}

/// Print the added-bucket table with the localized headers.
fn render(rows: &[Row]) {
    let name = hok_i18n::t!("cmd.bucket_header_name");
    let manifests = hok_i18n::t!("cmd.bucket_header_manifests");
    let updated = hok_i18n::t!("cmd.bucket_header_updated");
    let source = hok_i18n::t!("cmd.bucket_header_source");

    for line in layout(
        rows,
        [
            name.as_str(),
            manifests.as_str(),
            updated.as_str(),
            source.as_str(),
        ],
    ) {
        println!("{line}");
    }
}

/// Print the known-bucket table (shared by `bucket known` and `bucket list -k`).
fn render_known(buckets: &[(&str, &str)]) {
    let name = hok_i18n::t!("cmd.bucket_header_name");
    let repo = hok_i18n::t!("cmd.bucket_header_repo");

    output::header(hok_i18n::t!("cmd.header_buckets_list"));
    for line in layout_known(buckets, [name.as_str(), repo.as_str()]) {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, manifests: &str, updated: &str, source: &str) -> Row {
        Row {
            name: name.to_owned(),
            manifests: manifests.to_owned(),
            updated: updated.to_owned(),
            source: source.to_owned(),
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

    /// Header, rule and data rows share one layout, CJK bucket names do not
    /// shift the later columns, and the URL-last row carries no trailing spaces.
    #[test]
    fn layout_aligns_columns_with_cjk_names() {
        let rows = vec![
            row(
                "extras",
                "2383",
                "2026/9/12 0:31:15",
                "https://github.com/ScoopInstaller/Extras",
            ),
            row(
                "main",
                "1644",
                "2026/9/12 0:31:14",
                "https://github.com/ScoopInstaller/Main",
            ),
            row("中文仓库", "58", "", "D:\\本地\\bucket"),
        ];
        let lines = layout(&rows, ["名称", "清单数", "更新时间", "来源"])
            .iter()
            .map(|l| plain(l))
            .collect::<Vec<_>>();

        // The widest name is the CJK one (`中文仓库` = 8 columns).
        let manifests_col = 2 + visual_width("中文仓库") + 2;
        // `manifests` is right-aligned: its *right edge* is what lines up.
        let manifests_end = manifests_col + visual_width("清单数");
        let updated_col = manifests_end + 2;

        assert_eq!(value_column(&lines[0], "清单数"), manifests_col, "header");
        for (line, manifests) in lines[2..].iter().zip(["2383", "1644", "58"]) {
            assert_eq!(
                value_column(line, manifests) + visual_width(manifests),
                manifests_end,
                "on `{line}`"
            );
        }

        assert_eq!(value_column(&lines[0], "更新时间"), updated_col, "header");
        for (line, updated) in lines[2..]
            .iter()
            .zip(["2026/9/12 0:31:15", "2026/9/12 0:31:14"])
        {
            assert_eq!(value_column(line, updated), updated_col, "on `{line}`");
        }

        assert!(
            lines.iter().all(|l| !l.ends_with(' ')),
            "trailing spaces: {lines:#?}"
        );
    }

    /// The manifest count is right-aligned, header included.
    #[test]
    fn layout_right_aligns_the_manifest_count() {
        let rows = vec![
            row("extras", "2383", "2026/9/12", "https://e"),
            row("main", "1644", "2026/9/12", "https://m"),
            row("scoop-app", "17597", "2026/9/12", "https://s"),
        ];
        let lines = layout(&rows, ["Name", "Manifests", "Updated", "Source"])
            .iter()
            .map(|l| plain(l))
            .collect::<Vec<_>>();

        // Widest count is `17597` (5); the header (9) is wider, so every cell is
        // padded to 9 and both `2383` and `1644` gain leading spaces.
        assert!(lines[0].contains("Manifests"), "header: {}", lines[0]);
        assert!(lines[2].contains("     2383"), "row: {}", lines[2]);
        assert!(lines[3].contains("     1644"), "row: {}", lines[3]);
        assert!(lines[4].contains("    17597"), "row: {}", lines[4]);
    }

    /// A bucket without a git commit date leaves that column blank.
    #[test]
    fn layout_leaves_an_unknown_update_date_empty() {
        let rows = vec![row("local", "3", "", "D:\\bucket")];
        let lines = layout(&rows, ["Name", "Manifests", "Updated", "Source"])
            .iter()
            .map(|l| plain(l))
            .collect::<Vec<_>>();

        assert!(lines[2].ends_with("  D:\\bucket"), "row: {}", lines[2]);
        assert!(!lines[2].contains("none"), "row: {}", lines[2]);
    }

    /// The known-bucket table lines its repository column up under the header.
    #[test]
    fn known_layout_aligns_repository_column() {
        let buckets = [
            ("extras", "https://github.com/ScoopInstaller/Extras"),
            (
                "nerd-fonts",
                "https://github.com/matthewjberger/scoop-nerd-fonts",
            ),
        ];
        let lines = layout_known(&buckets, ["名称", "仓库"])
            .iter()
            .map(|l| plain(l))
            .collect::<Vec<_>>();

        let repo_col = 2 + visual_width("nerd-fonts") + 2;
        assert_eq!(value_column(&lines[0], "仓库"), repo_col, "header");
        assert_eq!(value_column(&lines[2], buckets[0].1), repo_col, "row 1");
        assert_eq!(value_column(&lines[3], buckets[1].1), repo_col, "row 2");

        assert!(lines.iter().all(|l| !l.ends_with(' ')), "{lines:#?}");
    }
}
