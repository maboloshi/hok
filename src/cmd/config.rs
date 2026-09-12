//! Get or set Scoop configuration values.

use clap::{Parser, Subcommand};
use libscoop::config::{ConfigCell, ConfigRow};
use libscoop::{Session, config};

use crate::format::{pad_visual, visual_width};
use crate::output::style;
use crate::{Result, output};

/// Configuration management
#[derive(Debug, Parser)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Edit the config file [default: system default editor]
    Edit,
    /// List all settings in key-value
    #[clap(alias = "ls")]
    List {
        /// Also show the effective default value of every setting
        #[arg(long)]
        all: bool,
    },
    /// Add a new setting to the config file
    #[clap(arg_required_else_help = true)]
    Set {
        /// The key of the config
        key: String,
        /// The value of the setting
        value: String,
    },
    /// Remove a setting from config file
    #[clap(arg_required_else_help = true)]
    Unset {
        /// The key of the setting
        key: String,
    },
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    match args.command {
        Command::Edit => {
            let path = &session.config().path;
            // `config::edit` launches `$EDITOR` (if set); fall back to the
            // system default handler otherwise.
            if !config::edit(session)? {
                libscoop::os::open_file(path)?;
            }
            Ok(())
        }
        Command::List { all } => {
            if all {
                // Column headers are localized here, in the CLI layer (i18n
                // lives here, not in the lib); alignment accounts for their
                // width.
                let rows = config::list_all_rows(session)?;
                let headers = [
                    hok_i18n::t!("cmd.config_col_key"),
                    hok_i18n::t!("cmd.config_col_value"),
                    hok_i18n::t!("cmd.config_col_default"),
                    hok_i18n::t!("cmd.config_col_desc"),
                ];

                output::header(hok_i18n::t!("cmd.header_config"));
                for line in layout(&rows, headers) {
                    println!("{line}");
                }
            } else {
                let config_json = config::list(session)?;
                output::info(session.config().path.display().to_string());
                println!("{}", config_json);
            }
            Ok(())
        }
        Command::Set { key, value } => {
            config::set(session, key.as_str(), value.as_str())?;
            output::info(hok_i18n::t!("cmd.config_set", key = key, value = value));
            Ok(())
        }
        Command::Unset { key } => {
            config::set(session, key.as_str(), "")?;
            output::info(hok_i18n::t!("cmd.config_unset", key = key));
            Ok(())
        }
    }
}

/// Render one value cell: the placeholder cells are localized, everything else
/// is the value itself (so it can be read back and passed to `config set`).
///
/// A list value is joined with the same separator `status` uses for its
/// multi-value cells.
fn cell_text(cell: &ConfigCell) -> String {
    match cell {
        ConfigCell::Unset => hok_i18n::t!("cmd.config_value_unset"),
        ConfigCell::Auto => hok_i18n::t!("cmd.config_value_auto"),
        ConfigCell::Text(text) => text.clone(),
        ConfigCell::List(items) => items.join(" · "),
    }
}

/// The short description of a setting key.
///
/// One literal `t!` per key instead of a dynamic `format!("config_desc.{key}")`
/// lookup: the i18n guard scans for literal keys (a dynamic one would show up
/// as a dead key), and a missing translation would then render the raw key
/// rather than fail the test.
fn description(key: &str) -> String {
    match key {
        "aria2-enabled" => hok_i18n::t!("cmd.config_desc_aria2_enabled"),
        "aria2-split" => hok_i18n::t!("cmd.config_desc_aria2_split"),
        "aria2-max-connection-per-server" => {
            hok_i18n::t!("cmd.config_desc_aria2_max_connection_per_server")
        }
        "jobs" => hok_i18n::t!("cmd.config_desc_jobs"),
        "aria2-min-split-size" => hok_i18n::t!("cmd.config_desc_aria2_min_split_size"),
        "cache_path" => hok_i18n::t!("cmd.config_desc_cache_path"),
        "cat_style" => hok_i18n::t!("cmd.config_desc_cat_style"),
        "editor" => hok_i18n::t!("cmd.config_desc_editor"),
        "default_architecture" => hok_i18n::t!("cmd.config_desc_default_architecture"),
        "global_path" => hok_i18n::t!("cmd.config_desc_global_path"),
        "gh_token" => hok_i18n::t!("cmd.config_desc_gh_token"),
        "ignore-failures" => hok_i18n::t!("cmd.config_desc_ignore_failures"),
        "ignore_running_processes" => hok_i18n::t!("cmd.config_desc_ignore_running_processes"),
        "language" => hok_i18n::t!("cmd.config_desc_language"),
        "last_update" => hok_i18n::t!("cmd.config_desc_last_update"),
        "no-color" => hok_i18n::t!("cmd.config_desc_no_color"),
        "confirm" => hok_i18n::t!("cmd.config_desc_confirm"),
        "script-engine" => hok_i18n::t!("cmd.config_desc_script_engine"),
        "no_junction" => hok_i18n::t!("cmd.config_desc_no_junction"),
        "private_hosts" => hok_i18n::t!("cmd.config_desc_private_hosts"),
        "proxy" => hok_i18n::t!("cmd.config_desc_proxy"),
        "root_path" => hok_i18n::t!("cmd.config_desc_root_path"),
        "use_isolated_path" => hok_i18n::t!("cmd.config_desc_use_isolated_path"),
        "use_sqlite_cache" => hok_i18n::t!("cmd.config_desc_use_sqlite_cache"),
        "virustotal_api_key" => hok_i18n::t!("cmd.config_desc_virustotal_api_key"),
        // A setting added to the lib without a description here still renders
        // (an empty last column), it just carries no explanation.
        _ => String::new(),
    }
}

/// Lay out the settings table: key | current value | default | description.
///
/// The description is the last column on purpose: the three value columns are
/// the ones that must line up, and their widths (paths, tokens) vary; a long
/// description at the end cannot push them out of alignment.
fn layout(rows: &[ConfigRow], headers: [String; 4]) -> Vec<String> {
    let [hdr_key, hdr_value, hdr_default, hdr_desc] = headers;

    let cells = rows
        .iter()
        .map(|row| {
            [
                row.key.clone(),
                cell_text(&row.value),
                cell_text(&row.default),
                description(&row.key),
            ]
        })
        .collect::<Vec<_>>();

    let column_width = |index: usize, header: &str| {
        cells
            .iter()
            .map(|cell| visual_width(&cell[index]))
            .max()
            .unwrap_or(0)
            .max(visual_width(header))
    };
    let w_key = column_width(0, &hdr_key);
    let w_value = column_width(1, &hdr_value);
    let w_default = column_width(2, &hdr_default);
    let w_desc = visual_width(&hdr_desc);

    let mut lines = vec![
        format!(
            "  {}  {}  {}  {}",
            style::emphasis(pad_visual(&hdr_key, w_key)),
            style::emphasis(pad_visual(&hdr_value, w_value)),
            style::emphasis(pad_visual(&hdr_default, w_default)),
            style::emphasis(&hdr_desc),
        ),
        format!(
            "  {}  {}  {}  {}",
            style::dim("-".repeat(w_key)),
            style::dim("-".repeat(w_value)),
            style::dim("-".repeat(w_default)),
            style::dim("-".repeat(w_desc)),
        ),
    ];

    for cell in &cells {
        lines.push(
            format!(
                "  {}  {}  {}  {}",
                style::emphasis(pad_visual(&cell[0], w_key)),
                pad_visual(&cell[1], w_value),
                pad_visual(&cell[2], w_default),
                cell[3],
            )
            .trim_end()
            .to_owned(),
        );
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str, value: ConfigCell, default: ConfigCell) -> ConfigRow {
        ConfigRow {
            key: key.to_owned(),
            value,
            default,
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

    /// Every row starts its value and default cells at the column the header
    /// declares, whatever the cell widths (a localized placeholder, a long
    /// path), and no line carries trailing spaces.
    #[test]
    fn layout_aligns_columns_under_the_header() {
        let rows = vec![
            row(
                "aria2-split",
                ConfigCell::Text("conn-7".to_owned()),
                ConfigCell::Text("def-5".to_owned()),
            ),
            row(
                "aria2-max-connection-per-server",
                ConfigCell::Text("9".to_owned()),
                ConfigCell::Text("5".to_owned()),
            ),
            row(
                "cache_path",
                ConfigCell::Text("path-A".to_owned()),
                ConfigCell::Unset,
            ),
            row("default_architecture", ConfigCell::Auto, ConfigCell::Auto),
            row(
                "private_hosts",
                ConfigCell::List(vec!["host-a".to_owned(), "host-b".to_owned()]),
                ConfigCell::Unset,
            ),
        ];
        let headers = ["配置项", "当前值", "默认值", "说明"].map(str::to_owned);
        let lines = layout(&rows, headers)
            .iter()
            .map(|l| plain(l))
            .collect::<Vec<_>>();

        assert!(
            lines.iter().all(|l| !l.ends_with(' ')),
            "trailing spaces: {lines:#?}"
        );

        // The header line is the oracle for the two columns it declares.
        let value_col = value_column(&lines[0], "当前值");
        let default_col = value_column(&lines[0], "默认值");

        assert_eq!(value_column(&lines[2], "conn-7"), value_col, "row 1 value");
        assert_eq!(
            value_column(&lines[2], "def-5"),
            default_col,
            "row 1 default"
        );
        assert_eq!(value_column(&lines[3], "9"), value_col, "row 2 value");
        assert_eq!(value_column(&lines[4], "path-A"), value_col, "row 3 value");
        assert_eq!(
            value_column(&lines[4], &cell_text(&ConfigCell::Unset)),
            default_col,
            "row 3 default placeholder"
        );
        // A list value renders element by element in the value column.
        assert_eq!(
            value_column(&lines[6], "host-a · host-b"),
            value_col,
            "row 5 value"
        );
    }

    /// The description column carries the localized text for a known key and
    /// leaves no gap for an unknown one.
    #[test]
    fn layout_carries_the_description_last() {
        let rows = vec![
            row(
                "jobs",
                ConfigCell::Text("4".to_owned()),
                ConfigCell::Text("4".to_owned()),
            ),
            row(
                "totally-unknown-key",
                ConfigCell::Text("x".to_owned()),
                ConfigCell::Unset,
            ),
        ];
        let headers = ["key", "value", "default", "description"].map(str::to_owned);
        let lines = layout(&rows, headers)
            .iter()
            .map(|l| plain(l))
            .collect::<Vec<_>>();

        assert!(
            lines[2].ends_with(&description("jobs")),
            "row 1: {}",
            lines[2]
        );
        assert!(!lines[3].ends_with(' '), "row 2: {}", lines[3]);
        assert_eq!(description("totally-unknown-key"), String::new());
    }

    /// Every setting the CLI describes has a key that the lib actually emits:
    /// the list below is the contract between `description` and
    /// `libscoop::config::list_all_rows`.
    #[test]
    fn every_supported_key_has_a_description() {
        for key in [
            "aria2-enabled",
            "aria2-split",
            "aria2-max-connection-per-server",
            "jobs",
            "aria2-min-split-size",
            "cache_path",
            "cat_style",
            "editor",
            "default_architecture",
            "global_path",
            "gh_token",
            "ignore-failures",
            "ignore_running_processes",
            "language",
            "last_update",
            "no-color",
            "confirm",
            "script-engine",
            "no_junction",
            "private_hosts",
            "proxy",
            "root_path",
            "use_isolated_path",
            "use_sqlite_cache",
            "virustotal_api_key",
        ] {
            assert!(!description(key).is_empty(), "no description for {key}");
        }
    }
}
