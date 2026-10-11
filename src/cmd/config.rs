//! Get or set Scoop configuration values.

use clap::{Parser, Subcommand};
use libscoop::config::{Config, IsolatedPath};
use libscoop::{Session, config, os};
use std::process::Stdio;

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
                // `cat_style` is handed to `bat --style` verbatim, so the value
                // is written only once bat says it accepts it: refusing here
                // beats a `hok cat` that has to fall back to plain JSON every
                // run. An empty value unsets the setting, which is always fine.
                if key == "cat_style"
                    && !value.is_empty()
                    && let Some(reason) = bat_style_rejection(value)
                {
                    return Err(anyhow::anyhow!(hok_i18n::t!(
                        "cmd.config_cat_style_invalid",
                        value = value,
                        reason = reason
                    )));
                }
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
            // Launching the editor is a command action, so it lives here;
            // `libscoop::os::open_file` is the system-handler fallback.
            if !edit_config(session)? {
                libscoop::os::open_file(&session.config().path)?;
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

/// Open the config file in the user's configured editor.
///
/// Launches the `editor` config key — or `$EDITOR` when that is unset — with
/// the config file path as its argument and waits for the editor to exit.
/// Returns `Ok(false)` when neither is set, so the caller can fall back to
/// opening the file with the system default handler.
///
/// The editor is first spawned directly (passing the config path as UTF-16 on
/// Windows, so non-ASCII paths — e.g. a CJK username — reach the editor
/// intact). If that fails — the value is a command line such as `code --wait`
/// or a `.cmd`/`.bat` script, which `std::process::Command` cannot start
/// directly — it is executed through `cmd /c`.
///
/// `hok config edit` itself is a hok extension with no upstream counterpart:
/// official Scoop's `scoop config edit` reads `edit` as a *key name* and
/// answers `'edit' is not set` (`libexec/scoop-config.ps1:178-188`), and it
/// never reads `$EDITOR` anywhere.
fn edit_config(session: &Session) -> Result<bool> {
    let config = session.config();
    // The `editor` config key wins; `$EDITOR` is the fallback, and the
    // system default handler is used when neither is set.
    let command = config
        .editor()
        .map(str::to_owned)
        .or_else(|| std::env::var("EDITOR").ok())
        .filter(|s| !s.trim().is_empty());
    match command {
        Some(editor) => {
            let path = config.path.clone();
            match std::process::Command::new(&editor).arg(&path).spawn() {
                Ok(mut child) => {
                    child.wait()?;
                    Ok(true)
                }
                Err(_) => {
                    let quoted = format!("\"{}\"", path.display());
                    let mut child = std::process::Command::new("cmd")
                        .args(["/c", &editor, &quoted])
                        .spawn()?;
                    child.wait()?;
                    Ok(true)
                }
            }
        }
        None => Ok(false),
    }
}

/// bat's rejection of `value` as a `--style` value, or `None` when there is
/// nothing to reject.
///
/// `cat_style` is not hok's vocabulary: it goes to `bat --style` verbatim, so
/// the accepted values are the installed bat's own and change with it
/// (components were added across its releases, and a `+`/`-` prefixed list
/// modifies the previous set instead of replacing it). A list frozen here
/// would reject a newer bat's values, so the installed bat is asked — through
/// the same [`super::bat_command`] that `cat` renders with.
///
/// bat not being on PATH is deliberately **not** a rejection: there is nothing
/// to ask, and `cat` reports an unusable bat at the point where one is actually
/// needed. A bat that cannot even be started gets the same treatment — that is
/// a fact about the machine, not about the value.
fn bat_style_rejection(value: &str) -> Option<String> {
    if !os::is_program_available("bat.exe") {
        return None;
    }
    let output = super::bat_command(value)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    if output.status.success() {
        return None;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    Some(if stderr.is_empty() {
        format!(
            "bat exited with code {}",
            output.status.code().unwrap_or(-1)
        )
    } else {
        stderr.to_string()
    })
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
    let rows = list_all_rows(session);
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

/// One cell of the config listing.
///
/// The CLI localizes the placeholder cells (`Unset`, `Auto`) and prints the
/// real ones verbatim. Collapsing `Unset` into the literal `"none"` would make
/// the placeholder indistinguishable from a real value — `proxy = none` is a
/// legitimate setting.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ConfigCell {
    /// The setting has no value.
    Unset,
    /// Resolved at runtime by hok (the architecture).
    Auto,
    /// A concrete value the user can read back and set verbatim.
    Text(String),
    /// A list value, rendered one element per entry.
    List(Vec<String>),
}

impl ConfigCell {
    fn text(value: impl Into<String>) -> ConfigCell {
        ConfigCell::Text(value.into())
    }

    fn optional(value: Option<String>) -> ConfigCell {
        value.map_or(ConfigCell::Unset, ConfigCell::Text)
    }
}

/// One row of [`list_all_rows`]: the key, its current value and its built-in
/// default.
struct ConfigRow {
    key: String,
    value: ConfigCell,
    default: ConfigCell,
}

/// Every supported setting with its current value and effective default.
///
/// The current value is what hok actually uses at runtime (defaults apply when
/// the key is unset); the default column shows the built-in default, including
/// environment-resolved paths (`$SCOOP`, `$SCOOP_CACHE`, ...).
///
/// Building the listing is the CLI's own rendering job: the library exposes the
/// typed accessors it reads ([`Config::default_root_path`] and friends for the
/// built-in path defaults) instead of the table itself.
fn list_all_rows(session: &Session) -> Vec<ConfigRow> {
    let config = session.config();

    let row = |key: &str, value: ConfigCell, default: ConfigCell| ConfigRow {
        key: key.to_owned(),
        value,
        default,
    };
    let mut rows = vec![row(
        "aria2-enabled",
        ConfigCell::text(fmt_bool(config.aria2_enabled())),
        ConfigCell::text("true"),
    )];
    rows.push(row(
        "aria2-split",
        ConfigCell::text(config.aria2_split().to_string()),
        ConfigCell::text("5"),
    ));
    rows.push(row(
        "aria2-max-connection-per-server",
        ConfigCell::text(config.aria2_max_connection_per_server().to_string()),
        ConfigCell::text("5"),
    ));
    rows.push(row(
        "jobs",
        ConfigCell::text(config.jobs().to_string()),
        ConfigCell::text("4"),
    ));
    rows.push(row(
        "aria2-min-split-size",
        ConfigCell::text(fmt_size(config.aria2_min_split_size())),
        ConfigCell::text("20M"),
    ));
    rows.push(row(
        "cache_path",
        ConfigCell::text(config.cache_path().display().to_string()),
        ConfigCell::text(Config::default_cache_path().display().to_string()),
    ));

    let cat_style = config.cat_style();
    rows.push(row(
        "cat_style",
        if cat_style.is_empty() {
            ConfigCell::Unset
        } else {
            ConfigCell::text(cat_style)
        },
        ConfigCell::Unset,
    ));
    rows.push(row(
        "editor",
        ConfigCell::optional(config.editor().map(str::to_owned)),
        ConfigCell::Unset,
    ));
    rows.push(row(
        "default_architecture",
        config
            .default_architecture()
            .map_or(ConfigCell::Auto, ConfigCell::text),
        ConfigCell::Auto,
    ));
    rows.push(row(
        "global_path",
        ConfigCell::text(config.global_path().display().to_string()),
        ConfigCell::text(Config::default_global_path().display().to_string()),
    ));
    rows.push(row(
        "gh_token",
        ConfigCell::optional(config.gh_token.clone()),
        ConfigCell::Unset,
    ));
    rows.push(row(
        "ignore_failures",
        ConfigCell::text(fmt_bool(config.ignore_failures())),
        ConfigCell::text("false"),
    ));
    rows.push(row(
        "ignore_running_processes",
        ConfigCell::text(fmt_bool(config.ignore_running_processes())),
        ConfigCell::text("false"),
    ));
    rows.push(row(
        "language",
        ConfigCell::text(config.language()),
        ConfigCell::text("auto"),
    ));
    rows.push(row(
        "last_update",
        ConfigCell::optional(config.last_update().map(str::to_owned)),
        ConfigCell::Unset,
    ));
    rows.push(row(
        "no_color",
        ConfigCell::text(fmt_bool(config.no_color())),
        ConfigCell::text("false"),
    ));
    rows.push(row(
        "confirm",
        ConfigCell::text(fmt_bool(config.confirm())),
        ConfigCell::text("false"),
    ));
    rows.push(row(
        "script_engine",
        ConfigCell::text(fmt_bool(config.script_engine())),
        ConfigCell::text("false"),
    ));
    rows.push(row(
        "no_junction",
        ConfigCell::text(fmt_bool(config.no_junction())),
        ConfigCell::text("false"),
    ));
    rows.push(row(
        "autostash_on_conflict",
        ConfigCell::text(fmt_bool(config.autostash_on_conflict())),
        ConfigCell::text("false"),
    ));
    rows.push(row(
        "private_hosts",
        // The host *match patterns* only: `headers` carries the auth tokens
        // these hosts need, and this listing must never print them.
        match config.private_hosts() {
            Some(hosts) if !hosts.is_empty() => ConfigCell::List(
                hosts
                    .iter()
                    .map(|host| host.match_pattern().to_owned())
                    .collect(),
            ),
            _ => ConfigCell::Unset,
        },
        ConfigCell::Unset,
    ));
    rows.push(row(
        "proxy",
        ConfigCell::optional(config.proxy().map(str::to_owned)),
        ConfigCell::Unset,
    ));
    rows.push(row(
        "root_path",
        ConfigCell::text(config.root_path().display().to_string()),
        ConfigCell::text(Config::default_root_path().display().to_string()),
    ));

    let isolated = config.use_isolated_path();
    rows.push(row(
        "use_git_history",
        ConfigCell::text(fmt_bool(config.use_git_history())),
        ConfigCell::text("true"),
    ));
    rows.push(row(
        "use_isolated_path",
        match isolated {
            None => ConfigCell::Unset,
            Some(IsolatedPath::Boolean(b)) => ConfigCell::text(fmt_bool(*b)),
            Some(IsolatedPath::Named(name)) => ConfigCell::text(name.clone()),
        },
        ConfigCell::Unset,
    ));
    rows.push(row(
        "use_sqlite_cache",
        ConfigCell::text(fmt_bool(config.use_sqlite_cache())),
        ConfigCell::text("false"),
    ));
    rows.push(row(
        "virustotal_api_key",
        ConfigCell::optional(config.virustotal_api_key.clone()),
        ConfigCell::Unset,
    ));

    rows
}

fn fmt_bool(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// Format a byte size back into the `aria2-min-split-size` style (5M, 10M).
fn fmt_size(bytes: u64) -> String {
    const G: u64 = 1024 * 1024 * 1024;
    const M: u64 = 1024 * 1024;
    const K: u64 = 1024;
    if bytes >= G && bytes.is_multiple_of(G) {
        format!("{}G", bytes / G)
    } else if bytes >= M && bytes.is_multiple_of(M) {
        format!("{}M", bytes / M)
    } else if bytes >= K && bytes.is_multiple_of(K) {
        format!("{}K", bytes / K)
    } else {
        bytes.to_string()
    }
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
        "autostash_on_conflict" => hok_i18n::t!("cmd.config_desc_autostash_on_conflict"),
        "private_hosts" => hok_i18n::t!("cmd.config_desc_private_hosts"),
        "proxy" => hok_i18n::t!("cmd.config_desc_proxy"),
        "root_path" => hok_i18n::t!("cmd.config_desc_root_path"),
        "use_git_history" => hok_i18n::t!("cmd.config_desc_use_git_history"),
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
    use crate::test_support::plain;

    fn row(key: &str, value: ConfigCell, default: ConfigCell) -> ConfigRow {
        ConfigRow {
            key: key.to_owned(),
            value,
            default,
        }
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

    /// Every setting the CLI describes has a key that [`list_all_rows`] emits:
    /// the list below is the contract between `description` and that table.
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
            "autostash_on_conflict",
            "private_hosts",
            "proxy",
            "root_path",
            "use_git_history",
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

    /// A session rooted at a throwaway directory: writes a minimal `hok.json`
    /// with `root_path`/`cache_path` pinned inside the temp dir (so the test
    /// never reads the real config or the global cache path) and loads it.
    fn test_session(name: &str) -> Session {
        let root = std::env::temp_dir().join(format!("hok_cmd_config_{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let config_path = root.join("hok.json");
        let root_escaped = root.to_string_lossy().replace('\\', "\\\\");
        let cache_escaped = root.join("cache").to_string_lossy().replace('\\', "\\\\");
        std::fs::write(
            &config_path,
            format!(
                r#"{{"root_path": "{}", "cache_path": "{}"}}"#,
                root_escaped, cache_escaped
            ),
        )
        .unwrap();
        Session::new_with(&config_path).unwrap()
    }

    /// `list_all_rows` covers every supported key with its current value and
    /// effective default; a placeholder stays distinguishable from a real
    /// value, and after `config set` the current value updates.
    #[test]
    fn list_all_rows_covers_supported_keys_with_current_and_default() {
        let session = test_session("list_all");

        let rows = list_all_rows(&session);
        // every key reachable through `hok config set` is listed
        for key in [
            "aria2-enabled",
            "aria2-split",
            "aria2-max-connection-per-server",
            "aria2-min-split-size",
            "jobs",
            "cache_path",
            "cat_style",
            "confirm",
            "default_architecture",
            "editor",
            "global_path",
            "gh_token",
            "ignore_failures",
            "ignore_running_processes",
            "language",
            "last_update",
            "no_color",
            "no_junction",
            "autostash_on_conflict",
            "private_hosts",
            "proxy",
            "root_path",
            "script_engine",
            "use_git_history",
            "use_isolated_path",
            "use_sqlite_cache",
            "virustotal_api_key",
        ] {
            assert!(
                rows.iter().any(|r| r.key == key),
                "missing key {key} in {:?}",
                rows.iter().map(|r| r.key.as_str()).collect::<Vec<_>>()
            );
        }

        // An absent setting is `Unset` — never the literal "none", which is a
        // real `proxy` value the CLI could not tell apart from it — and an
        // auto-detected one is `Auto`; real values stay verbatim.
        assert_eq!(value_of(&rows, "proxy"), ConfigCell::Unset);
        assert_eq!(value_of(&rows, "editor"), ConfigCell::Unset);
        assert_eq!(value_of(&rows, "cat_style"), ConfigCell::Unset);
        assert_eq!(value_of(&rows, "default_architecture"), ConfigCell::Auto);
        assert_eq!(
            value_of(&rows, "no_junction"),
            ConfigCell::Text("false".to_owned())
        );

        // after setting, the current value column shows the new value
        libscoop::config::set(&session, "no_junction", "true").unwrap();
        let rows = list_all_rows(&session);
        assert_eq!(
            value_of(&rows, "no_junction"),
            ConfigCell::Text("true".to_owned())
        );
    }

    /// A row's default column is a promise about the runtime: for every
    /// boolean key it must equal the getter's value on a fresh (all-unset)
    /// config. This is the guard for the drift S12 found — the runtime default
    /// was changed to `false` while the column kept the old `true`.
    #[test]
    fn boolean_default_column_matches_the_runtime_default() {
        let session = test_session("bool_defaults");
        let rows = list_all_rows(&session);
        let config = session.config();
        for (key, actual) in [
            ("aria2-enabled", config.aria2_enabled()),
            ("ignore_failures", config.ignore_failures()),
            (
                "ignore_running_processes",
                config.ignore_running_processes(),
            ),
            ("no_color", config.no_color()),
            ("confirm", config.confirm()),
            ("script_engine", config.script_engine()),
            ("no_junction", config.no_junction()),
            ("autostash_on_conflict", config.autostash_on_conflict()),
            ("use_sqlite_cache", config.use_sqlite_cache()),
        ] {
            assert_eq!(
                default_of(&rows, key),
                ConfigCell::Text(fmt_bool(actual).to_owned()),
                "{key}: the default column disagrees with the runtime default"
            );
        }
    }

    fn value_of(rows: &[ConfigRow], key: &str) -> ConfigCell {
        rows.iter()
            .find(|r| r.key == key)
            .unwrap_or_else(|| panic!("missing row {key}"))
            .value
            .clone()
    }

    fn default_of(rows: &[ConfigRow], key: &str) -> ConfigCell {
        rows.iter()
            .find(|r| r.key == key)
            .unwrap_or_else(|| panic!("missing row {key}"))
            .default
            .clone()
    }
}
