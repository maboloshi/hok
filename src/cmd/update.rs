//! Fetch and update subscribed buckets, or upgrade installed package(s).

use std::io::IsTerminal;

use clap::{ArgAction, Parser};
use libscoop::{Event, Session, SyncOption, bucket, package};

use crate::cmd::shared_args::{SyncArgs, SyncFlags, ensure_global};
use crate::{Result, cui, output};

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

    /// Skip package integrity check (Scoop: --skip-hash-check)
    #[arg(short = 's', long, visible_alias = "skip-hash-check", action = ArgAction::SetTrue)]
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

    /// Ignore cache and force download (Scoop: -k, --no-cache)
    #[arg(short = 'D', long, short_alias = 'k', visible_alias = "no-cache", action = ArgAction::SetTrue)]
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

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    // --all is a shorthand for upgrading all packages (like passing '*')
    if args.all && args.package.is_empty() {
        return execute_upgrade(session, &[String::from("*")], &args.sync_args(), args.force).await;
    }

    if !args.package.is_empty() {
        return execute_upgrade(session, &args.package, &args.sync_args(), args.force).await;
    }

    // Bucket update mode (no packages specified).
    // Mirrors scoop-update.ps1: `--global` is invalid without an explicit
    // <app> — reject it instead of silently ignoring the flag.
    if args.global {
        return Err(anyhow::anyhow!(hok_i18n::t!("cmd.update_global_no_app")));
    }
    update_buckets(session, args.force).await
}

/// Update all buckets with simple inline status.
async fn update_buckets(session: &Session, force: bool) -> Result<()> {
    // Cooldown: skip if buckets were updated less than 15 minutes ago (unless --force)
    if !force && let Some(remaining) = session.config().update_cooldown_remaining() {
        output::status(hok_i18n::t!("cmd.bucket_cooldown", remaining = remaining));
        return Ok(());
    }

    let mut rx = session
        .event_bus()
        .receiver()
        .expect("event bus outbound receiver already taken");

    let handle = tokio::spawn(async move {
        // Buckets pull concurrently (bucket::update spawns one task per
        // bucket), so each bucket gets its own spinner row that finishes in
        // place — an inline status/done pair would interleave the
        // "Updating..."/"updated." lines when buckets finish out of order.
        let mut ui = cui::RowUI::new();
        while let Some(event) = rx.recv().await {
            match event {
                Event::BucketUpdateProgress(ctx) => {
                    let name = ctx.name();
                    if ctx.state().started() {
                        ui.start(name);
                    } else if ctx.state().succeeded() {
                        ui.done(name);
                    } else if let Some(err) = ctx.state().failed() {
                        ui.fail(name, err);
                    }
                }
                Event::BucketUpdateDone => break,
                _ => {}
            }
        }
        ui.finish_all();
    });

    output::header(hok_i18n::t!("cmd.header_buckets"));
    let changes = bucket::update(session).await?;
    handle.await.unwrap();

    // Refresh SQLite manifest cache with visible feedback. `bucket::update`
    // returns the manifest diff, so the cache is updated incrementally (only
    // the changed manifests are re-parsed) instead of rebuilt in full — and
    // nothing is printed at all when the cache is disabled in config.
    refresh_manifest_cache(session, &changes)?;

    Ok(())
}

/// Refresh the SQLite manifest cache with a single-step spinner.
///
/// On a terminal the refresh spins (`cui::RowUI`, indicatif) and finishes in
/// place: `✓ <cache_done>`, `✓ <cache_rebuilt>`, or `✓ <cache_rescanned>`, which
/// names the buckets that had to be re-scanned. On a non-terminal (piped) it
/// falls back to the plain `status`/`done` line pair, since indicatif's
/// `MultiProgress` is hidden and renders nothing when the output is not a TTY.
/// Nothing at all is printed when the SQLite cache is disabled.
///
/// A failed refresh is an error, not a silent one: the cache drops its sync
/// markers (so a retry rebuilds it from scratch) and the command fails with a
/// retry hint.
fn refresh_manifest_cache(session: &Session, changes: &bucket::UpdateChanges) -> Result<()> {
    if !session.config().use_sqlite_cache() {
        return Ok(());
    }

    // The spinner starts *before* the work: a full rebuild can run for tens of
    // seconds, and this row is the only feedback while it does.
    if std::io::stdout().is_terminal() {
        let mut ui = cui::RowUI::new();
        ui.set_spin("manifest-cache", hok_i18n::t!("cmd.refresh_cache"));
        match package::manifest_cache::refresh(session, changes) {
            Ok(outcome) => {
                ui.done_with("manifest-cache", cache_refresh_message(&outcome));
                Ok(())
            }
            Err(e) => {
                // The row prints the message; the exit code still reports the
                // failure.
                ui.fail(
                    "manifest-cache",
                    &hok_i18n::t!("cmd.cache_refresh_failed", e = e),
                );
                Err(crate::ExitCode(1).into())
            }
        }
    } else {
        output::status(hok_i18n::t!("cmd.refresh_cache"));
        match package::manifest_cache::refresh(session, changes) {
            Ok(outcome) => {
                output::done(cache_refresh_message(&outcome));
                Ok(())
            }
            Err(e) => {
                output::err(hok_i18n::t!("cmd.cache_refresh_failed", e = e));
                Err(crate::ExitCode(1).into())
            }
        }
    }
}

/// The completion line for a refresh outcome.
fn cache_refresh_message(outcome: &package::manifest_cache::RefreshOutcome) -> String {
    use package::manifest_cache::RefreshOutcome;

    match outcome {
        RefreshOutcome::Rebuilt => hok_i18n::t!("cmd.cache_rebuilt"),
        RefreshOutcome::Rescanned(buckets) if !buckets.is_empty() => {
            hok_i18n::t!("cmd.cache_rescanned", buckets = buckets.join(", "))
        }
        // Incremental, or a `Disabled` outcome the caller already filtered out.
        _ => hok_i18n::t!("cmd.cache_done"),
    }
}

/// Shared upgrade logic — used by both `update` (when packages given) and `upgrade`.
pub async fn execute_upgrade(
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

    let mut options = sync.to_sync_options(session);
    // Always restrict the update to installed packages (OnlyUpgrade keeps
    // the existence check and the installed-only query). --force adds
    // SyncOption::Force so packages at their current version are
    // reinstalled too, instead of falling into the plain install path
    // (which would expand `*` to every bucket package).
    options.push(SyncOption::OnlyUpgrade);
    if !force {
        // "All apps are up to date" is upgrade semantics: the update command
        // decides it, not the event loop. Nothing upgradable and at least
        // one of the targets installed means there is nothing to do.
        // A named target that isn't installed must NOT be reported as
        // up-to-date — fall through to sync so it reports PackageNotFound.
        let mut missing_target = false;
        for &q in &queries {
            if q != "*" && package::query::query_installed(session, &[q], &[])?.is_empty() {
                missing_target = true;
                break;
            }
        }
        let upgradable = package::query::query_installed(
            session,
            &queries,
            &[libscoop::QueryOption::Upgradable],
        )?;
        let installed = package::query::query_installed(session, &queries, &[])?;
        if !missing_target && upgradable.is_empty() && !installed.is_empty() {
            output::info(hok_i18n::t!("cmd.outdated"));
            return Ok(());
        }
    } else {
        options.push(SyncOption::Force);
    }

    let handle = crate::eventloop::run_event_loop_default(session);

    let sync_result = package::sync::sync(session, queries, options).await;
    handle.await.unwrap();
    // A declined confirmation is not a failure: exit 0 without ok_all.
    if matches!(sync_result, Err(libscoop::Error::UserAborted)) {
        return Ok(());
    }
    sync_result?;

    output::done(hok_i18n::t!("output.ok_all"));

    Ok(())
}
