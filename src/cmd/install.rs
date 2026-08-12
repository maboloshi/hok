//! Install package(s) command.
//!
//! Thin CLI wrapper around [`package::sync::sync`]. Parses package names and
//! sync options from the command line, checks for administrator privileges
//! (for global install), and dispatches to the install pipeline.
//!
//! # Note
//!
//! This file is intentionally thin — all complex lifecycle logic lives in
//! `libscoop::operation` and `libscoop::package::sync`.

use clap::{ArgAction, Parser};
use libscoop::{package, Session};

use crate::cmd::shared_args::{ensure_global, SyncArgs, SyncFlags};
use crate::{output, Result};

/// Install package(s)
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// The package(s) to install
    #[arg(required = true, action = ArgAction::Append)]
    package: Vec<String>,

    /// Install globally (to $SCOOP_GLOBAL)
    #[arg(short = 'g', long, action = ArgAction::SetTrue)]
    global: bool,

    /// Download package(s) without performing installation
    #[arg(short = 'd', long, action = ArgAction::SetTrue)]
    download_only: bool,

    /// Ignore failures to ensure a complete transaction
    #[arg(short = 'f', long, action = ArgAction::SetTrue)]
    ignore_failure: bool,

    /// Leverage cache and suppress network access
    #[arg(short = 'o', long, action = ArgAction::SetTrue)]
    offline: bool,

    /// Assume yes to all prompts and run non-interactively
    #[arg(short = 'y', long, action = ArgAction::SetTrue)]
    assume_yes: bool,

    /// Ignore cache and force download (Scoop: -k, --no-cache)
    #[arg(short = 'D', long, short_alias = 'k', visible_alias = "no-cache", action = ArgAction::SetTrue)]
    ignore_cache: bool,

    /// Do not install dependencies (may break packages)
    #[arg(short = 'I', long, action = ArgAction::SetTrue)]
    independent: bool,

    /// Do not replace package(s)
    #[arg(short = 'R', long, action = ArgAction::SetTrue)]
    no_replace: bool,

    /// Escape hold to allow changes on held package(s)
    #[arg(short = 'S', long, action = ArgAction::SetTrue)]
    escape_hold: bool,

    /// Do not upgrade package(s)
    #[arg(short = 'U', long, action = ArgAction::SetTrue)]
    no_upgrade: bool,

    /// Skip package integrity check (Scoop: --skip-hash-check)
    #[arg(short = 's', long, visible_alias = "skip-hash-check", action = ArgAction::SetTrue)]
    no_hash_check: bool,

    /// Use the specified architecture (32bit/64bit/arm64), overriding the
    /// runtime-detected and configured default (Scoop's `-a/--arch`)
    #[arg(short = 'a', long = "arch")]
    arch: Option<String>,
}

impl SyncFlags for Args {
    fn sync_args(&self) -> SyncArgs {
        SyncArgs {
            global: self.global,
            assume_yes: self.assume_yes,
            ignore_failure: self.ignore_failure,
            offline: self.offline,
            no_hash_check: self.no_hash_check,
            independent: self.independent,
            no_replace: self.no_replace,
            escape_hold: self.escape_hold,
            no_upgrade: self.no_upgrade,
            ignore_cache: self.ignore_cache,
            download_only: self.download_only,
        }
    }
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    // `-a/--arch` overrides the effective architecture (Scoop's
    // `Format-ArchitectureString` + `Get-DefaultArchitecture` override).
    // Parsed after the session was created, so it beats the
    // `default_architecture` config.
    if let Some(arch) = args.arch.as_deref() {
        let arch = libscoop::internal::arch::Arch::parse(arch)?;
        libscoop::internal::arch::Arch::set_default_architecture(arch);
    }

    ensure_global(session, args.global, "install")?;

    let queries = args.package.iter().map(|s| s.as_str()).collect::<Vec<_>>();

    // Prune already-installed packages (matching PS1's prune_installed behavior)
    let (to_install, already_installed) = package::query::prune_installed(session, &queries)?;
    for name in &already_installed {
        output::warn(format!("'{name}' is already installed. Skipping."));
    }
    if to_install.is_empty() {
        return Ok(());
    }

    let options = args.to_sync_options(session);

    let handle = crate::eventloop::run_event_loop_default(session);

    package::sync::sync(session, to_install.clone(), options)?;
    handle.join().unwrap();

    // Show unsatisfied suggestions from manifests of installed packages
    let suggestions = package::query::suggest(session, &to_install)?;
    for entry in &suggestions {
        let joined = entry.candidates.join("' or '");
        output::info(format!(
            "'{}' suggests installing '{}'.",
            entry.package, joined
        ));
    }

    Ok(())
}
