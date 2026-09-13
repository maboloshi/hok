//! Upgrade installed package(s).

use clap::{ArgAction, Parser};
use libscoop::{Session, SyncOption, package};

use crate::Result;
use crate::cmd::shared_args::{SyncArgs, SyncFlags, ensure_global};
use crate::output;

/// Upgrade installed package(s)
#[derive(Debug, Parser)]
pub struct Args {
    /// The package(s) to be upgraded (default: all except held)
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
    /// Escape hold to allow to upgrade held package(s)
    #[arg(short = 'S', long, action = ArgAction::SetTrue)]
    escape_hold: bool,
    /// Skip package integrity check
    #[arg(short = 's', long, action = ArgAction::SetTrue)]
    skip_hash_check: bool,
    /// Force upgrade: reinstall apps that are already at the requested version
    /// (Scoop's `-f/--force`)
    #[arg(short = 'f', long, action = ArgAction::SetTrue)]
    force: bool,
    /// Install globally (to $SCOOP_GLOBAL)
    #[arg(short = 'g', long, action = ArgAction::SetTrue)]
    global: bool,
    /// Update all installed packages (alternative to '*')
    #[arg(short = 'a', long, action = clap::ArgAction::SetTrue)]
    all: bool,
}

impl SyncFlags for Args {
    fn sync_args(&self) -> SyncArgs {
        SyncArgs {
            global: self.global,
            assume_yes: self.assume_yes,
            ignore_failures: self.ignore_failures,
            offline: self.offline,
            skip_hash_check: self.skip_hash_check,
            independent: false,
            no_replace: false,
            escape_hold: self.escape_hold,
            no_upgrade: false,
            no_cache: false,
            download_only: false,
        }
    }
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    // --all is a shorthand for upgrading all packages (like passing '*'),
    // matching `update --all` semantics.
    if args.all && args.package.is_empty() {
        return execute_upgrade(session, &[String::from("*")], &args.sync_args(), args.force).await;
    }

    execute_upgrade(session, &args.package, &args.sync_args(), args.force).await
}

/// The upgrade engine: `upgrade` always runs it, `update` delegates to it
/// whenever packages are named (or `--all` is given).
pub(super) async fn execute_upgrade(
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
    // A named target that isn't installed must NOT be reported as up-to-date:
    // the batch still runs so `sync` reports it (`Confirm-InstallationStatus`,
    // `lib/core.ps1:1084-1121`), and the closing success footer is then
    // suppressed — Scoop reports the app and exits 0 without a success line
    // (`scoop-update.ps1:424-427,468`).
    let mut missing_target = false;
    for &q in &queries {
        if q != "*" && package::query::query_installed(session, &[q], &[])?.is_empty() {
            missing_target = true;
            break;
        }
    }
    if !force {
        // "All apps are up to date" is upgrade semantics: the upgrade engine
        // decides it, not the event loop. Nothing upgradable and at least
        // one of the targets installed means there is nothing to do.
        let upgradable = package::query::query_installed(
            session,
            &queries,
            &[libscoop::QueryOption::Upgradable],
        )?;
        let installed = package::query::query_installed(session, &queries, &[])?;
        if !missing_target && upgradable.is_empty() && !installed.is_empty() {
            // Upstream names every app it found current and closes with a hint
            // (`libexec/scoop-update.ps1:442,457`).
            for pkg in &installed {
                println!(
                    "{}",
                    hok_i18n::t!(
                        "cmd.update_latest",
                        name = pkg.name(),
                        version = pkg.version()
                    )
                );
            }
            output::info(hok_i18n::t!("cmd.update_all_latest"));
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

    if missing_target {
        return Ok(());
    }

    output::done(hok_i18n::t!("output.ok_all"));

    Ok(())
}
