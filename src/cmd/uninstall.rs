//! Uninstall package(s).

use clap::Parser;
use libscoop::{Session, SyncOption, package};

use crate::cmd::shared_args::ensure_global;
use crate::{Result, output};

/// Uninstall package(s)
#[derive(Debug, Parser)]
pub struct Args {
    /// The package(s) to uninstall
    #[arg(required = true)]
    package: Vec<String>,
    /// Ignore failures to ensure a complete transaction (persistent setting:
    /// 'hok config ignore_failures true')
    #[arg(long, action = clap::ArgAction::SetTrue)]
    ignore_failures: bool,
    /// Assume yes to all prompts and run non-interactively
    #[arg(short = 'y', long, action = clap::ArgAction::SetTrue)]
    assume_yes: bool,
    /// Skip package integrity check
    #[arg(short = 's', long, action = clap::ArgAction::SetTrue)]
    skip_hash_check: bool,
    /// Uninstall a globally installed app (from $SCOOP_GLOBAL)
    #[arg(short = 'g', long, action = clap::ArgAction::SetTrue)]
    global: bool,
    /// Remove all persistent data
    #[arg(short = 'p', long, action = clap::ArgAction::SetTrue)]
    purge: bool,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    ensure_global(session, args.global, "uninstall")?;

    let mut options = vec![SyncOption::Remove];

    if args.purge {
        options.push(SyncOption::Purge);
    }

    if args.assume_yes {
        options.push(SyncOption::AssumeYes);
    }

    if args.ignore_failures || session.config().ignore_failures() {
        options.push(SyncOption::IgnoreFailures);
    }

    if args.skip_hash_check {
        options.push(SyncOption::SkipHashCheck);
    }

    let queries = args.package.iter().map(|s| s.as_str()).collect::<Vec<_>>();

    // `remove` reports every query that isn't installed and skips it
    // (`Confirm-InstallationStatus`, `lib/core.ps1:1084-1121`); a batch with
    // nothing installed therefore still exits 0 but must not end in the
    // success footer. The same call `remove` uses decides it here, `Broken`
    // included so a partial install still counts as installed.
    let any_installed = !package::query::query_installed(
        session,
        &queries,
        &[
            libscoop::QueryOption::Explicit,
            libscoop::QueryOption::Broken,
        ],
    )?
    .is_empty();

    let handle = crate::eventloop::run_event_loop_default(session);

    let sync_result = package::sync::sync(session, queries, options).await;
    handle.await.unwrap();
    // A declined confirmation is not a failure: exit 0 without ok_all.
    if matches!(sync_result, Err(libscoop::Error::UserAborted)) {
        return Ok(());
    }
    sync_result?;

    if any_installed {
        output::done(hok_i18n::t!("output.ok_all"));
    }

    Ok(())
}
