//! Get or set Scoop configuration values.

use clap::{Parser, Subcommand};
use libscoop::config::{ConfigCell, ConfigRow};
use libscoop::{Session, config};

use crate::format::{pad_visual, visual_width};
use crate::output::style;
use crate::{Result, output};

/// Configuration management
#[derive(Debug, Parser)]
#[command(args_conflicts_with_subcommands = true)]
pub struct Args {
    /// Print the settings as JSON
    #[arg(
        short = 'j',
        long = "json",
        action = clap::ArgAction::SetTrue,
        conflicts_with_all = ["all", "key", "value"]
    )]
    json: bool,
    /// List every setting with its effective default
    #[arg(
        short = 'a',
        long = "all",
        action = clap::ArgAction::SetTrue,
        conflicts_with_all = ["key", "value"]
    )]
    all: bool,
    /// Print this setting, or write it when a value follows
    key: Option<String>,
    /// The new value for <KEY>
    value: Option<String>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Edit the config file [default: system default editor]
    Edit,
    /// Remove a setting from the config file
    #[clap(arg_required_else_help = true)]
    Rm {
        /// The key of the setting
        key: String,
    },
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let Some(command) = args.command else {
        return match (args.key.as_deref(), args.value.as_deref()) {
            // `hok config <name> <value>` sets, like `scoop config <name> <value>`.
            (Some(key), Some(value)) => {
                config::set(session, key, value)?;
                output::info(hok_i18n::t!("cmd.config_set", key = key, value = value));
                Ok(())
            }
            // `hok config <name>` prints that setting.
            (Some(key), None) => print_value(session, key),
            // Bare `hok config` is the file's own settings; `-j` the same JSON,
            // `-a` every setting with its default.
            (None, _) if args.all => print_table(session),
            (None, _) if args.json => print_json(session),
            (None, _) => print_settings(session),
        };
    };
    match command {
        Command::Edit => {
            let path = &session.config().path;
            // `config::edit` launches `$EDITOR` (if set); fall back to the
            // system default handler otherwise.
            if !config::edit(session)? {
                libscoop::os::open_file(path)?;
            }
            Ok(())
        }
        Command::Rm { key } => {
            config::set(session, key.as_str(), "")?;
            output::info(hok_i18n::t!("cmd.config_removed", key = key));
            Ok(())
        }
    }
}

/// Print one setting, or upstream's `'<name>' is not set`.
fn print_value(session: &Session, key: &str) -> Result<()> {
    match config::value(session, key)? {
        Some(value) => println!("{}", render_value(&value)),
        None => output::info(hok_i18n::t!("cmd.config_not_set", key = key)),
    }
    Ok(())
}

/// One stored value as the CLI prints it: a string verbatim (so it can be fed
/// straight back as a `config <key> <value>` argument), everything else as
/// JSON.
fn render_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Bare `hok config`: the settings the config file actually holds, one
/// `key = value` line each — upstream's bare `scoop config` shows exactly the
/// file (`libexec/scoop-config.ps1:157-158`) rather than every known setting.
fn print_settings(session: &Session) -> Result<()> {
    let settings = file_settings(session)?;
    output::note(session.config().path.display().to_string());
    for line in settings_lines(&settings) {
        println!("{line}");
    }
    Ok(())
}

/// `-j` / `--json`: the same settings as [`print_settings`], as one JSON
/// document.
fn print_json(session: &Session) -> Result<()> {
    // The path is a note for the human on stderr, so stdout stays one JSON
    // document: `hok config -j | jq` must not see a stray first line.
    output::note(session.config().path.display().to_string());
    println!("{}", config::list(session)?);
    Ok(())
}

/// `-a` / `--all`: every known setting with its current value, its effective
/// default and a short description (the hok extension that answers "what can I
/// set, and to what?").
fn print_table(session: &Session) -> Result<()> {
    // Column headers are localized here, in the CLI layer (i18n lives here, not
    // in the lib); alignment accounts for their width.
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
    Ok(())
}

/// The config file's own JSON object.
fn file_settings(session: &Session) -> Result<serde_json::Map<String, serde_json::Value>> {
    let document: serde_json::Value = serde_json::from_str(&config::list(session)?)?;
    Ok(document.as_object().cloned().unwrap_or_default())
}

/// One `  key = value` line per setting, the keys aligned.
fn settings_lines(settings: &serde_json::Map<String, serde_json::Value>) -> Vec<String> {
    let width = settings
        .keys()
        .map(|key| visual_width(key))
        .max()
        .unwrap_or(0);
    settings
        .iter()
        .map(|(key, value)| {
            format!(
                "  {} {} {}",
                style::emphasis(pad_visual(key, width)),
                style::dim("="),
                render_value(value)
            )
        })
        .collect()
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
        "ignore_failures" => hok_i18n::t!("cmd.config_desc_ignore_failures"),
        "ignore_running_processes" => hok_i18n::t!("cmd.config_desc_ignore_running_processes"),
        "language" => hok_i18n::t!("cmd.config_desc_language"),
        "last_update" => hok_i18n::t!("cmd.config_desc_last_update"),
        "no_color" => hok_i18n::t!("cmd.config_desc_no_color"),
        "confirm" => hok_i18n::t!("cmd.config_desc_confirm"),
        "script_engine" => hok_i18n::t!("cmd.config_desc_script_engine"),
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
            style::accent_bold(pad_visual(&hdr_key, w_key)),
            style::accent_bold(pad_visual(&hdr_value, w_value)),
            style::accent_bold(pad_visual(&hdr_default, w_default)),
            style::accent_bold(&hdr_desc),
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
            "ignore_failures",
            "ignore_running_processes",
            "language",
            "last_update",
            "no_color",
            "confirm",
            "script_engine",
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

    /// `hok config <name>` prints a string verbatim (so it round-trips into
    /// `config <name> <value>`) and every other stored value in its JSON form.
    #[test]
    fn render_value_keeps_strings_verbatim() {
        assert_eq!(render_value(&serde_json::json!("C:\\scoop")), "C:\\scoop");
        assert_eq!(render_value(&serde_json::json!(true)), "true");
        assert_eq!(render_value(&serde_json::json!(5)), "5");
        assert_eq!(render_value(&serde_json::json!(["a", "b"])), r#"["a","b"]"#);
    }

    /// Bare `config` renders one `key = value` line per stored setting, with
    /// the keys padded so the value column starts at the same offset.
    #[test]
    fn settings_lines_align_the_value_column() {
        let settings: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
            r#"{"aria2-split":7,"no_junction":true,"proxy":"127.0.0.1:1080"}"#,
        )
        .unwrap();
        let lines: Vec<String> = settings_lines(&settings)
            .iter()
            .map(|line| plain(line))
            .collect();

        assert_eq!(lines[0], "  aria2-split = 7");
        assert_eq!(lines[1], "  no_junction = true");
        assert_eq!(lines[2], "  proxy       = 127.0.0.1:1080");
        let equals: Vec<usize> = lines.iter().map(|l| l.find('=').unwrap()).collect();
        assert!(
            equals.windows(2).all(|pair| pair[0] == pair[1]),
            "value column is not aligned: {lines:#?}"
        );
    }
}
