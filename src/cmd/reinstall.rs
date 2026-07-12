use clap::{ArgAction, Parser};
use crossterm::ExecutableCommand;
use libscoop::{operation, Event, QueryOption, Session, SyncOption};

use crate::{cui, output, Result};

/// Reinstall package(s) (uninstall then install)
#[derive(Debug, Parser)]
pub struct Args {
    /// The package(s) to reinstall
    #[arg(required = true, action = ArgAction::Append)]
    package: Vec<String>,
    /// Assume yes to all prompts and run non-interactively
    #[arg(short = 'y', long, action = ArgAction::SetTrue)]
    assume_yes: bool,
    /// Ignore cache and force re-download
    #[arg(short = 'f', long, action = ArgAction::SetTrue)]
    force: bool,
    /// Ignore failures to ensure a complete transaction
    #[arg(short = 'i', long, action = ArgAction::SetTrue)]
    ignore_failure: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let queries: Vec<&str> = args.package.iter().map(|s| s.as_str()).collect();

    // Snapshot which packages are held and get their full bucket-qualified names
    let (held_names, install_queries) = find_held_and_queries(session, &queries);

    let mut opts = vec![];
    if args.assume_yes {
        opts.push(SyncOption::AssumeYes);
    }
    if args.ignore_failure || session.config().ignore_failures() {
        opts.push(SyncOption::IgnoreFailure);
    }
    if args.force {
        opts.push(SyncOption::IgnoreCache);
    }

    // Phase 1: Uninstall (respect --assume-yes, user may want to review)
    let mut remove_opts = vec![SyncOption::Remove, SyncOption::EscapeHold];
    remove_opts.extend_from_slice(&opts);
    run_remove(session, &queries, &remove_opts)?;

    // Phase 2: Install same version (always auto-confirm — reinstall is explicit intent)
    let install_queries_ref: Vec<&str> = install_queries.iter().map(|s| s.as_str()).collect();
    let mut install_opts = vec![SyncOption::NoUpgrade, SyncOption::EscapeHold, SyncOption::AssumeYes];
    install_opts.extend_from_slice(&opts);
    run_install(session, &install_queries_ref, &install_opts)?;

    // Phase 3: Restore held status for previously held packages
    for name in &held_names {
        let _ = operation::package_hold(session, name, true);
    }

    Ok(())
}

/// Query which target packages are currently held, returning held names
/// and bucket-qualified queries for exact reinstall.
fn find_held_and_queries(session: &Session, queries: &[&str]) -> (Vec<String>, Vec<String>) {
    let mut held = Vec::new();
    let mut exact_queries = Vec::new();
    for q in queries {
        if let Ok(pkgs) = operation::package_query(session, vec![q], vec![QueryOption::Explicit], true) {
            for pkg in &pkgs {
                if pkg.is_held() {
                    held.push(pkg.name().to_string());
                }
                // Use bucket-qualified name to avoid candidate selection on reinstall
                exact_queries.push(format!("{}/{}", pkg.bucket(), pkg.name()));
            }
        }
    }
    (held, exact_queries)
}

/// Uninstall event handler with simple output.
fn run_remove(session: &Session, queries: &[&str], opts: &[SyncOption]) -> Result<()> {
    let rx = session.event_bus().receiver();
    let tx = session.event_bus().sender();

    let handle = std::thread::spawn(move || {
        while let Ok(event) = rx.recv() {
            match event {
                Event::PackageResolveStart => output::status("Resolving packages..."),
                Event::PromptTransactionNeedConfirm(transaction) => {
                    if let Some(remove) = transaction.remove_view() {
                        output::header("The following packages will be REMOVED:");
                        let output = remove
                            .iter()
                            .map(|p| format!("{}-{}", p.ident(), p.version()))
                            .collect::<Vec<_>>()
                            .join("  ");
                        println!("  {output}");
                    }
                    let answer = cui::prompt_yes_no();
                    let _ = tx.send(Event::PromptTransactionNeedConfirmResult(answer));
                }
                Event::PackageCommitStart(ctx) => output::status(format!("Uninstalling {ctx}...")),
                Event::PackageShortcutRemoveProgress(ctx) => output::detail(format!("removing shortcut: {ctx}")),
                Event::PackageShimRemoveProgress(ctx) => output::detail(format!("removing shim: {ctx}")),
                Event::PackageCommitDone(ctx) => output::done(format!("'{ctx}' was uninstalled.")),
                Event::PackageSyncDone => break,
                Event::PackageExtractStart(ctx) => output::detail(format!("extract: {ctx}")),
                _ => {}
            }
        }
    });

    operation::package_sync(session, queries.to_vec(), opts.to_vec())?;
    handle.join().unwrap();
    Ok(())
}

/// Install event handler with download progress bar.
fn run_install(session: &Session, queries: &[&str], opts: &[SyncOption]) -> Result<()> {
    let rx = session.event_bus().receiver();
    let tx = session.event_bus().sender();

    let mut dlprogress = cui::MultiProgressUI::new();
    let mut stdout = std::io::stdout();
    let _ = stdout.execute(crossterm::cursor::Hide);

    let handle = std::thread::spawn(move || {
        while let Ok(event) = rx.recv() {
            match event {
                Event::PackageResolveStart => output::status("Resolving packages..."),
                Event::PackageDownloadSizingStart => output::status("Calculating download size..."),
                Event::PackageDownloadStart => output::status("Downloading packages..."),
                Event::PackageDownloadProgress(ctx) => {
                    dlprogress.update(
                        ctx.ident.to_owned(),
                        ctx.url.to_owned(),
                        ctx.filename.to_owned(),
                        ctx.dltotal,
                        ctx.dlnow,
                    );
                }
                Event::PackageDownloadDone => {}
                Event::PackageIntegrityCheckStart => output::status("Checking package integrity..."),
                Event::PackageIntegrityCheckDone => output::done("Checking package integrity...Ok"),
                Event::PromptTransactionNeedConfirm(_) => {
                    // Auto-confirm (AssumeYes is always set for reinstall)
                    let _ = tx.send(Event::PromptTransactionNeedConfirmResult(true));
                }
                Event::PackageCommitStart(ctx) => output::status(format!("Installing {ctx}...")),
                Event::PackageExtractProgress(ctx) => output::detail(format!("extracting: {ctx}")),
                Event::PackageShimAddProgress(ctx) => output::detail(format!("creating shim: {ctx}")),
                Event::PackageShortcutAddProgress(ctx) => output::detail(format!("creating shortcut: {ctx}")),
                Event::PackageCommitDone(ctx) => output::done(format!("'{ctx}' was installed.")),
                Event::PackageSyncDone => break,
                Event::PackageExtractStart(ctx) => output::detail(format!("extract: {ctx}")),
                _ => {}
            }
        }
    });

    operation::package_sync(session, queries.to_vec(), opts.to_vec())?;
    handle.join().unwrap();
    Ok(())
}
