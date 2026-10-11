//! Fetch and update subscribed buckets, or upgrade installed package(s).

use std::io::IsTerminal;

use clap::{ArgAction, Parser};
use libscoop::{Event, Session, bucket, package};

use crate::cmd::shared_args::{SyncArgs, SyncFlags};
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

    /// Skip package integrity check
    #[arg(short = 's', long, action = ArgAction::SetTrue)]
    skip_hash_check: bool,

    /// Force update even when there isn't a newer version (Scoop's `-f/--force`)
    #[arg(short = 'f', long, action = ArgAction::SetTrue)]
    force: bool,

    /// Don't install dependencies automatically
    #[arg(short = 'i', long, action = ArgAction::SetTrue)]
    independent: bool,

    /// Do not upgrade package(s)
    #[arg(short = 'U', long, action = ArgAction::SetTrue)]
    no_upgrade: bool,

    /// Do not replace package(s)
    #[arg(short = 'R', long, action = ArgAction::SetTrue)]
    no_replace: bool,

    /// Don't use the download cache
    #[arg(short = 'k', long, action = ArgAction::SetTrue)]
    no_cache: bool,

    /// Install globally (to $SCOOP_GLOBAL)
    #[arg(short = 'g', long, action = ArgAction::SetTrue)]
    global: bool,

    /// Update all installed packages (alternative to '*')
    #[arg(short = 'a', long, action = ArgAction::SetTrue)]
    all: bool,

    /// Remove the old versions of the package(s) this run upgrades; nothing is
    /// removed when no package was upgraded
    #[arg(short = 'c', long, action = ArgAction::SetTrue)]
    cleanup: bool,
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
            no_upgrade: self.no_upgrade,
            no_cache: self.no_cache,
            // `update` never downloads without installing (unlike `install`)
            download_only: false,
        }
    }
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    // --all is a shorthand for upgrading all packages (like passing '*')
    if args.all && args.package.is_empty() {
        return super::upgrade::execute_upgrade(
            session,
            &[String::from("*")],
            &args.sync_args(),
            args.force,
            args.cleanup,
        )
        .await;
    }

    if !args.package.is_empty() {
        return super::upgrade::execute_upgrade(
            session,
            &args.package,
            &args.sync_args(),
            args.force,
            args.cleanup,
        )
        .await;
    }

    // Bucket update mode (no packages specified).
    // Mirrors scoop-update.ps1: `--global` is invalid without an explicit
    // <app> — reject it instead of silently ignoring the flag.
    if args.global {
        return Err(anyhow::anyhow!(hok_i18n::t!("cmd.update_global_no_app")));
    }
    // Upstream also rejects `--no-cache` without an <app>
    // (`libexec/scoop-update.ps1:393-396`).
    if args.no_cache {
        return Err(anyhow::anyhow!(hok_i18n::t!("cmd.update_no_cache_no_app")));
    }
    update_buckets(session).await?;

    // A bucket-only run upgrades nothing, so `-c` is a documented no-op here
    // rather than an error: it says what it did (nothing) and where to go.
    if args.cleanup {
        super::upgrade::cleanup_hint();
    }

    Ok(())
}

/// Update all buckets with simple inline status.
async fn update_buckets(session: &Session) -> Result<()> {
    let mut rx = session
        .event_bus()
        .receiver()
        .expect("event bus outbound receiver already taken");

    // Buckets pull concurrently (bucket::update spawns one task per bucket), so
    // each bucket gets its own spinner row that finishes in place — an inline
    // status/done pair would interleave the "Updating..."/"updated." lines when
    // buckets finish out of order. In a pipe indicatif draws nothing at all, so
    // the same events render as plain lines instead (failures and skips only):
    // a silently failed pull must not look like a successful update.
    let terminal = std::io::stdout().is_terminal();
    let handle = tokio::spawn(async move {
        let mut ui = cui::BucketRows::new(terminal);
        let mut failed: Vec<String> = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                Event::BucketUpdateProgress(ctx) => {
                    let name = ctx.name();
                    if ctx.state().started() {
                        ui.start(name);
                    } else if ctx.state().succeeded() {
                        ui.done(name);
                    } else if let Some(err) = ctx.state().failed() {
                        failed.push(name.to_owned());
                        ui.fail(
                            name,
                            err,
                            &hok_i18n::t!("cmd.bucket_update_failed", bucket = name, e = err),
                        );
                    } else if ctx.state().skipped() {
                        ui.skip(&hok_i18n::t!("cmd.bucket_not_git_skipped", bucket = name));
                    }
                }
                Event::BucketUpdateDone => break,
                _ => {}
            }
        }
        ui.finish_all();
        failed
    });

    output::header(hok_i18n::t!("cmd.header_buckets"));
    let changes = bucket::update(session).await?;
    let failed = handle.await.unwrap();

    // Refresh SQLite manifest cache with visible feedback. `bucket::update`
    // returns the manifest diff, so the cache is updated incrementally (only
    // the changed manifests are re-parsed) instead of rebuilt in full — and
    // nothing is printed at all when the cache is disabled in config.
    refresh_manifest_cache(session, &changes)?;

    // The manifest history index is maintained next to the cache but not by
    // it: it is deliberately un-gated from `use_sqlite_cache` (it backs
    // `app@<old version>` resolution, not cache serving) and it spins on its
    // own row (see `refresh_manifest_history`).
    refresh_manifest_history(session);

    // Buckets that were never fetched are named once, at the end: by now their
    // rows are settled, and this is the line that survives a scrollback or a CI
    // log. The exit code stays 0 — upstream reports a failed pull and carries
    // on (libexec/scoop-update.ps1:198 does not check the pull's status).
    if !failed.is_empty() {
        output::warn(hok_i18n::t!(
            "cmd.bucket_update_failed_summary",
            count = failed.len(),
            buckets = failed.join(", ")
        ));
    }

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

/// Maintain the manifest history index with a single-step spinner.
///
/// Same shape as [`refresh_manifest_cache`] — a `cui::RowUI` spinner row on a
/// terminal, the plain `status`/`done` line pair in a pipe, where indicatif
/// draws nothing. The row is the step's only feedback, and it earns its place:
/// a cold seed of a large bucket takes tens of seconds against milliseconds
/// once the index is warm (measured on isolated copies: extras 92,616 commits
/// 22 s on 16 threads and 97 s on one; main 81,453 commits 14 s on 16).
///
/// A failure is *not* rendered the way the cache's is. The index is derived
/// data — it backs `app@<old version>` resolution and keeps its previous
/// baseline so the next update retries — so the row ends with a dim note and
/// no `✓`, `hok update` still succeeds, and only the log records the error.
fn refresh_manifest_history(session: &Session) {
    if std::io::stdout().is_terminal() {
        let mut ui = cui::RowUI::new();
        ui.set_spin("manifest-history", hok_i18n::t!("cmd.history_index"));
        match package::manifest_history::refresh(session) {
            Ok(()) => ui.done_with("manifest-history", hok_i18n::t!("cmd.history_index_done")),
            Err(e) => {
                tracing::warn!("manifest history index refresh failed: {e}");
                ui.note(hok_i18n::t!("cmd.history_index_skipped", e = e));
                ui.finish_all();
            }
        }
    } else {
        output::status(hok_i18n::t!("cmd.history_index"));
        match package::manifest_history::refresh(session) {
            Ok(()) => output::done(hok_i18n::t!("cmd.history_index_done")),
            Err(e) => {
                tracing::warn!("manifest history index refresh failed: {e}");
                output::warn(hok_i18n::t!("cmd.history_index_skipped", e = e));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `-c/--cleanup` parses on `update` too, without stealing an existing
    /// short flag (`-o`, `-y`, `-s`, `-f`, `-i`, `-U`, `-R`, `-k`, `-g`, `-a`).
    #[test]
    fn cleanup_flag_parses_on_update() {
        assert!(!Args::try_parse_from(["update"]).unwrap().cleanup);
        assert!(Args::try_parse_from(["update", "-c"]).unwrap().cleanup);
        assert!(
            Args::try_parse_from(["update", "--cleanup"])
                .unwrap()
                .cleanup
        );
        // Still parses alongside the flags it shares a run with.
        let args = Args::try_parse_from(["update", "myapp", "-c", "-U", "-y"]).unwrap();
        assert!(args.cleanup && args.no_upgrade && args.assume_yes);
    }
}
