//! Manage shims (add / remove / list / inspect).

use clap::{ArgAction, Parser, Subcommand};
use libscoop::{
    Session, ShimInfo, add_custom, list_shim_info, remove_by_name, shim_file, shim_info,
};

use crate::format::{pad_visual, visual_width};
use crate::output::style;
use crate::{Result, output};

/// Manipulate shims
#[derive(Debug, Parser)]
pub struct Args {
    /// Manipulate global shim(s)
    #[arg(short = 'g', long)]
    global: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Add a custom shim
    ///
    /// `hok shim add <name> <target> [args...]` — arguments starting with
    /// `-` must follow a `--` (POSIX-style option terminator).
    Add {
        /// `<name> <target> [args...]` — name, then the target command path
        /// (or a command name resolved through `PATH`), then any args
        #[arg(required = true, num_args = 2.., allow_hyphen_values = true)]
        parts: Vec<String>,
    },
    /// Remove shims (CAUTION: may remove shims created by an app manifest)
    #[clap(alias = "rm")]
    Remove {
        /// Shim name(s)
        #[arg(required = true, action = ArgAction::Append)]
        names: Vec<String>,
    },
    /// List all shims (default), optionally filtered by regex pattern(s)
    #[clap(alias = "ls")]
    List {
        /// Regex pattern(s) to filter by
        #[arg(action = ArgAction::Append)]
        patterns: Vec<String>,
    },
    /// Show shim information for a specific app
    Info {
        /// Shim name
        name: String,
    },
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    // `-g/--global` switches the session so `shims_dir()` resolves to the
    // global shims directory for the operation.
    if args.global {
        session.set_global(true);
    }

    match args.command.unwrap_or(Command::List { patterns: vec![] }) {
        Command::Add { parts } => {
            let name = &parts[0];
            let target = &parts[1];
            // A leading `--` (POSIX terminator) is not part of the args —
            // clap may surface it as a value under allow_hyphen_values.
            let args = parts[2..]
                .strip_prefix(["--".to_string()].as_slice())
                .unwrap_or(&parts[2..]);
            match add_custom(session, name, target, args) {
                Ok(()) => {}
                // Scoop reports a missing `<command_path>` with its own exit
                // code 3 (`libexec/scoop-shim.ps1:114-123`) instead of a
                // generic failure.
                Err(libscoop::Error::ShimTargetMissing(path)) => {
                    output::err(hok_i18n::t!("error.shim_target_missing", path = path));
                    return Err(crate::ExitCode(3).into());
                }
                Err(err) => return Err(err.into()),
            }
            output::done(hok_i18n::t!("cmd.shim_added", name = name));
        }
        Command::Remove { names } => {
            let mut failed = Vec::new();
            for name in &names {
                if remove_by_name(session, name)? {
                    output::done(hok_i18n::t!("cmd.shim_removed_by_name", name = name));
                } else {
                    failed.push(name.clone());
                }
            }
            if !failed.is_empty() {
                for name in &failed {
                    output::err(hok_i18n::t!("cmd.shim_not_found", name = name));
                }
                // Each missing shim was already reported above (localized);
                // just exit nonzero — do not also print a hardcoded-English
                // summary via the top-level error path. Scoop reserves 3 for
                // "shim not found" (`libexec/scoop-shim.ps1:139`), which is
                // why 1 would be indistinguishable from a real failure.
                return Err(crate::ExitCode(3).into());
            }
        }
        Command::List { patterns } => {
            let infos = list_shim_info(session)?;
            if infos.is_empty() {
                output::warn(hok_i18n::t!("cmd.shim_no_dir"));
                return Ok(());
            }
            // Compile the combined regex (Scoop joins patterns with `|`).
            let combined = patterns.join("|");
            let regex = if combined.is_empty() {
                None
            } else {
                match regex::Regex::new(&combined) {
                    Ok(re) => Some(re),
                    Err(e) => {
                        return Err(anyhow::anyhow!(hok_i18n::t!(
                            "cmd.shim_invalid_pattern",
                            pattern = combined,
                            e = e
                        )));
                    }
                }
            };
            let infos = infos
                .into_iter()
                .filter(|i| regex.as_ref().is_none_or(|re| re.is_match(&i.name)))
                .collect::<Vec<_>>();
            // A pattern that matches nothing renders nothing (upstream's
            // `Format-Table` over an empty pipeline prints no table either).
            if !infos.is_empty() {
                render(&infos);
            }
        }
        Command::Info { name } => {
            // Upstream `Get-ShimPath` describes one shim: the `.shim` metadata
            // file, else the `.ps1` script shim (`libexec/scoop-shim.ps1:83-90`);
            // hok additionally accepts the other variants it can generate.
            let Some(path) = shim_file(session, &name) else {
                output::err(hok_i18n::t!("cmd.shim_not_found", name = name));
                // Scoop splits the two not-found outcomes: a shim that exists
                // in the *other* scope exits 2 with a hint on how to ask for
                // it, a shim that exists nowhere exits 3
                // (`libexec/scoop-shim.ps1:169-183`).
                let wanted_global = args.global;
                session.set_global(!wanted_global);
                let other = shim_file(session, &name);
                session.set_global(wanted_global);
                if other.is_none() {
                    return Err(crate::ExitCode(3).into());
                }
                let scope = if wanted_global {
                    hok_i18n::t!("cmd.shim_scope_local")
                } else {
                    hok_i18n::t!("cmd.shim_scope_global")
                };
                let hint = if wanted_global {
                    format!("hok shim info {name}")
                } else {
                    format!("hok shim info {name} --global")
                };
                output::info(hok_i18n::t!(
                    "cmd.shim_other_scope",
                    scope = scope,
                    cmd = hint
                ));
                return Err(crate::ExitCode(2).into());
            };
            render(&[shim_info(session, &path)]);
        }
    }

    Ok(())
}

/// One shim's five rendered cells, in upstream's column order
/// (`supporting/formats/ScoopTypes.Format.ps1xml:34-62`).
fn columns(info: &ShimInfo, yes: &str, no: &str) -> [String; 5] {
    [
        info.name.clone(),
        info.source.clone(),
        info.alternatives.clone(),
        if info.is_global { yes } else { no }.to_owned(),
        if info.is_hidden { yes } else { no }.to_owned(),
    ]
}

/// Lay out upstream's Name | Source | Alternatives | IsGlobal | IsHidden
/// table: an emphasized header, a dim dashed rule, then one line per shim.
///
/// Every column but the last is padded to its width; `IsHidden` is never
/// padded, so no rendered line carries trailing spaces.
fn layout(rows: &[[String; 5]], headers: [&str; 5]) -> Vec<String> {
    let mut widths = headers.map(visual_width);
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(visual_width(cell));
        }
    }

    let mut lines = vec![
        format!(
            "  {}  {}  {}  {}  {}",
            style::accent_bold(pad_visual(headers[0], widths[0])),
            style::accent_bold(pad_visual(headers[1], widths[1])),
            style::accent_bold(pad_visual(headers[2], widths[2])),
            style::accent_bold(pad_visual(headers[3], widths[3])),
            style::accent_bold(headers[4]),
        ),
        format!(
            "  {}  {}  {}  {}  {}",
            style::dim("-".repeat(widths[0])),
            style::dim("-".repeat(widths[1])),
            style::dim("-".repeat(widths[2])),
            style::dim("-".repeat(widths[3])),
            style::dim("-".repeat(widths[4])),
        ),
    ];

    for row in rows {
        lines.push(format!(
            "  {}  {}  {}  {}  {}",
            style::emphasis(pad_visual(&row[0], widths[0])),
            pad_visual(&row[1], widths[1]),
            pad_visual(&row[2], widths[2]),
            pad_visual(&row[3], widths[3]),
            row[4],
        ));
    }

    lines
}

/// Print the shim table with the localized headers.
fn render(infos: &[ShimInfo]) {
    let yes = hok_i18n::t!("cmd.shim_true");
    let no = hok_i18n::t!("cmd.shim_false");
    let name = hok_i18n::t!("cmd.shim_header_name");
    let source = hok_i18n::t!("cmd.shim_header_source");
    let alternatives = hok_i18n::t!("cmd.shim_header_alternatives");
    let is_global = hok_i18n::t!("cmd.shim_header_is_global");
    let is_hidden = hok_i18n::t!("cmd.shim_header_is_hidden");

    let rows = infos
        .iter()
        .map(|i| columns(i, &yes, &no))
        .collect::<Vec<_>>();

    for line in layout(
        &rows,
        [
            name.as_str(),
            source.as_str(),
            alternatives.as_str(),
            is_global.as_str(),
            is_hidden.as_str(),
        ],
    ) {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(
        name: &str,
        source: &str,
        alternatives: &str,
        is_global: bool,
        is_hidden: bool,
    ) -> ShimInfo {
        ShimInfo {
            name: name.to_owned(),
            path: std::path::PathBuf::from(format!("{name}.exe")),
            source: source.to_owned(),
            alternatives: alternatives.to_owned(),
            is_global,
            is_hidden,
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

    /// The five upstream columns line up (CJK names included), the last column
    /// is not padded, and no line carries trailing spaces.
    #[test]
    fn layout_aligns_the_five_columns() {
        let rows = vec![
            columns(
                &info("git", "gitapp", "gitapp extra", true, false),
                "True",
                "False",
            ),
            columns(
                &info("中文命令", "external", "", false, true),
                "True",
                "False",
            ),
        ];
        let lines = layout(
            &rows,
            ["Name", "Source", "Alternatives", "IsGlobal", "IsHidden"],
        );

        assert_eq!(lines.len(), 4, "header, rule and two rows");
        let plain_lines = lines.iter().map(|l| plain(l)).collect::<Vec<_>>();

        let source_col = 2 + visual_width("中文命令") + 2;
        let alternatives_col = source_col + visual_width("external") + 2;
        let global_col = alternatives_col + visual_width("Alternatives") + 2;
        let hidden_col = global_col + visual_width("IsGlobal") + 2;

        assert_eq!(value_column(&plain_lines[2], "gitapp"), source_col);
        assert_eq!(
            value_column(&plain_lines[2], "gitapp extra"),
            alternatives_col
        );
        assert_eq!(value_column(&plain_lines[2], "True"), global_col);
        assert_eq!(value_column(&plain_lines[2], "False"), hidden_col);
        assert_eq!(value_column(&plain_lines[3], "external"), source_col);
        assert_eq!(value_column(&plain_lines[3], "False"), global_col);
        assert_eq!(value_column(&plain_lines[3], "True"), hidden_col);

        assert!(
            plain_lines.iter().all(|l| !l.ends_with(' ')),
            "trailing spaces: {plain_lines:#?}"
        );
    }
}
