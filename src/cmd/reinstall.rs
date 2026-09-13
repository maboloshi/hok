//! Reinstall a package.

use clap::{ArgAction, Parser};
use libscoop::{Session, SyncOption, package};

use crate::cmd::shared_args::{SyncArgs, SyncFlags, ensure_global};
use crate::{Result, eventloop, output};

/// Reinstall a package
#[derive(Debug, Parser)]
pub struct Args {
    /// The package(s) to reinstall
    #[arg(required = true)]
    package: Vec<String>,

    /// Leverage cache and suppress network access
    #[arg(short = 'o', long, action = ArgAction::SetTrue)]
    offline: bool,

    /// Don't use the download cache
    #[arg(short = 'k', long, action = ArgAction::SetTrue)]
    no_cache: bool,

    /// Skip package integrity check
    #[arg(short = 's', long, action = ArgAction::SetTrue)]
    skip_hash_check: bool,

    /// Ignore failures to ensure a complete transaction (persistent setting:
    /// 'hok config ignore_failures true')
    #[arg(long, action = ArgAction::SetTrue)]
    ignore_failures: bool,

    /// Install globally (to $SCOOP_GLOBAL)
    #[arg(short = 'g', long, action = ArgAction::SetTrue)]
    global: bool,
}

impl SyncFlags for Args {
    fn sync_args(&self) -> SyncArgs {
        SyncArgs {
            global: self.global,
            // Reinstall always runs non-interactively; the remaining flags
            // are not exposed by `scoop reinstall`.
            assume_yes: true,
            ignore_failures: self.ignore_failures,
            offline: self.offline,
            skip_hash_check: self.skip_hash_check,
            independent: false,
            no_replace: false,
            no_upgrade: false,
            no_cache: self.no_cache,
            download_only: false,
        }
    }
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    ensure_global(session, args.global, "reinstall")?;

    // Build sync options once, reuse for both phases
    let all_opts = args.to_sync_options(session);

    let mut hold_set = std::collections::BTreeSet::new();
    let mut exact_queries = Vec::new();

    // Resolve queries to exact bucket-qualified names.
    // `Broken` is included so a partial/failed install (missing `current` or
    // manifest.json/install.json) can be recovered: reinstall uninstalls the
    // broken app dir then reinstalls from its bucket source — the recovery
    // Scoop suggests with "Please reinstall it or fix the manifest.".
    for q in &args.package {
        match package::query::query(
            session,
            vec![q.as_str()],
            vec![
                libscoop::QueryOption::Explicit,
                libscoop::QueryOption::Broken,
            ],
            true,
        ) {
            Ok(pkgs) if !pkgs.is_empty() => {
                for pkg in &pkgs {
                    if pkg.is_held() {
                        hold_set.insert(pkg.name().to_string());
                    }
                    exact_queries.push(format!("{}/{}", pkg.bucket(), pkg.name()));
                }
            }
            // Empty resolution must not silently succeed: sync() would
            // otherwise run both phases with zero queries and merely
            // report "all apps are up to date".
            _ => {
                return Err(anyhow::anyhow!(hok_i18n::t!(
                    "cmd.reinstall_not_installed",
                    name = q
                )));
            }
        }
    }

    // Per-package running-process check. A running app is skipped — the check
    // printed the reason — and the rest of the batch is reinstalled, matching
    // Scoop's uninstall/update behaviour; `ignore_running_processes` turns that
    // skip into a carry-on. Skipped apps are also dropped from the hold set so
    // their hold is never released.
    let mut keep = Vec::new();
    for q in &exact_queries {
        let name = q.rsplit('/').next().unwrap_or(q.as_str());
        match package::sync::check_not_running(
            session,
            name,
            package::sync::RunningAction::Reinstall,
        ) {
            Ok(package::sync::RunningCheck::Running) => {
                hold_set.remove(name);
            }
            Ok(package::sync::RunningCheck::NotRunning | package::sync::RunningCheck::Ignored) => {
                keep.push(q.clone());
            }
            Err(e) => return Err(e.into()),
        }
    }
    let exact_queries = keep;
    if exact_queries.is_empty() {
        return Ok(());
    }

    // Release held packages temporarily
    let held: Vec<&str> = hold_set.iter().map(|s| s.as_str()).collect();
    if !held.is_empty() {
        output::status(hok_i18n::t!("reinstall.releasing", names = held.join(", ")));
        for name in &held {
            package::hold::hold(session, name, false)?;
        }
        output::done(hok_i18n::t!("reinstall.released"));
    }

    let queries: Vec<&str> = exact_queries.iter().map(|s| s.as_str()).collect();

    // Step 1: Uninstall
    let mut remove_opts = all_opts.clone();
    remove_opts.push(SyncOption::Remove);
    run_sync_phase(session, &queries, &remove_opts).await?;

    // The outbound receiver is single-consumer: after the remove phase's
    // event loop has finished (broken on PackageSyncDone and drained), reset
    // the bus so the install phase gets a fresh outbound channel.
    session.event_bus().reset_outbound();

    // Step 2: Install
    run_sync_phase(session, &queries, &all_opts).await?;

    // Re-hold packages that were held before
    if !held.is_empty() {
        output::status(hok_i18n::t!("reinstall.reholding", names = held.join(", ")));
        for name in &held {
            package::hold::hold(session, name, true)?;
        }
        output::done(hok_i18n::t!("reinstall.reheld"));
    }

    output::done(hok_i18n::t!("output.ok_all"));

    Ok(())
}

/// Run one sync phase under an event loop on the session's event bus.
///
/// The outbound receiver is single-consumer: the first phase takes it, and
/// before the second phase runs the bus must be reset explicitly (see the
/// callers) so a fresh outbound channel is handed to the next event loop.
async fn run_sync_phase(session: &Session, queries: &[&str], opts: &[SyncOption]) -> Result<()> {
    let handle = eventloop::run_event_loop_default(session);
    package::sync::sync(session, queries.to_vec(), opts.to_vec()).await?;
    handle.await.unwrap();
    Ok(())
}
