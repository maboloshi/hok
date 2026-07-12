use clap::{ArgAction, Parser};
use libscoop::{operation, Event, Session, SyncOption};

use crate::{cui, output, Result};

/// Uninstall package(s)
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// The package(s) to uninstall
    #[arg(required = true, action = ArgAction::Append)]
    package: Vec<String>,
    /// Remove unneeded dependencies as well
    #[arg(short = 'c', long, action = ArgAction::SetTrue)]
    cascade: bool,
    /// Purge package(s) persistent data as well
    #[arg(short = 'p', long, action = ArgAction::SetTrue)]
    purge: bool,
    /// Assume yes to all prompts and run non-interactively
    #[arg(short = 'y', long, action = ArgAction::SetTrue)]
    assume_yes: bool,
    /// Disable dependent check (may break other packages)
    #[arg(long, action = ArgAction::SetTrue)]
    no_dependent_check: bool,
    /// Escape hold to allow to uninstall held package(s)
    #[arg(short = 'S', long, action = ArgAction::SetTrue)]
    escape_hold: bool,
    /// Ignore failures to ensure a complete transaction
    #[arg(short = 'f', long, action = ArgAction::SetTrue)]
    ignore_failure: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let queries = args.package.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    let mut options = vec![SyncOption::Remove];

    if args.assume_yes {
        options.push(SyncOption::AssumeYes);
    }

    if args.cascade {
        options.push(SyncOption::Cascade);
    }

    if args.no_dependent_check {
        options.push(SyncOption::NoDependentCheck);
    }

    if args.escape_hold {
        options.push(SyncOption::EscapeHold);
    }

    if args.purge {
        options.push(SyncOption::Purge);
    }

    if args.ignore_failure || session.config().ignore_failures() {
        options.push(SyncOption::IgnoreFailure);
    }

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
                            .map(|p| {
                                format!(
                                "{}-{}",
                                p.ident(),
                                p.version(),
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("  ");
                        println!("  {}", output);
                    }

                    let answer = cui::prompt_yes_no();
                    let _ = tx.send(Event::PromptTransactionNeedConfirmResult(answer));
                }
                Event::PackageCommitStart(ctx) => {
                    output::status(format!("Uninstalling {ctx}..."));
                }
                Event::PackageShortcutRemoveProgress(ctx) => {
                    output::detail(format!("removing shortcut: {ctx}"));
                }
                Event::PackageShimRemoveProgress(ctx) => {
                    output::detail(format!("removing shim: {ctx}"));
                }
                Event::PackagePersistPurgeStart => {
                    output::detail("removing persisted data...");
                }
                Event::PackageCommitDone(ctx) => {
                    output::done(format!("'{ctx}' was uninstalled."));
                }
                Event::PackageSyncDone => break,
                Event::PackageExtractStart(ctx) => output::detail(format!("extract: {ctx}")),
                _ => {}
            }
        }
    });

    operation::package_sync(session, queries, options)?;
    handle.join().unwrap();

    Ok(())
}
