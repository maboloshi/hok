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

    /// Ignore cache and force download (Scoop: -k, --no-cache)
    #[arg(short = 'D', long, short_alias = 'k', visible_alias = "no-cache", action = ArgAction::SetTrue)]
    ignore_cache: bool,

    /// Skip package integrity check (Scoop: --skip-hash-check)
    #[arg(short = 's', long, visible_alias = "skip-hash-check", action = ArgAction::SetTrue)]
    no_hash_check: bool,

    /// Ignore failures to ensure a complete transaction
    #[arg(short = 'f', long, action = ArgAction::SetTrue)]
    ignore_failure: bool,

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
            ignore_failure: self.ignore_failure,
            offline: self.offline,
            no_hash_check: self.no_hash_check,
            independent: false,
            no_replace: false,
            escape_hold: false,
            no_upgrade: false,
            ignore_cache: self.ignore_cache,
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

    // Per-package running-process check. A running app aborts the whole
    // reinstall unless `ignore_failures` is enabled — the app's
    // process-in-use failure is then skipped while the rest of the batch is
    // reinstalled. When `ignore_running_processes` is enabled,
    // check_not_running already printed a warning and the app proceeds.
    // Skipped apps are also dropped from the hold set so their hold is
    // never released.
    let mut keep = Vec::new();
    for q in &exact_queries {
        let name = q.rsplit('/').next().unwrap_or(q.as_str());
        match package::sync::check_not_running(session, name, "reinstalling") {
            Ok(_) => keep.push(q.clone()),
            Err(libscoop::Error::AppRunning(name))
                if all_opts.contains(&SyncOption::IgnoreFailure) =>
            {
                output::warn(hok_i18n::t!(
                    "reinstall.running_process",
                    name = name
                ));
                hold_set.remove(&name);
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
        output::status(hok_i18n::t!(
            "reinstall.releasing",
            names = held.join(", ")
        ));
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
        output::status(hok_i18n::t!(
            "reinstall.reholding",
            names = held.join(", ")
        ));
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
