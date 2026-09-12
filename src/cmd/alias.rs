//! Manage Scoop aliases.
//!
//! Mirrors upstream `libexec/scoop-alias.ps1` + `lib/commands.ps1`: `add`
//! writes a `<shims>\scoop-<name>.ps1` script and indexes it in the config,
//! `rm` removes both, and `list` renders Name/Command (plus a Summary column
//! under `-v`). Aliases are dispatched the way upstream does it — by the
//! presence of the script file — see [`dispatch`].

use std::ffi::OsString;

use clap::{ArgAction, Parser, Subcommand};
use libscoop::{Session, alias};

use crate::Result;
use crate::format::{pad_visual, visual_width};
use crate::output::{self, style};

/// List, add, or remove Scoop aliases
///
/// Examples:
///   hok alias                              list all aliases
///   hok alias list                         list all aliases
///   hok alias add <name> <command>         add an alias
///   hok alias add <name> <command> <desc>  add an alias with a description
///   hok alias rm  <name>                   remove an alias
#[derive(Debug, Parser)]
pub struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List all aliases (default)
    #[clap(alias = "ls")]
    List {
        /// Show the alias description column
        // A count, not a flag: `-v` is the CLI's global verbosity option, so
        // `hok alias list -v` and `hok -v alias list` share it (as `info` and
        // `ci-auto-pr` already do).
        #[arg(short = 'v', long, action = ArgAction::Count)]
        verbose: u8,
    },

    /// Add a new alias
    Add {
        /// Alias name
        name: String,
        /// Command to execute
        value: String,
        /// Alias description (written as the script's `# Summary:` line)
        description: Option<String>,
    },

    /// Remove an alias
    #[clap(alias = "rm", alias = "delete")]
    Remove {
        /// Alias name to remove
        name: String,
    },
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    match args.command {
        Some(Command::Add {
            name,
            value,
            description,
        }) => add(session, &name, &value, description.as_deref()),
        Some(Command::Remove { name }) => remove(session, &name),
        Some(Command::List { verbose }) => list(session, verbose > 0),
        // No subcommand lists the aliases (Scoop requires one and exits 1;
        // hok keeps the convenience form — see the S2 decision log).
        None => list(session, false),
    }
}

/// Run `name` as an alias when `shims\scoop-<name>.ps1` exists.
///
/// Returns `None` when the token is not an alias, so the caller falls back to
/// clap's own unknown-subcommand diagnostic (message and exit code unchanged).
pub(crate) async fn dispatch(
    session: &Session,
    name: &str,
    args: &[OsString],
) -> Result<Option<i32>> {
    if alias::resolve(session, name).is_none() {
        return Ok(None);
    }
    Ok(Some(alias::run(session, name, args).await?))
}

/// Add an alias, rejecting names that clap would resolve before the alias is
/// ever consulted.
fn add(session: &Session, name: &str, value: &str, description: Option<&str>) -> Result<()> {
    // `hok list`, `hok s`, `hok rm` … are clap subcommands, and clap wins over
    // any alias — writing one would leave an entry that can never be invoked.
    // Upstream shadows it silently; hok fails loudly instead.
    if crate::cmd::cli_command_names()
        .iter()
        .any(|n| n.eq_ignore_ascii_case(name))
    {
        return Err(libscoop::Error::AliasReservedName(name.to_owned()).into());
    }

    alias::add(session, name, value, description)?;
    output::info(hok_i18n::t!("cmd.alias_added", name = name, value = value));
    Ok(())
}

/// Remove an alias: its script file and its config entry.
fn remove(session: &Session, name: &str) -> Result<()> {
    alias::remove(session, name)?;
    output::info(hok_i18n::t!("cmd.alias_removed", name = name));
    Ok(())
}

/// Print the alias table.
fn list(session: &Session, verbose: bool) -> Result<()> {
    let aliases = alias::list(session)?;
    if aliases.is_empty() {
        output::warn(hok_i18n::t!("cmd.no_aliases"));
        return Ok(());
    }

    output::header(hok_i18n::t!("cmd.header_aliases"));
    for line in layout(&aliases, verbose) {
        println!("{line}");
    }
    Ok(())
}

/// Lay out the alias table: an emphasized header, a dim dashed rule, then one
/// line per alias.
///
/// `Command` and `Summary` are last-on-the-line columns (Summary only under
/// `-v`), so no rendered line carries trailing spaces — the same rule the
/// `bucket`/`list` tables follow.
fn layout(aliases: &[alias::Alias], verbose: bool) -> Vec<String> {
    let hdr_name = hok_i18n::t!("cmd.alias_header_name");
    let hdr_command = hok_i18n::t!("cmd.alias_header_command");
    let hdr_summary = hok_i18n::t!("cmd.alias_header_summary");

    let w_name = visual_width(&hdr_name).max(
        aliases
            .iter()
            .map(|a| visual_width(&a.name))
            .max()
            .unwrap_or(0),
    );
    // The last rendered column is never padded: Command when Summary is
    // hidden, Summary when it is shown.
    let w_command = if verbose {
        visual_width(&hdr_command).max(
            aliases
                .iter()
                .map(|a| visual_width(&a.command))
                .max()
                .unwrap_or(0),
        )
    } else {
        visual_width(&hdr_command)
    };

    let name = style::accent_bold(pad_visual(&hdr_name, w_name));
    let command = style::accent_bold(if verbose {
        pad_visual(&hdr_command, w_command)
    } else {
        hdr_command.to_string()
    });
    let mut lines = vec![
        if verbose {
            format!("  {name}  {command}  {}", style::accent_bold(&hdr_summary))
        } else {
            format!("  {name}  {command}")
        },
        if verbose {
            format!(
                "  {}  {}  {}",
                style::dim("-".repeat(w_name)),
                style::dim("-".repeat(w_command)),
                style::dim("-".repeat(visual_width(&hdr_summary))),
            )
        } else {
            format!(
                "  {}  {}",
                style::dim("-".repeat(w_name)),
                style::dim("-".repeat(visual_width(&hdr_command))),
            )
        },
    ];

    for a in aliases {
        let name = style::emphasis(pad_visual(&a.name, w_name));
        lines.push(if verbose {
            // An alias without a description (and every `<BROKEN>` one) leaves
            // the Summary column empty; trimming keeps the "no trailing
            // spaces" rule the other hok tables follow.
            format!(
                "  {name}  {}  {}",
                pad_visual(&a.command, w_command),
                a.summary
            )
            .trim_end()
            .to_string()
        } else {
            format!("  {name}  {}", a.command)
        });
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alias(name: &str, command: &str, summary: &str) -> alias::Alias {
        alias::Alias {
            name: name.to_owned(),
            command: command.to_owned(),
            summary: summary.to_owned(),
            broken: false,
        }
    }

    /// Strip ANSI styling so the table can be asserted as plain text.
    fn plain(lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .map(|l| {
                let mut out = String::new();
                let mut in_escape = false;
                for c in l.chars() {
                    if in_escape {
                        if c == 'm' {
                            in_escape = false;
                        }
                    } else if c == '\u{1b}' {
                        in_escape = true;
                    } else {
                        out.push(c);
                    }
                }
                out
            })
            .collect()
    }

    #[test]
    fn table_columns_line_up_and_carry_no_trailing_spaces() {
        crate::output::set_color_enabled(false);
        let rows = [alias("rm", "scoop uninstall $args[0]", "Uninstall")];
        let lines = plain(&layout(&rows, false));
        // Name column widens to the header, Command is the last column.
        assert_eq!(lines[0], "  Name  Command");
        assert_eq!(lines[1], "  ----  -------");
        assert_eq!(lines[2], "  rm    scoop uninstall $args[0]");
        for line in &lines {
            assert_eq!(line.trim_end(), line, "no trailing spaces: {line:?}");
        }
    }

    #[test]
    fn verbose_appends_the_summary_column() {
        crate::output::set_color_enabled(false);
        let rows = [
            alias("rm", "scoop uninstall", "Uninstall"),
            // No description: the Summary cell is empty and must not leave
            // trailing spaces behind.
            alias("zz", "scoop status", ""),
        ];
        let lines = plain(&layout(&rows, true));
        assert_eq!(lines[0], "  Name  Command          Summary");
        assert_eq!(lines[1], "  ----  ---------------  -------");
        assert_eq!(lines[2], "  rm    scoop uninstall  Uninstall");
        assert_eq!(lines[3], "  zz    scoop status");
        for line in &lines {
            assert_eq!(line.trim_end(), line, "no trailing spaces: {line:?}");
        }
    }

    #[test]
    fn a_broken_alias_renders_its_marker() {
        crate::output::set_color_enabled(false);
        let mut row = alias("gone", "scoop list", "");
        row.command = "<BROKEN>".to_owned();
        row.broken = true;
        let lines = plain(&layout(&[row], false));
        assert_eq!(lines[2], "  gone  <BROKEN>");
    }
}
