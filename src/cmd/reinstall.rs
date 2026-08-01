use clap::{ArgAction, Parser};
use libscoop::internal::os::is_admin;
use libscoop::{package, Session, SyncOption};

use crate::cmd::shared_args::{Cmd, SyncArgs};
use crate::{eventloop, output, Result};

/// Reinstall a package
#[derive(Debug, Parser)]
pub struct Args {
    /// The package(s) to reinstall
    #[arg(required = true)]
    package: Vec<String>,

    /// Leverage cache and suppress network access
    #[arg(short = 'o', long, action = ArgAction::SetTrue)]
    offline: bool,

    /// Ignore cache and force download
    #[arg(short = 'D', long, action = ArgAction::SetTrue)]
    ignore_cache: bool,

    /// Skip package integrity check
    #[arg(short = 's', long, action = ArgAction::SetTrue)]
    no_hash_check: bool,

    /// Ignore failures to ensure a complete transaction
    #[arg(short = 'f', long, action = ArgAction::SetTrue)]
    ignore_failure: bool,

    /// Install globally (to $SCOOP_GLOBAL)
    #[arg(short = 'g', long, action = ArgAction::SetTrue)]
    global: bool,
}

impl Cmd for Args {
    type Args = Self;

    fn execute(args: Self::Args, session: &Session) -> Result<()> {
        session.set_global(args.global);
        if args.global && !is_admin() {
            anyhow::bail!("ERROR: you need admin rights to install global apps");
        }

        // Build SyncArgs once, reuse for both phases
        let sync = SyncArgs {
            global: args.global,
            assume_yes: true,
            ignore_failure: args.ignore_failure,
            offline: args.offline,
            no_hash_check: args.no_hash_check,
            independent: false,
            no_replace: false,
            escape_hold: false,
            no_upgrade: false,
            ignore_cache: args.ignore_cache,
            download_only: false,
        };

        let mut hold_set = std::collections::BTreeSet::new();
        let mut exact_queries = Vec::new();

        // Resolve queries to exact bucket-qualified names.
        // Only installed packages (with install.json + manifest.json) are accepted.
        for q in &args.package {
            if let Ok(pkgs) = package::query::query(
                session,
                vec![q.as_str()],
                vec![libscoop::QueryOption::Explicit],
                true,
            ) {
                for pkg in &pkgs {
                    if pkg.is_held() {
                        hold_set.insert(pkg.name().to_string());
                    }
                    exact_queries.push(format!("{}/{}", pkg.bucket(), pkg.name()));
                }
            } else {
                return Err(anyhow::anyhow!(
                    rust_i18n::t!("cmd.reinstall_not_installed", name = q)
                ));
            }
        }

        // Release held packages temporarily
        let held: Vec<&str> = hold_set.iter().map(|s| s.as_str()).collect();
        if !held.is_empty() {
            output::status(format!("Releasing held packages: {}", held.join(", ")));
            for name in &held {
                package::hold::hold(session, name, false)?;
            }
            output::done(rust_i18n::t!("reinstall.released"));
        }

        let queries: Vec<&str> = exact_queries.iter().map(|s| s.as_str()).collect();

        // Step 1: Uninstall
        let all_opts = sync.to_sync_options(session);
        let mut remove_opts = all_opts.clone();
        remove_opts.push(SyncOption::Remove);
        run_remove(session, &queries, &remove_opts)?;

        // Step 2: Install
        run_install(session, &queries, &all_opts)?;

        // Re-hold packages that were held before
        if !held.is_empty() {
            output::status(format!("Re-holding packages: {}", held.join(", ")));
            for name in &held {
                package::hold::hold(session, name, true)?;
            }
            output::done(rust_i18n::t!("reinstall.reheld"));
        }

        Ok(())
    }
}

/// Module-level execute for dispatch compatibility.
#[inline]
pub fn execute(args: Args, session: &Session) -> Result<()> {
    <Args as Cmd>::execute(args, session)
}

/// Uninstall phase.
fn run_remove(session: &Session, queries: &[&str], opts: &[SyncOption]) -> Result<()> {
    let handle = eventloop::run_event_loop_default(session);
    package::sync::sync(session, queries.to_vec(), opts.to_vec())?;
    handle.join().unwrap();
    Ok(())
}

/// Install phase.
fn run_install(session: &Session, queries: &[&str], opts: &[SyncOption]) -> Result<()> {
    let config = eventloop::EventLoopConfig {
        auto_confirm: true,
        ..Default::default()
    };
    let handle = eventloop::run_event_loop(
        session,
        config,
        Box::new(crate::scoop_handler::ScoopHandler::new()),
    );
    package::sync::sync(session, queries.to_vec(), opts.to_vec())?;
    handle.join().unwrap();
    Ok(())
}
