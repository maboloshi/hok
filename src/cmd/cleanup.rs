//! Clean up apps by removing old versions.

use clap::{ArgAction, Parser};
use libscoop::package::cleanup::CleanupOptions;
use libscoop::{Session, cache, package};

use crate::cmd::shared_args::ensure_global;
use crate::{Result, output};

/// Cleanup apps by removing old versions
#[derive(Debug, Parser)]
#[clap()]
pub struct Args {
    /// Given named app(s) to be cleaned up (use '*' or --all to cleanup all apps)
    #[arg(action = ArgAction::Append)]
    app: Vec<String>,
    /// Cleanup all apps (alternative to '*')
    #[arg(short = 'a', long, action = ArgAction::SetTrue)]
    all: bool,
    /// Cleanup a globally installed app
    #[arg(short = 'g', long, action = ArgAction::SetTrue)]
    global: bool,
    /// Remove outdated download cache simultaneously
    #[arg(short = 'k', long, action = ArgAction::SetTrue)]
    cache: bool,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    // --all or '*' overrides any specific app names
    let all = args.all || args.app.iter().any(|a| a == "*");

    // Upstream refuses a bare `cleanup` rather than sweeping every app
    // (`scoop-cleanup.ps1:24`).
    if args.app.is_empty() && !args.all {
        return Err(anyhow::anyhow!(hok_i18n::t!("cmd.cleanup_app_missing")));
    }

    // Upstream requires elevation for any `--global` cleanup, including the
    // sweep-all form (`scoop-cleanup.ps1:26-28`).
    ensure_global(session, args.global, "cleanup")?;

    // Cleanup is a maintenance operation; individual failures should not abort it.
    let options = CleanupOptions {
        ignore_failures: true,
        cache: args.cache,
    };

    let mut results = Vec::new();
    // `Some(0)` means every named app was reported missing/broken: the global
    // "nothing to clean" line would then contradict the per-app errors.
    let mut named_targets = None;

    if all {
        // Local first, then global — upstream appends the global apps to the
        // local list (`scoop-cleanup.ps1:67-70`). The app list is always
        // explicit, so a named cleanup that resolved to nothing cannot fall
        // back to a full sweep.
        session.set_global(false);
        let names = package::cleanup::installed_app_names(session);
        results.extend(package::cleanup::cleanup(session, &names, &options)?);
        if args.global {
            session.set_global(true);
            let names = package::cleanup::installed_app_names(session);
            results.extend(package::cleanup::cleanup(session, &names, &options)?);
        }
    } else {
        // Named apps go through upstream's `Confirm-InstallationStatus`
        // (`lib/core.ps1:1084-1121`): unusable names are reported and dropped,
        // the rest are cleaned in the requested scope.
        let names = package::cleanup::confirm_installation_status(session, &args.app, args.global);
        named_targets = Some(names.len());
        session.set_global(args.global);
        results.extend(package::cleanup::cleanup(session, &names, &options)?);
    }

    for (name, count, failed) in &results {
        let msg = if *failed > 0 {
            hok_i18n::t!(
                "cmd.cleanup_removed_failed",
                count = *count,
                failed = *failed
            )
        } else {
            hok_i18n::t!("cmd.cleanup_removed", count = *count)
        };
        output::named(name.as_str(), msg);
    }

    if results.is_empty() {
        if named_targets != Some(0) {
            output::info(hok_i18n::t!("cmd.cleanup_no_old"));
        }
    } else if all {
        // Sweep-all success tail (Scoop prints this for the non-verbose
        // `--all` form); named-app cleans already reported per-app counts.
        output::done(hok_i18n::t!("cmd.cleanup_all_done"));
    }

    if args.cache {
        // Upstream clears leftover partial downloads once, after the sweep
        // (`scoop-cleanup.ps1:79-81`).
        cache::remove_incomplete(session)?;
    }

    Ok(())
}
