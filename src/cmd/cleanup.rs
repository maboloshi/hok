//! Clean up apps by removing old versions.

use clap::{ArgAction, Parser};
use libscoop::package::cleanup::CleanupOptions;
use libscoop::{Session, cache, package};

use crate::cmd::shared_args::ensure_global;
use crate::output::style;
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
    /// Also report apps that are already clean
    // A count, not a flag: `-v` is the CLI's global verbosity option, so
    // `hok cleanup -v` and `hok -v cleanup` share it (as `alias list` and
    // `info` already do).
    #[arg(short = 'v', long, action = ArgAction::Count)]
    verbose: u8,
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
        results.extend(package::cleanup::cleanup(
            session,
            &names,
            &options,
            &|_| {},
        )?);
        if args.global {
            session.set_global(true);
            let names = package::cleanup::installed_app_names(session);
            results.extend(package::cleanup::cleanup(
                session,
                &names,
                &options,
                &|_| {},
            )?);
        }
    } else {
        // Named apps go through upstream's `Confirm-InstallationStatus`
        // (`lib/core.ps1:1084-1121`): unusable names are reported and dropped,
        // the rest are cleaned in the requested scope.
        let names = package::cleanup::confirm_installation_status(session, &args.app, args.global);
        named_targets = Some(names.len());
        session.set_global(args.global);
        results.extend(package::cleanup::cleanup(
            session,
            &names,
            &options,
            &|_| {},
        )?);
    }

    // Upstream renders the removed versions on the app's line
    // (`Removing $app: <v1> <v2>`, `scoop-cleanup.ps1:43-46`); its verbose
    // `"$app is already clean"` note (`:39`) maps to hok's `-v`. `reported`
    // tracks whether any per-app line was emitted, so the fallback summary
    // does not repeat what those lines already said.
    let mut reported = false;
    for res in &results {
        if !res.removed.is_empty() {
            println!(
                "  {} {}",
                style::warning(hok_i18n::t!("cmd.cleanup_removing", name = res.name)),
                res.removed.join(" ")
            );
            reported = true;
        } else if args.verbose > 0 && res.failures.is_empty() {
            output::done(hok_i18n::t!("cmd.cleanup_already_clean", name = res.name));
            reported = true;
        }

        if !res.failures.is_empty() {
            output::named(
                res.name.as_str(),
                hok_i18n::t!(
                    "cmd.cleanup_removed_failed",
                    count = res.removed.len(),
                    failed = res.failures.len()
                ),
            );
        }
    }

    // Upstream closes the sweep-all form with this line unconditionally
    // (`scoop-cleanup.ps1:83-85`); the named form has no summary.
    if all {
        output::done(hok_i18n::t!("cmd.cleanup_all_done"));
    } else if !reported && named_targets != Some(0) {
        output::info(hok_i18n::t!("cmd.cleanup_no_old"));
    }

    if args.cache {
        // Upstream clears leftover partial downloads once, after the sweep
        // (`scoop-cleanup.ps1:79-81`).
        cache::remove_incomplete(session)?;
    }

    Ok(())
}
