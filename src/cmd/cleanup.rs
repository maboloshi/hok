//! Clean up apps by removing old versions.

use std::io::IsTerminal;
use std::sync::Mutex;

use clap::{ArgAction, Parser};
use libscoop::package::cleanup::{CleanupOptions, CleanupProgress, CleanupResult};
use libscoop::{Session, cache, package};

use crate::cmd::shared_args::ensure_global;
use crate::cui::RowUI;
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

/// Per-version progress rendering.
///
/// On a terminal every old version gets its own spinner row, which finishes in
/// place as `✓ app: 1.0.0` or `✗ app: 1.0.0 (err)`: the sweep reports each
/// version as it goes instead of dumping the whole batch at the end, and a
/// failure stays on the row of the version that failed. In a pipe `indicatif`
/// draws nothing, so the same lines are rendered from the settled results
/// (which also keeps a redirected run in the requested order).
enum Progress {
    Rich(RowUI),
    Pipe,
}

impl Progress {
    fn new(terminal: bool) -> Progress {
        if terminal {
            Progress::Rich(RowUI::new())
        } else {
            Progress::Pipe
        }
    }

    /// Render one notification from the sweep's workers (a terminal only).
    fn render(&mut self, progress: &CleanupProgress) {
        let Progress::Rich(ui) = self else {
            return;
        };
        match progress {
            CleanupProgress::Started { name, version } => {
                let label = row_label(name, version);
                let verb = hok_i18n::t!("cmd.cleanup_removing", name = name).to_string();
                ui.set_spin(&label, format!("{verb} {version} ..."));
            }
            CleanupProgress::Finished {
                name,
                version,
                error,
            } => {
                let label = row_label(name, version);
                match error {
                    None => ui.done(&label),
                    Some(error) => ui.fail(&label, error),
                }
            }
        }
    }

    /// Settle any row the sweep left open (when it aborted on an error).
    fn finish(&mut self) {
        if let Progress::Rich(ui) = self {
            ui.finish_all();
        }
    }
}

/// The label of one version's row: also the text of its settled line, so a
/// terminal row and a piped line read identically (`app: 1.0.0`).
fn row_label(name: &str, version: &str) -> String {
    hok_i18n::t!("cmd.cleanup_row", name = name, ver = version).to_string()
}

/// Render one settled result as plain lines: the piped counterpart of the
/// terminal rows.
fn render_settled(result: &CleanupResult, verbose: u8) {
    for version in &result.removed {
        output::done(row_label(&result.name, version));
    }
    for failure in &result.failures {
        eprintln!(
            "  {} {} ({})",
            style::danger_bold("✗"),
            style::emphasis(row_label(&result.name, &failure.version)),
            style::danger(&failure.error)
        );
    }
    if verbose > 0 && result.removed.is_empty() && result.failures.is_empty() {
        output::done(hok_i18n::t!(
            "cmd.cleanup_already_clean",
            name = result.name
        ));
    }
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

    // The sweep removes version directories in parallel and reports each one as
    // it goes; its rows are settled even when the sweep itself fails.
    let terminal = std::io::stdout().is_terminal();
    let progress = Mutex::new(Progress::new(terminal));
    let observer = |event: CleanupProgress| progress.lock().unwrap().render(&event);

    let mut results = Vec::new();
    // `Some(0)` means every named app was reported missing/broken: the global
    // "nothing to clean" line would then contradict the per-app errors.
    let mut named_targets = None;

    let sweep = (|| -> Result<()> {
        if all {
            // Local first, then global — upstream appends the global apps to the
            // local list (`scoop-cleanup.ps1:67-70`). The app list is always
            // explicit, so a named cleanup that resolved to nothing cannot fall
            // back to a full sweep.
            session.set_global(false);
            let names = package::cleanup::installed_app_names(session);
            results.extend(package::cleanup::cleanup(
                session, &names, &options, &observer,
            )?);
            if args.global {
                session.set_global(true);
                let names = package::cleanup::installed_app_names(session);
                results.extend(package::cleanup::cleanup(
                    session, &names, &options, &observer,
                )?);
            }
        } else {
            // Named apps go through upstream's `Confirm-InstallationStatus`
            // (`lib/core.ps1:1084-1121`): unusable names are reported and
            // dropped, the rest are cleaned in the requested scope.
            let names =
                package::cleanup::confirm_installation_status(session, &args.app, args.global);
            named_targets = Some(names.len());
            session.set_global(args.global);
            results.extend(package::cleanup::cleanup(
                session, &names, &options, &observer,
            )?);
        }
        Ok(())
    })();

    progress.lock().unwrap().finish();
    sweep?;

    // The settled lines. A terminal already rendered them as rows (only the
    // verbose "already clean" notes are left); a pipe has no rows, so every
    // line is rendered here, in the requested order.
    if terminal {
        if args.verbose > 0 {
            for result in &results {
                if result.removed.is_empty() && result.failures.is_empty() {
                    output::done(hok_i18n::t!(
                        "cmd.cleanup_already_clean",
                        name = result.name
                    ));
                }
            }
        }
    } else {
        for result in &results {
            render_settled(result, args.verbose);
        }
    }

    // Versions that survived are the one thing worth repeating at the end: the
    // rows scroll away, this line survives a scrollback or a CI log. Upstream
    // aborts the whole sweep on the first failure; hok finishes it and reports
    // here, so the exit code stays 0.
    let failed: Vec<&str> = results
        .iter()
        .filter(|result| !result.failures.is_empty())
        .map(|result| result.name.as_str())
        .collect();
    let reported = results.iter().any(|result| !result.removed.is_empty())
        || (args.verbose > 0
            && results
                .iter()
                .any(|result| result.removed.is_empty() && result.failures.is_empty()));

    if !failed.is_empty() {
        output::warn(hok_i18n::t!(
            "cmd.cleanup_failed_summary",
            count = failed.len(),
            apps = failed.join(", ")
        ));
    } else if all {
        // Upstream closes the sweep-all form with this line
        // (`scoop-cleanup.ps1:83-85`); the named form has no summary.
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
