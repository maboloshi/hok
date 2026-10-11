//! Display information about a package.
//!
//! The field set, order and values mirror upstream `scoop info`
//! (libexec/scoop-info.ps1) — including `-v/--verbose`, which adds the
//! manifest path, the install layout with on-disk sizes, and the download size
//! of a not-installed app. The data comes from [`libscoop::package::info`];
//! this module only renders it in the CLI's one style.

use clap::{ArgAction, Parser};
use libscoop::Session;
use libscoop::package::info::{self, DownloadSize, DownloadSizeError, Info, InstalledSize, Lookup};

use crate::format::{filesize, visual_width};
use crate::output::style;
use crate::{Result, output};

/// Show an app's information
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// The app name (`bucket/name` also accepted), a manifest URL or a local
    /// manifest path
    query: String,
    /// Show full paths and URLs
    // A count, not a flag: `-v` is the CLI's global verbosity option, so
    // `hok info -v` and `hok -v info` share it (as `ci-auto-pr` already does).
    // Kept out of the doc comment so clap has a single (localizable) help line.
    #[arg(short = 'v', long, action = ArgAction::Count)]
    verbose: u8,
}

/// One rendered field: a label plus one or more value lines.
struct Field {
    label: String,
    lines: Vec<String>,
}

/// Print a package's detail block (the `info` body).
fn print_package(info: &Info, session: &Session) {
    let mut fields: Vec<Field> = Vec::new();

    // Upstream appends the marker to `Name` (scoop-info.ps1:79-81).
    let mut name = info.name.clone();
    if info.deprecated {
        name.push_str(" (DEPRECATED)");
    }
    fields.push(field(hok_i18n::t!("cmd.info_name"), vec![name]));

    if let Some(description) = &info.description {
        fields.push(field(
            hok_i18n::t!("cmd.info_description"),
            vec![description.clone()],
        ));
    }

    fields.push(field(
        hok_i18n::t!("cmd.info_version"),
        vec![match &info.version {
            info::VersionOutput::Manifest(version) | info::VersionOutput::Installed(version) => {
                version.clone()
            }
            info::VersionOutput::UpdateAvailable { installed, latest } => hok_i18n::t!(
                "cmd.info_update_available",
                installed = installed,
                latest = latest
            )
            .to_string(),
        }],
    ));

    fields.push(field(
        hok_i18n::t!("cmd.info_source"),
        vec![info.source.clone()],
    ));

    if let Some(website) = &info.website {
        fields.push(field(
            hok_i18n::t!("cmd.info_website"),
            vec![website.clone()],
        ));
    }
    if let Some(license) = &info.license {
        fields.push(field(
            hok_i18n::t!("cmd.info_license"),
            vec![license.clone()],
        ));
    }
    if let Some(dependencies) = &info.dependencies {
        fields.push(field(
            hok_i18n::t!("cmd.info_dependencies"),
            vec![dependencies.clone()],
        ));
    }
    if let Some(updated_at) = &info.updated_at {
        fields.push(field(
            hok_i18n::t!("cmd.info_updated_at"),
            vec![updated_at.clone()],
        ));
    }
    if let Some(updated_by) = &info.updated_by {
        fields.push(field(
            hok_i18n::t!("cmd.info_updated_by"),
            vec![updated_by.clone()],
        ));
    }
    if let Some(manifest_file) = &info.manifest_file {
        fields.push(field(
            hok_i18n::t!("cmd.info_manifest"),
            vec![manifest_file.clone()],
        ));
    }

    if let Some(installed) = &info.installed {
        // Upstream marks every version of a global install with ` *global*`
        // (scoop-info.ps1:144).
        let global = session.is_global();
        let rows = installed
            .iter()
            .map(|version| {
                if global {
                    format!("{version} *global*")
                } else {
                    version.clone()
                }
            })
            .collect::<Vec<_>>();
        fields.push(field(hok_i18n::t!("cmd.info_installed"), rows));
    }

    if let Some(size) = &info.installed_size {
        fields.push(field(
            hok_i18n::t!("cmd.info_installed_size"),
            size_lines(size),
        ));
    }
    if let Some(size) = &info.download_size {
        fields.push(field(
            hok_i18n::t!("cmd.info_download_size"),
            vec![download_size_line(size)],
        ));
    }

    if let Some(binaries) = &info.binaries {
        fields.push(field(
            hok_i18n::t!("cmd.info_binaries"),
            vec![binaries.clone()],
        ));
    }
    if let Some(shortcuts) = &info.shortcuts {
        fields.push(field(
            hok_i18n::t!("cmd.info_shortcuts"),
            vec![shortcuts.clone()],
        ));
    }
    if let Some(environment) = &info.environment {
        fields.push(field(
            hok_i18n::t!("cmd.info_environment"),
            environment.clone(),
        ));
    }
    if let Some(path_added) = &info.path_added {
        fields.push(field(
            hok_i18n::t!("cmd.info_path_added"),
            path_added.clone(),
        ));
    }
    if let Some(suggestions) = &info.suggestions {
        fields.push(field(
            hok_i18n::t!("cmd.info_suggestions"),
            vec![suggestions.clone()],
        ));
    }
    if let Some(notes) = &info.notes {
        fields.push(field(hok_i18n::t!("cmd.info_notes"), notes.clone()));
    }

    render(&fields);
}

/// A field with a single value line.
fn field(label: String, lines: Vec<String>) -> Field {
    Field { label, lines }
}

/// Lay out the label/value block with one aligned label column, so a
/// multi-line value (installed versions, environment, notes) reads as one
/// field.
///
/// Every line's value starts at the same column — the label column plus one
/// separating space — including the continuation lines of a multi-line value.
/// An empty continuation line stays empty instead of being padded out with
/// spaces: upstream joins `notes` with "`n" and prints them as they are, blank
/// separator lines included.
fn layout(fields: &[Field]) -> Vec<String> {
    let width = fields
        .iter()
        .map(|field| visual_width(&field.label))
        .max()
        .unwrap_or(0);

    let mut lines = Vec::new();
    for field in fields {
        for (index, line) in field.lines.iter().enumerate() {
            if index == 0 {
                let pad = " ".repeat(width - visual_width(&field.label));
                lines.push(format!(
                    "  {}{} {}",
                    style::accent_bold(&field.label),
                    pad,
                    line
                ));
            } else if line.is_empty() {
                lines.push(String::new());
            } else {
                // The value column of the first line: two spaces of indent,
                // the label column, and its separating space.
                lines.push(format!("  {:width$}{}", "", line, width = width + 1));
            }
        }
    }
    lines
}

/// Print the block built by [`layout`].
fn render(fields: &[Field]) {
    for line in layout(fields) {
        println!("{line}");
    }
}

/// Upstream's `Installed size` value (scoop-info.ps1:173-195): a bare total
/// when there are no old versions, persisted data or cached downloads, else
/// the non-zero categories.
fn size_lines(size: &InstalledSize) -> Vec<String> {
    if size.old + size.persisted + size.cached == 0 {
        return vec![filesize(size.current)];
    }

    let rows = [
        (hok_i18n::t!("cmd.info_size_current_version"), size.current),
        (hok_i18n::t!("cmd.info_size_old_versions"), size.old),
        (hok_i18n::t!("cmd.info_size_persisted_data"), size.persisted),
        (hok_i18n::t!("cmd.info_size_cached_downloads"), size.cached),
        (hok_i18n::t!("cmd.info_size_total"), size.total),
    ];
    let width = rows
        .iter()
        .map(|(label, _)| visual_width(label.as_ref()))
        .max()
        .unwrap_or(0);

    rows.into_iter()
        .filter(|(_, bytes)| *bytes != 0)
        .map(|(label, bytes)| {
            format!(
                "{}{} {}",
                label,
                " ".repeat(width - visual_width(label.as_ref())),
                filesize(bytes)
            )
        })
        .collect()
}

/// Upstream's `Download size` value (scoop-info.ps1:221-225).
fn download_size_line(size: &DownloadSize) -> String {
    let cached = if size.cached {
        hok_i18n::t!("cmd.info_download_cached").to_string()
    } else {
        String::new()
    };

    let reason = match &size.error {
        Some(DownloadSizeError::NoContentLength { host }) => {
            hok_i18n::t!("cmd.info_download_no_content_length", host = host).to_string()
        }
        Some(DownloadSizeError::Down { host }) => {
            hok_i18n::t!("cmd.info_download_server_down", host = host).to_string()
        }
        None => String::new(),
    };

    match size.bytes {
        Some(bytes) => format!("{}{cached}", filesize(bytes)),
        None => format!(
            "{}{cached}",
            hok_i18n::t!("cmd.info_download_unknown", reason = reason)
        ),
    }
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let query = args.query;

    match info::lookup(session, &query, args.verbose > 0).await? {
        Lookup::NotFound => Err(anyhow::anyhow!(hok_i18n::t!(
            "cmd.info_not_found",
            query = query
        ))),
        Lookup::Single(info) => {
            print_package(&info, session);
            Ok(())
        }
        Lookup::Installed(infos) => {
            let total = infos.len();
            for (index, info) in infos.iter().enumerate() {
                print_package(info, session);
                if index != total - 1 {
                    println!();
                }
            }
            Ok(())
        }
        Lookup::Candidates(packages) => {
            let length = packages.len();
            output::info(hok_i18n::t!("cmd.found", count = length, query = query));
            const CAP: usize = 30;
            for pkg in packages.iter().take(CAP) {
                println!("  {}  {}", pkg.ident(), pkg.version());
            }
            if length > CAP {
                output::info(hok_i18n::t!("cmd.info_more", n = length - CAP));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::plain;

    /// The visual column at which `value` starts in a rendered `line`.
    fn value_column(line: &str, value: &str) -> usize {
        let line = plain(line);
        let at = line.find(value).expect("the value is on the line");
        visual_width(&line[..at])
    }

    /// Every line — including the continuation lines of a multi-line value —
    /// starts its value at the same column, whatever the label widths are.
    #[test]
    fn values_align_across_labels_and_continuation_lines() {
        let fields = vec![
            Field {
                label: "名称".to_owned(),
                lines: vec!["git".to_owned()],
            },
            Field {
                label: "Installed size".to_owned(),
                lines: vec![
                    "当前版本:   389.1 MB".to_owned(),
                    "持久化数据: 357 B".to_owned(),
                ],
            },
            Field {
                label: "备注".to_owned(),
                lines: vec!["first line".to_owned(), "second line".to_owned()],
            },
        ];

        // The widest label is `Installed size` (14 columns), so the value
        // column is 2 + 14 + 1.
        let expected = 2 + visual_width("Installed size") + 1;
        for line in layout(&fields) {
            let value = match line.as_str() {
                line if line.contains("git") => "git",
                line if line.contains("389.1 MB") => "当前版本:   389.1 MB",
                line if line.contains("357 B") => "持久化数据: 357 B",
                line if line.contains("first line") => "first line",
                line if line.contains("second line") => "second line",
                other => panic!("unexpected line: {other}"),
            };
            assert_eq!(value_column(&line, value), expected, "on `{line}`");
        }
    }

    /// A blank separator line inside `notes` is printed as an empty line, not
    /// as indentation trailing into nothing.
    #[test]
    fn blank_continuation_lines_carry_no_trailing_spaces() {
        let fields = vec![Field {
            label: "备注".to_owned(),
            lines: vec!["paragraph".to_owned(), String::new(), "more".to_owned()],
        }];

        let lines = layout(&fields);
        assert_eq!(lines.len(), 3);
        assert!(lines[1].is_empty(), "blank line was `{}`", lines[1]);
    }
}
