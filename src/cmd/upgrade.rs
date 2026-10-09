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
    /// Skip package integrity check
    #[arg(short = 's', long, action = ArgAction::SetTrue)]
    skip_hash_check: bool,
    /// Force upgrade: reinstall apps that are already at the requested version
    /// (Scoop's `-f/--force`)
    #[arg(short = 'f', long, action = ArgAction::SetTrue)]
    force: bool,
    /// Don't install dependencies automatically
    #[arg(short = 'i', long, action = ArgAction::SetTrue)]
    independent: bool,
    /// Don't use the download cache
    #[arg(short = 'k', long, action = ArgAction::SetTrue)]
    no_cache: bool,
    /// Install globally (to $SCOOP_GLOBAL)
    #[arg(short = 'g', long, action = ArgAction::SetTrue)]
    global: bool,
    /// Update all installed packages (alternative to '*')
    #[arg(short = 'a', long, action = clap::ArgAction::SetTrue)]
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
            no_replace: false,
            no_upgrade: false,
            no_cache: self.no_cache,
            download_only: false,
        }
    }
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    // --all is a shorthand for upgrading all packages (like passing '*'),
    // matching `update --all` semantics.
    if args.all && args.package.is_empty() {
        return execute_upgrade(
            session,
            &[String::from("*")],
            &args.sync_args(),
            args.force,
            args.cleanup,
        )
        .await;
    }

    execute_upgrade(
        session,
        &args.package,
        &args.sync_args(),
        args.force,
        args.cleanup,
    )
    .await
}

/// The upgrade engine: `upgrade` always runs it, `update` delegates to it
/// whenever packages are named (or `--all` is given).
///
/// `cleanup` is the `-c/--cleanup` phase: once the batch has succeeded, the
/// old versions of exactly the packages whose installed version changed are
/// removed. The versions are snapshotted before the batch for that reason — a
/// sweep of *all* apps would delete leftovers the user never asked about.
pub(super) async fn execute_upgrade(
    session: &Session,
    packages: &[String],
    sync: &SyncArgs,
    force: bool,
    cleanup: bool,
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
    // The installed versions `-c` measures the batch against (empty without it).
    let before = if cleanup {
        installed_versions(session, &queries)?
    } else {
        Vec::new()
    };
    // A named target that isn't installed must NOT be reported as up-to-date:
    // the batch still runs so `sync` reports it and the command exits 0
    // (`Confirm-InstallationStatus`, `lib/core.ps1:1084-1121`;
    // `scoop-update.ps1:424-427,468`).
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
            // Nothing can have been upgraded, so `-c` is a documented no-op
            // that says so instead of silently doing nothing.
            if cleanup {
                cleanup_upgraded(session, &queries, &before)?;
            }
            return Ok(());
        }
    } else {
        options.push(SyncOption::Force);
    }

    let handle = crate::eventloop::run_event_loop_default(session);

    let sync_result = package::sync::sync(session, queries.clone(), options).await;
    handle.await.unwrap();
    // A declined confirmation is not a failure: exit 0.
    if matches!(sync_result, Err(libscoop::Error::UserAborted)) {
        return Ok(());
    }
    sync_result?;

    if cleanup {
        cleanup_upgraded(session, &queries, &before)?;
    }

    Ok(())
}

/// The installed `(name, version)` pairs a query resolves to.
fn installed_versions(session: &Session, queries: &[&str]) -> Result<Vec<(String, String)>> {
    Ok(package::query::query_installed(session, queries, &[])?
        .into_iter()
        .map(|pkg| (pkg.name().to_owned(), pkg.version().to_owned()))
        .collect())
}

/// The names whose installed version changed between two snapshots.
///
/// An app that disappeared (or that the after-snapshot no longer reports) is
/// *not* an upgrade: there are no old versions of it left to remove.
fn upgraded_names(before: &[(String, String)], after: &[(String, String)]) -> Vec<String> {
    before
        .iter()
        .filter(|(name, version)| {
            after
                .iter()
                .any(|(now_name, now_version)| now_name == name && now_version != version)
        })
        .map(|(name, _)| name.clone())
        .collect()
}

/// The `-c/--cleanup` phase: remove the old versions of the packages this run
/// actually upgraded.
///
/// The sweep is restricted to [`upgraded_names`] — an app that merely has a
/// leftover version directory is left alone, as is the download cache (that is
/// `hok cleanup -k`'s job). A run that upgraded nothing is a no-op and points
/// the user at `hok cleanup -a` instead.
fn cleanup_upgraded(
    session: &Session,
    queries: &[&str],
    before: &[(String, String)],
) -> Result<()> {
    let names = upgraded_names(before, &installed_versions(session, queries)?);
    if names.is_empty() {
        cleanup_hint();
        return Ok(());
    }

    super::cleanup::sweep(session, &names, false)?;

    Ok(())
}

/// Tell the user `-c` had nothing to clean up, and where to go instead.
pub(super) fn cleanup_hint() {
    output::info(hok_i18n::t!("cmd.update_cleanup_no_upgrade"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(name: &str, version: &str) -> (String, String) {
        (name.to_owned(), version.to_owned())
    }

    /// `-c` must target only the apps whose installed version changed: an app
    /// with a leftover version dir that was *not* upgraded is left alone, and a
    /// vanished/new app is not an upgrade either.
    #[test]
    fn upgraded_names_selects_only_changed_versions() {
        let before = vec![
            pair("upgraded", "1.0.0"),
            pair("current", "1.0.0"),
            pair("gone", "1.0.0"),
        ];
        let after = vec![
            pair("upgraded", "2.0.0"),
            pair("current", "1.0.0"),
            pair("installed-now", "3.0.0"),
        ];

        assert_eq!(
            upgraded_names(&before, &after),
            vec!["upgraded".to_owned()],
            "only a version change counts as an upgrade"
        );
    }

    /// The no-op shape: nothing changed, so nothing is swept.
    #[test]
    fn upgraded_names_is_empty_without_a_version_change() {
        let snapshot = vec![pair("a", "1.0.0"), pair("b", "2.0.0")];
        assert!(upgraded_names(&snapshot, &snapshot).is_empty());
        assert!(upgraded_names(&[], &snapshot).is_empty());
    }

    /// `-c/--cleanup` exists on both upgrade entry points (and is off by
    /// default): the flag must not silently fall through to another option.
    #[test]
    fn cleanup_flag_parses_on_upgrade() {
        assert!(!Args::try_parse_from(["upgrade"]).unwrap().cleanup);
        assert!(Args::try_parse_from(["upgrade", "-c"]).unwrap().cleanup);
        assert!(
            Args::try_parse_from(["upgrade", "--cleanup"])
                .unwrap()
                .cleanup
        );
    }
}
