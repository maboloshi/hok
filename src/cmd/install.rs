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
use libscoop::{Session, package};

use crate::cmd::shared_args::{SyncArgs, SyncFlags, ensure_global};
use crate::{Result, output};

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

    /// Ignore failures to ensure a complete transaction (persistent setting:
    /// 'hok config ignore_failures true')
    #[arg(long, action = ArgAction::SetTrue)]
    ignore_failures: bool,

    /// Leverage cache and suppress network access
    #[arg(short = 'o', long, action = ArgAction::SetTrue)]
    offline: bool,

    /// Assume yes to all prompts and run non-interactively
    #[arg(short = 'y', long, action = ArgAction::SetTrue)]
    assume_yes: bool,

    /// Don't use the download cache
    #[arg(short = 'k', long, action = ArgAction::SetTrue)]
    no_cache: bool,

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
    skip_hash_check: bool,

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
            ignore_failures: self.ignore_failures,
            offline: self.offline,
            skip_hash_check: self.skip_hash_check,
            independent: self.independent,
            no_replace: self.no_replace,
            escape_hold: self.escape_hold,
            no_upgrade: self.no_upgrade,
            no_cache: self.no_cache,
            download_only: self.download_only,
        }
    }
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    // `-a/--arch` overrides the effective architecture (Scoop's
    // `Format-ArchitectureString` + `Get-DefaultArchitecture` override).
    // Parsed after the session was created, so it beats the
    // `default_architecture` config.
    if let Some(arch) = args.arch.as_deref() {
        session.set_default_architecture(arch)?;
    }

    ensure_global(session, args.global, "install")?;

    let queries = args.package.iter().map(|s| s.as_str()).collect::<Vec<_>>();

    // Prune already-installed packages (matching PS1's prune_installed behavior)
    let (to_install, already_installed) = package::query::prune_installed(session, &queries)?;
    for name in &already_installed {
        output::warn(hok_i18n::t!("cmd.already_installed_skip", name = name));
    }
    if to_install.is_empty() {
        return Ok(());
    }

    let options = args.to_sync_options(session);

    let handle = crate::eventloop::run_event_loop_default(session);

    let sync_result = package::sync::sync(session, to_install.clone(), options).await;
    handle.await.unwrap();
    // A declined confirmation is not a failure: exit 0 without the
    // "Everything is ok!" message.
    if matches!(sync_result, Err(libscoop::Error::UserAborted)) {
        return Ok(());
    }
    sync_result?;

    // Show unsatisfied suggestions from manifests of installed packages
    let suggestions = package::query::suggest(session, &to_install)?;
    for entry in &suggestions {
        let joined = entry.candidates.join("' or '");
        output::info(hok_i18n::t!(
            "cmd.suggests_install",
            pkg = entry.package,
            names = joined
        ));
    }

    if args.download_only {
        output::done(hok_i18n::t!("output.downloaded_only"));
    } else {
        output::done(hok_i18n::t!("output.ok_all"));
    }

    Ok(())
}
