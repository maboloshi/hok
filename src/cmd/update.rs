use clap::{ArgAction, Parser};
use libscoop::internal::os::is_admin;
use libscoop::{operation, Event, Session, SyncOption};

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
    pub package: Vec<String>,
    /// Ignore failures to ensure a complete transaction
    #[arg(short = 'f', long, action = ArgAction::SetTrue)]
    pub ignore_failure: bool,
    /// Leverage cache and suppress network access
    #[arg(short = 'o', long, action = ArgAction::SetTrue)]
    pub offline: bool,
    /// Assume yes to all prompts and run non-interactively
    #[arg(short = 'y', long, action = ArgAction::SetTrue)]
    pub assume_yes: bool,
    /// Escape hold to allow to upgrade held package(s)
    #[arg(short = 'S', long, action = ArgAction::SetTrue)]
    pub escape_hold: bool,
    /// Skip package integrity check
    #[arg(short = 's', long, action = ArgAction::SetTrue)]
    pub no_hash_check: bool,
    /// Force update even within cooldown period
    #[arg(long, action = ArgAction::SetTrue)]
    pub force: bool,
    /// Do not install dependencies (may break packages)
    #[arg(short = 'I', long, action = ArgAction::SetTrue)]
    pub independent: bool,
    /// Do not upgrade package(s)
    #[arg(short = 'U', long, action = ArgAction::SetTrue)]
    pub no_upgrade: bool,
    /// Do not replace package(s)
    #[arg(short = 'R', long, action = ArgAction::SetTrue)]
    pub no_replace: bool,
    /// Ignore cache and force download
    #[arg(short = 'D', long, action = ArgAction::SetTrue)]
    pub ignore_cache: bool,
    /// Install globally (to $SCOOP_GLOBAL)
    #[arg(short = 'g', long, action = ArgAction::SetTrue)]
    pub global: bool,
    /// Hide extraneous messages
    #[arg(short = 'q', long, action = ArgAction::SetTrue)]
    pub quiet: bool,
    /// Update all installed packages (alternative to '*')
    #[arg(short = 'a', long, action = ArgAction::SetTrue)]
    pub all: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    // --all is a shorthand for upgrading all packages (like passing '*')
    if args.all && args.package.is_empty() {
        return execute_upgrade(session, &[String::from("*")], &args);
    }

    if !args.package.is_empty() {
        return execute_upgrade(session, &args.package, &args);
    }

    // Bucket update mode (no packages specified)
    update_buckets(session, args.force)
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
    operation::bucket_update(session)?;
    handle.join().unwrap();

    // Refresh SQLite manifest cache with visible feedback
    if session.config().use_sqlite_cache() {
        output::status(rust_i18n::t!("cmd.refresh_cache"));
        operation::refresh_manifest_cache(session);
        output::done(rust_i18n::t!("cmd.cache_done"));
    }

    Ok(())
}

/// Shared upgrade logic — used by both `update` (when packages given) and `upgrade`.
pub fn execute_upgrade(session: &Session, packages: &[String], args: &Args) -> Result<()> {
    let mut queries = packages.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    if queries.is_empty() {
        queries.push("*");
    }

    session.set_global(args.global);
    if args.global && !is_admin() {
        anyhow::bail!("ERROR: you need admin rights to install global apps");
    }

    // When --force is set, skip OnlyUpgrade so that packages already at the
    // latest version are still reinstalled (matching PS1's "force update").
    let mut options = if args.force {
        vec![]
    } else {
        vec![SyncOption::OnlyUpgrade]
    };

    if args.assume_yes {
        options.push(SyncOption::AssumeYes);
    }

    if args.escape_hold {
        options.push(SyncOption::EscapeHold);
    }

    if args.ignore_failure || session.config().ignore_failures() {
        options.push(SyncOption::IgnoreFailure);
    }

    if args.offline {
        options.push(SyncOption::Offline);
    }

    if args.no_hash_check {
        options.push(SyncOption::NoHashCheck);
    }

    let handle = crate::eventloop::run_event_loop_default(session);

    operation::package_sync(session, queries, options)?;
    handle.join().unwrap();

    Ok(())
}
