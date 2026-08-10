//! Fetch and update subscribed buckets, or upgrade installed package(s).

use clap::{ArgAction, Parser};
use libscoop::{bucket, package, Event, Session, SyncOption};

use crate::cmd::shared_args::{ensure_global, SyncArgs, SyncFlags};
use crate::{output, Result};

/// Fetch and update subscribed buckets, or upgrade installed package(s)
///
/// Examples:
///   hok update         update buckets only
///   hok update <app>   upgrade a specific package (Scoop-compatible)
///   hok update *       upgrade all packages
#[derive(Debug, Parser)]
pub struct Args {
    /// The package(s) to be upgraded (omit to only update buckets)
    #[arg(action = ArgAction::Append)]
    package: Vec<String>,

    /// Ignore failures to ensure a complete transaction
    #[arg(long, action = ArgAction::SetTrue)]
    ignore_failure: bool,

    /// Leverage cache and suppress network access
    #[arg(short = 'o', long, action = ArgAction::SetTrue)]
    offline: bool,

    /// Assume yes to all prompts and run non-interactively
    #[arg(short = 'y', long, action = ArgAction::SetTrue)]
    assume_yes: bool,

    /// Escape hold to allow to upgrade held package(s)
    #[arg(short = 'S', long, action = ArgAction::SetTrue)]
    escape_hold: bool,

    /// Skip package integrity check
    #[arg(short = 's', long, action = ArgAction::SetTrue)]
    no_hash_check: bool,

    /// Force update: bypass the cooldown and reinstall apps that are already
    /// at the requested version (Scoop's `-f/--force`)
    #[arg(short = 'f', long, action = ArgAction::SetTrue)]
    force: bool,

    /// Do not install dependencies (may break packages)
    #[arg(short = 'I', long, action = ArgAction::SetTrue)]
    independent: bool,

    /// Do not upgrade package(s)
    #[arg(short = 'U', long, action = ArgAction::SetTrue)]
    no_upgrade: bool,

    /// Do not replace package(s)
    #[arg(short = 'R', long, action = ArgAction::SetTrue)]
    no_replace: bool,

    /// Ignore cache and force download
    #[arg(short = 'D', long, action = ArgAction::SetTrue)]
    ignore_cache: bool,

    /// Install globally (to $SCOOP_GLOBAL)
    #[arg(short = 'g', long, action = ArgAction::SetTrue)]
    global: bool,

    /// Update all installed packages (alternative to '*')
    #[arg(short = 'a', long, action = ArgAction::SetTrue)]
    all: bool,
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
            // `update` never downloads without installing (unlike `install`)
            download_only: false,
        }
    }
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    // --all is a shorthand for upgrading all packages (like passing '*')
    if args.all && args.package.is_empty() {
        return execute_upgrade(session, &[String::from("*")], &args.sync_args(), args.force);
    }

    if !args.package.is_empty() {
        return execute_upgrade(session, &args.package, &args.sync_args(), args.force);
    }

    // Bucket update mode (no packages specified).
    // Mirrors scoop-update.ps1: `--global` is invalid without an explicit
    // <app> — reject it instead of silently ignoring the flag.
    if args.global {
        return Err(anyhow::anyhow!(rust_i18n::t!("cmd.update_global_no_app")));
    }
    update_buckets(session, args.force)
}

use crate::cmd::shared_args::Cmd;

impl Cmd for Args {
    type Args = Self;

    #[inline]
    fn execute(args: Self::Args, session: &Session) -> Result<()> {
        execute(args, session)
    }
}

/// Update all buckets with simple inline status.
fn update_buckets(session: &Session, force: bool) -> Result<()> {
    // Cooldown: skip if buckets were updated less than 15 minutes ago (unless --force)
    if !force {
        if let Some(remaining) = session.config().update_cooldown_remaining() {
            output::status(format!(
                "Buckets recently updated. Next update allowed in ~{remaining}s. Use --force to update now."
            ));
            return Ok(());
        }
    }

    let rx = session.event_bus().receiver();

    let handle = std::thread::spawn(move || {
        while let Ok(event) = rx.recv() {
            match event {
                Event::BucketUpdateProgress(ctx) => {
                    if ctx.state().started() {
                        output::status(format!("Updating '{}'...", ctx.name()));
                    } else if ctx.state().succeeded() {
                        output::done(format!("'{}' updated.", ctx.name()));
                    } else if let Some(err) = ctx.state().failed() {
                        output::err(format!("'{}' failed: {}", ctx.name(), err));
                    }
                }
                Event::BucketUpdateDone => break,
                _ => {}
            }
        }
    });

    output::header(rust_i18n::t!("cmd.header_buckets"));
    bucket::update(session)?;
    handle.join().unwrap();

    // Refresh SQLite manifest cache with visible feedback
    if session.config().use_sqlite_cache() {
        output::status(rust_i18n::t!("cmd.refresh_cache"));
        package::manifest_cache::refresh(session);
        output::done(rust_i18n::t!("cmd.cache_done"));
    }

    Ok(())
}

/// Shared upgrade logic — used by both `update` (when packages given) and `upgrade`.
pub fn execute_upgrade(
    session: &Session,
    packages: &[String],
    sync: &SyncArgs,
    force: bool,
) -> Result<()> {
    let mut queries = packages.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    if queries.is_empty() {
        queries.push("*");
    }

    ensure_global(session, sync.global, "upgrade")?;

    // When --force is set, skip OnlyUpgrade so that packages already at the
    // latest version are still reinstalled (matching PS1's "force update").
    let mut options = sync.to_sync_options(session);
    if !force {
        options.push(SyncOption::OnlyUpgrade);
    }

    let handle = crate::eventloop::run_event_loop_default(session);

    package::sync::sync(session, queries, options)?;
    handle.join().unwrap();

    Ok(())
}
