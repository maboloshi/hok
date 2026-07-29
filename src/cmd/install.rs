use clap::{ArgAction, Parser};
use libscoop::internal::os::is_admin;
use libscoop::{operation, QueryOption, Session, SyncOption};

use crate::{output, Result};

/// Install package(s)
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// The package(s) to install
    #[arg(required = true, action = ArgAction::Append)]
    package: Vec<String>,
    /// Download package(s) without performing installation
    #[arg(short = 'd', long, action = ArgAction::SetTrue)]
    download_only: bool,
    /// Ignore failures to ensure a complete transaction
    #[arg(short = 'f', long, action = ArgAction::SetTrue)]
    ignore_failure: bool,
    /// Leverage cache and suppress network access
    #[arg(short = 'o', long, action = ArgAction::SetTrue)]
    offline: bool,
    /// Assume yes to all prompts and run non-interactively
    #[arg(short = 'y', long, action = ArgAction::SetTrue)]
    assume_yes: bool,
    /// Ignore cache and force download
    #[arg(short = 'D', long, action = ArgAction::SetTrue)]
    ignore_cache: bool,
    /// Do not install dependencies (may break packages)
    #[arg(short = 'I', long, action = ArgAction::SetTrue)]
    independent: bool,
    /// Do not replace package(s)
    #[arg(short = 'R', long, action = ArgAction::SetTrue)]
    no_replace: bool,
    /// Escape hold to allow changes on held package(s)
    #[arg(short = 'S', long, action = ArgAction::SetTrue)]
    escape_hold: bool,
    /// Do not upgrade package(s)
    #[arg(short = 'U', long, action = ArgAction::SetTrue)]
    no_upgrade: bool,
    /// Skip package integrity check
    #[arg(short = 's', long, action = ArgAction::SetTrue)]
    no_hash_check: bool,
    /// Install globally (to $SCOOP_GLOBAL)
    #[arg(short = 'g', long, action = ArgAction::SetTrue)]
    global: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let mut options = vec![];

    if args.assume_yes {
        options.push(SyncOption::AssumeYes);
    }

    if args.download_only {
        options.push(SyncOption::DownloadOnly);
    }

    if args.escape_hold {
        options.push(SyncOption::EscapeHold);
    }

    if args.ignore_failure || session.config().ignore_failures() {
        options.push(SyncOption::IgnoreFailure);
    }

    if args.ignore_cache {
        options.push(SyncOption::IgnoreCache);
    }

    if args.no_upgrade {
        options.push(SyncOption::NoUpgrade);
    }

    if args.no_replace {
        options.push(SyncOption::NoReplace);
    }

    if args.offline {
        options.push(SyncOption::Offline);
    }

    if args.independent {
        options.push(SyncOption::NoDependencies);
    }

    if args.no_hash_check {
        options.push(SyncOption::NoHashCheck);
    }

    session.set_global(args.global);

    if args.global && !is_admin() {
        anyhow::bail!("ERROR: you need admin rights to install global apps");
    }

    let queries = args.package.iter().map(|s| s.as_str()).collect::<Vec<_>>();

    // Prune already-installed packages (matching PS1's prune_installed behavior)
    let (to_install, already_installed) = prune_installed(session, &queries)?;
    for name in &already_installed {
        output::warn(format!("'{name}' is already installed. Skipping."));
    }
    if to_install.is_empty() {
        return Ok(());
    }

    let handle = crate::eventloop::run_event_loop(session, Default::default());

    operation::package_sync(session, to_install.clone(), options)?;
    handle.join().unwrap();

    // Show suggestions from manifests of installed packages
    show_suggestions(session, &to_install)?;

    Ok(())
}

/// Query installed packages and display their `suggest` field entries.
fn show_suggestions(session: &Session, packages: &[&str]) -> Result<()> {
    let installed = match operation::package_query(
        session,
        packages.to_vec(),
        vec![QueryOption::Explicit],
        true,
    ) {
        Ok(pkgs) => pkgs,
        Err(_) => return Ok(()),
    };

    for pkg in &installed {
        let manifest = pkg.manifest();
        if let Some(suggest) = manifest.suggest() {
            let name = pkg.name();
            output::info(format!("Suggestions for '{name}':"));
            for (key, values) in suggest {
                let vals = values
                    .devectorize()
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", ");
                output::info(format!("  {key}: {vals}"));
            }
        }
    }

    Ok(())
}

/// Query installed packages and split into those that need installing vs those
/// already installed. Returns `(to_install, already_installed)`.
fn prune_installed<'s>(
    session: &Session,
    queries: &[&'s str],
) -> Result<(Vec<&'s str>, Vec<String>)> {
    // Query installed packages matching the given queries (exact match)
    let installed = match operation::package_query(
        session,
        queries.to_vec(),
        vec![QueryOption::Explicit],
        true,
    ) {
        Ok(pkgs) => pkgs,
        Err(_) => return Ok((queries.to_vec(), vec![])),
    };

    let mut already_installed = Vec::new();
    let mut to_install = Vec::new();

    for q in queries {
        let installed_names: Vec<&str> = installed
            .iter()
            .filter(|p| {
                // Match by exact name (case-insensitive) or bucket/name
                let q_normalized = q.to_lowercase();
                let p_name = p.name().to_lowercase();
                let p_ident = p.ident().to_lowercase();
                q_normalized == p_name || q_normalized == p_ident
            })
            .map(|p| p.name())
            .collect();

        if installed_names.is_empty() {
            to_install.push(*q);
        } else {
            already_installed.push(installed_names[0].to_string());
        }
    }

    Ok((to_install, already_installed))
}