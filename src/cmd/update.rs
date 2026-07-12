use clap::{ArgAction, Parser};
use crossterm::{
    cursor,
    terminal::{Clear, ClearType},
    ExecutableCommand,
};
use libscoop::{operation, Event, Session, SyncOption};

use crate::{cui, output, util, Result};

/// Fetch and update subscribed buckets, or upgrade installed package(s)
///
/// Examples:
///   hok update         update buckets only
///   hok update <app>   upgrade a specific package (Scoop-compatible)
///   hok update *       upgrade all packages
#[derive(Debug, Parser)]
pub struct Args {
    /// The package(s) to be upgraded (omit to only update buckets)
    #[arg(action = ArgAction::Append)]
    pub package: Vec<String>,
    /// Ignore failures to ensure a complete transaction
    #[arg(short = 'f', long, action = ArgAction::SetTrue)]
    pub ignore_failure: bool,
    /// Leverage cache and suppress network access
    #[arg(short = 'o', long, action = ArgAction::SetTrue)]
    pub offline: bool,
    /// Assume yes to all prompts and run non-interactively
    #[arg(short = 'y', long, action = ArgAction::SetTrue)]
    pub assume_yes: bool,
    /// Escape hold to allow to upgrade held package(s)
    #[arg(short = 'S', long, action = ArgAction::SetTrue)]
    pub escape_hold: bool,
    /// Skip package integrity check
    #[arg(long, action = ArgAction::SetTrue)]
    pub no_hash_check: bool,
    /// Force update even within cooldown period
    #[arg(long, action = ArgAction::SetTrue)]
    pub force: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    if args.package.is_empty() {
        update_buckets(session, args.force)
    } else {
        execute_upgrade(session, &args.package, &args)
    }
}

fn update_buckets(session: &Session, force: bool) -> Result<()> {
    // Cooldown: skip if buckets were updated less than 15 minutes ago (unless --force)
    if !force {
        if let Some(remaining) = session.config().update_cooldown_remaining() {
            output::status(format!(
                "Buckets recently updated. Next update allowed in ~{remaining}s. Use --force to update now."
            ));
            return Ok(());
        }
    }

    let rx = session.event_bus().receiver();

    let handle = std::thread::spawn(move || {
        let mut progress = cui::BucketUpdateUI::new();

        while let Ok(event) = rx.recv() {
            match event {
                Event::BucketUpdateProgress(ctx) => {
                    if ctx.state().started() {
                        progress.add(ctx.name());
                    } else if ctx.state().succeeded() {
                        progress.succeed(ctx.name());
                    } else {
                        let err_msg = ctx.state().failed().unwrap();
                        progress.fail(ctx.name(), err_msg);
                    }
                }
                Event::BucketUpdateDone => break,
                _ => {}
            }
        }

        let mut stdout = std::io::stdout();
        let step = (progress.data.len() - progress.cursor) as u16;
        let _ = stdout.execute(cursor::MoveToNextLine(step)).unwrap();
    });

    output::header("Updating buckets");

    let mut stdout = std::io::stdout();
    let _ = stdout.execute(cursor::Hide);

    operation::bucket_update(session)?;

    handle.join().unwrap();

    // Refresh SQLite manifest cache with visible feedback
    if session.config().use_sqlite_cache() {
        output::status("Refreshing manifest cache...");
        operation::refresh_manifest_cache(session);
        output::done("Manifest cache refreshed.");
    }

    let _ = stdout.execute(cursor::Show);

    Ok(())
}

/// Shared upgrade logic — used by both `update` (when packages given) and `upgrade`.
pub fn execute_upgrade(session: &Session, packages: &[String], args: &Args) -> Result<()> {
    let mut queries = packages.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    if queries.is_empty() {
        queries.push("*");
    }
    let mut options = vec![SyncOption::OnlyUpgrade];

    if args.assume_yes {
        options.push(SyncOption::AssumeYes);
    }

    if args.escape_hold {
        options.push(SyncOption::EscapeHold);
    }

    if args.ignore_failure || session.config().ignore_failures() {
        options.push(SyncOption::IgnoreFailure);
    }

    if args.offline {
        options.push(SyncOption::Offline);
    }

    if args.no_hash_check {
        options.push(SyncOption::NoHashCheck);
    }

    let rx = session.event_bus().receiver();
    let tx = session.event_bus().sender();

    let mut stdout = std::io::stdout();
    let _ = stdout.execute(cursor::Hide);

    let mut dlprogress = cui::MultiProgressUI::new();

    let handle = std::thread::spawn(move || {
        while let Ok(event) = rx.recv() {
            match event {
                Event::PackageResolveStart => output::status("Resolving packages..."),
                Event::PackageDownloadSizingStart => output::status("Calculating download size..."),
                Event::PackageDownloadStart => output::status("Downloading packages..."),
                Event::PackageDownloadProgress(ctx) => {
                    let ident = ctx.ident.to_owned();
                    let url = ctx.url.to_owned();
                    let filename = ctx.filename.to_owned();
                    let dltotal = ctx.dltotal;
                    let dlnow = ctx.dlnow;

                    dlprogress.update(ident, url, filename, dltotal, dlnow);
                }
                Event::PackageDownloadDone => {}
                Event::PackageIntegrityCheckStart => output::status("Checking package integrity..."),
                Event::PackageIntegrityCheckProgress(ctx) => {
                    let mut stdout = std::io::stdout();
                    stdout
                        .execute(cursor::MoveToPreviousLine(1))
                        .unwrap()
                        .execute(Clear(ClearType::CurrentLine))
                        .unwrap();
                    println!("Checking package integrity...{ctx}");
                }
                Event::PackageIntegrityCheckDone => {
                    let mut stdout = std::io::stdout();
                    stdout
                        .execute(cursor::MoveToPreviousLine(1))
                        .unwrap()
                        .execute(Clear(ClearType::CurrentLine))
                        .unwrap();
                    println!("Checking package integrity...Ok");
                }
                Event::PromptTransactionNeedConfirm(transaction) => {
                    if let Some(install) = transaction.install_view() {
                        output::header("The following packages will be INSTALLED:");
                        let output = install
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

                    if let Some(upgrade) = transaction.upgrade_view() {
                        if transaction.install_view().is_some() {
                            println!();
                        }
                        output::header("The following packages will be UPGRADED:");
                        let output = upgrade
                            .iter()
                            .map(|p| {
                                format!(
                                    "{}-{}",
                                    p.ident(),
                                    p.upgradable_version().unwrap(),
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("  ");
                        println!("  {}", output);
                    }

                    if let Some(replace) = transaction.replace_view() {
                        if transaction.install_view().is_some()
                            || transaction.upgrade_view().is_some()
                        {
                            println!();
                        }
                        output::header("The following packages will be REPLACED:");
                        let output = replace
                            .iter()
                            .map(|p| {
                                format!(
                                    "{}/{}",
                                    p.bucket(),
                                    p.name(),
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("  ");
                        println!("  {}", output);
                    }

                    if let Some(download_size) = transaction.download_size() {
                        let out = util::humansize(download_size.total, true);
                        if download_size.total > 0 {
                            if download_size.estimated {
                                println!("\nTotal download size: {out} (estimated)");
                            } else {
                                println!("\nTotal download size: {}", out);
                            }
                        } else {
                            println!("\nNothing to download, all cached.");
                        }
                    }

                    let mut stdout = std::io::stdout();
                    let _ = stdout.execute(cursor::Show);
                    let answer = cui::prompt_yes_no();
                    let _ = tx.send(Event::PromptTransactionNeedConfirmResult(answer));
                    let _ = stdout.execute(cursor::Hide);
                }
                Event::PackageSyncDone => break,
                Event::PackageExtractStart(ctx) => output::detail(format!("extract: {ctx}")),
                _ => {}
            }
        }
    });

    operation::package_sync(session, queries, options)?;

    handle.join().unwrap();

    let _ = stdout.execute(cursor::Show);

    Ok(())
}
