use clap::Parser;
use libscoop::{operation, Session, SyncOption};

use crate::{eventloop, output, Result};

/// Reinstall a package
#[derive(Debug, Parser)]
pub struct Args {
    /// The package(s) to reinstall
    #[arg(required = true)]
    package: Vec<String>,
    /// Leverage cache and suppress network access
    #[arg(short = 'o', long, action = clap::ArgAction::SetTrue)]
    offline: bool,
    /// Ignore cache and force download
    #[arg(short = 'D', long, action = clap::ArgAction::SetTrue)]
    ignore_cache: bool,
    /// Skip package integrity check
    #[arg(long, action = clap::ArgAction::SetTrue)]
    no_hash_check: bool,
    /// Ignore failures to ensure a complete transaction
    #[arg(short = 'f', long, action = clap::ArgAction::SetTrue)]
    ignore_failure: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let mut hold_set = std::collections::BTreeSet::new();
    let mut exact_queries = Vec::new();

    // Resolve queries to exact bucket-qualified names
    for q in &args.package {
        if let Ok(pkgs) = operation::package_query(
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
        }
    }

    // Release held packages temporarily
    let held: Vec<&str> = hold_set.iter().map(|s| s.as_str()).collect();
    if !held.is_empty() {
        output::status(format!("Releasing held packages: {}", held.join(", ")));
        for name in &held { operation::package_hold(session, name, false)?; }
        output::done(rust_i18n::t!("reinstall.released"));
    }

    // Step 1: Uninstall
    let queries: Vec<&str> = exact_queries.iter().map(|s| s.as_str()).collect();
    let mut opts = vec![SyncOption::AssumeYes];
    if args.ignore_failure {
        opts.push(SyncOption::IgnoreFailure);
    }

    run_remove(session, &queries, &opts)?;

    // Step 2: Install
    if args.offline {
        opts.push(SyncOption::Offline);
    }
    if args.ignore_cache {
        opts.push(SyncOption::IgnoreCache);
    }
    if args.no_hash_check {
        opts.push(SyncOption::NoHashCheck);
    }

    run_install(session, &queries, &opts)?;

    // Re-hold packages that were held before
    if !held.is_empty() {
        output::status(format!("Re-holding packages: {}", held.join(", ")));
        for name in &held { operation::package_hold(session, name, true)?; }
        output::done(rust_i18n::t!("reinstall.reheld"));
    }

    Ok(())
}

/// Uninstall phase.
fn run_remove(session: &Session, queries: &[&str], opts: &[SyncOption]) -> Result<()> {
    let handle = eventloop::run_event_loop(session, Default::default());
    operation::package_sync(session, queries.to_vec(), opts.to_vec())?;
    handle.join().unwrap();
    Ok(())
}

/// Install phase.
fn run_install(session: &Session, queries: &[&str], opts: &[SyncOption]) -> Result<()> {
    let config = eventloop::EventLoopConfig {
        auto_confirm: true,
        ..Default::default()
    };
    let handle = eventloop::run_event_loop(session, config);
    operation::package_sync(session, queries.to_vec(), opts.to_vec())?;
    handle.join().unwrap();
    Ok(())
}
