//! Manage shims (add / remove / list / inspect).

use clap::{ArgAction, Parser, Subcommand};
use libscoop::{add_custom, package, remove_by_name, Session};

use crate::{output, Result};

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

pub fn execute(args: Args, session: &Session) -> Result<()> {
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
            add_custom(session, name, target, args)?;
            output::done(rust_i18n::t!("cmd.shim_added", name = name));
        }
        Command::Remove { names } => {
            let mut failed = Vec::new();
            for name in &names {
                if remove_by_name(session, name)? {
                    output::done(rust_i18n::t!("cmd.shim_removed_by_name", name = name));
                } else {
                    failed.push(name.clone());
                }
            }
            if !failed.is_empty() {
                for name in &failed {
                    output::err(rust_i18n::t!("cmd.shim_not_found", name = name));
                }
                return Err(anyhow::anyhow!("shim(s) not found: {}", failed.join(", ")));
            }
        }
        Command::List { patterns } => {
            let shims = list_all_shims(session)?;
            if shims.is_empty() {
                output::warn(rust_i18n::t!("cmd.shim_no_dir"));
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
                        return Err(anyhow::anyhow!("invalid pattern '{}': {}", combined, e));
                    }
                }
            };
            for name in shims
                .iter()
                .filter(|n| regex.as_ref().is_none_or(|re| re.is_match(n)))
            {
                output::named(name.as_str(), "(shim)");
            }
        }
        Command::Info { name } => {
            let paths = package::shim::shim_paths(session, &name)?;
            if paths.is_empty() {
                output::err(format!("shim not found: {}", name));
                return Err(anyhow::anyhow!("shim not found: {}", name));
            }
            for (ext, path) in &paths {
                let ty = match ext.as_str() {
                    ".exe" => "Application",
                    ".ps1" => "ExternalScript",
                    _ => "Application",
                };
                output::change(format!("{}{}", name, ext), ty, path.display().to_string());
            }
        }
    }

    Ok(())
}

/// List shim names from both local and global shims directories (matching
/// Scoop's `list`, which merges both scopes).
fn list_all_shims(session: &Session) -> Result<Vec<String>> {
    let original_global = session.is_global();
    let mut names = Vec::new();

    for global in [false, true] {
        session.set_global(global);
        names.extend(package::shim::list_shims(session)?);
    }
    session.set_global(original_global);

    names.sort();
    names.dedup();
    Ok(names)
}
