//! Install package(s) command.
//!
//! Thin CLI wrapper around [`operation::install`]. Parses package names and
//! sync options from the command line, checks for administrator privileges
//! (for global install), and dispatches to the install pipeline.
//!
//! # Note
//!
//! This file is intentionally thin — all complex lifecycle logic lives in
//! `libscoop::operation` and `libscoop::package::sync`.

use clap::{ArgAction, Parser};
use libscoop::internal::os::is_admin;
use libscoop::{operation, QueryOption, Session};

use crate::cmd::shared_args::{Cmd, SyncArgs};
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

    /// Ignore cache and force download
    #[arg(short = 'D', long, action = ArgAction::SetTrue)]
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

    /// Skip package integrity check
    #[arg(short = 's', long, action = ArgAction::SetTrue)]
    no_hash_check: bool,
}

impl Cmd for Args {
    type Args = Self;

    fn execute(args: Self::Args, session: &Session) -> Result<()> {
        session.set_global(args.global);
        if args.global && !is_admin() {
            anyhow::bail!("ERROR: you need admin rights to install global apps");
        }

        let queries = args.package.iter().map(|s| s.as_str()).collect::<Vec<_>>();

        // Prune already-installed packages (matching PS1's prune_installed behavior)
        let (to_install, already_installed) = operation::package_prune_installed(session, &queries)?;
        for name in &already_installed {
            output::warn(format!("'{name}' is already installed. Skipping."));
        }
        if to_install.is_empty() {
            return Ok(());
        }

        // Build SyncArgs from individual fields, then convert to options
        let sync = SyncArgs {
            global: args.global,
            assume_yes: args.assume_yes,
            ignore_failure: args.ignore_failure,
            offline: args.offline,
            no_hash_check: args.no_hash_check,
            independent: args.independent,
            no_replace: args.no_replace,
            escape_hold: args.escape_hold,
            no_upgrade: args.no_upgrade,
            ignore_cache: args.ignore_cache,
            download_only: args.download_only,
        };
        let options = sync.to_sync_options(session);

        let handle = crate::eventloop::run_event_loop_default(session);

        operation::package_sync(session, to_install.clone(), options)?;
        handle.join().unwrap();

        // Show suggestions from manifests of installed packages
        show_suggestions(session, &to_install)?;

        Ok(())
    }
}

/// Module-level execute for dispatch compatibility.
#[inline]
pub fn execute(args: Args, session: &Session) -> Result<()> {
    <Args as Cmd>::execute(args, session)
}

/// Query installed packages and display their `suggest` field entries.
fn show_suggestions(session: &Session, packages: &[&str]) -> Result<()> {
    let installed = match operation::package_query(
        session,
        packages.to_vec(),
        vec![QueryOption::Explicit],
        true,
    ) {
        Ok(pkgs) => pkgs,
        Err(_) => return Ok(()),
    };

    for pkg in &installed {
        let manifest = pkg.manifest();
        if let Some(suggest) = manifest.suggest() {
            let name = pkg.name();
            output::info(format!("Suggestions for '{name}':"));
            for (key, values) in suggest {
                let vals = values
                    .devectorize()
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", ");
                output::info(format!("  {key}: {vals}"));
            }
        }
    }


    Ok(())
}
