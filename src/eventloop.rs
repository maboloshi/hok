//! Unified event loop for package sync operations.
//!
//! Spawns a tokio task to receive events from libscoop's event bus while the
//! calling async task runs the sync operation. Download progress bars, cursor
//! management, and interactive prompts are handled here; all other event
//! rendering is delegated to an EventHandler.
//!
//! # Design
//!
//! - **Separation of concerns**: The loop handles only UI-critical interactions
//!   (progress bars, confirmations, candidate selection). All other events are
//!   forwarded to a user-provided EventHandler trait object.
//! - **Non-blocking**: Runs as a tokio task on the multi-thread runtime, so
//!   the sync operation (e.g. package_sync) can proceed concurrently without
//!   occupying a dedicated OS thread.
//! - **Cursor safety**: Uses CursorGuard (RAII) to hide the cursor during
//!   progress updates and restore it on panic, cancellation or normal exit.
//!
//! # Extending the Event Loop
//!
//! - **To add new interactive logic** (e.g., a new user prompt) **for all commands**:
//!   add a new match arm in run_event_loop for the corresponding Event variant.
//!   Remember to temporarily show the cursor (cursor::Show) before reading input.
//! - **To customise output for existing events** (e.g., log formatting) **without
//!   changing interactive behaviour**: implement a custom EventHandler and pass
//!   it to run_event_loop instead of using run_event_loop_default.
//! - **To change global defaults** (progress bars): modify
//!   run_event_loop_default as needed.
//!
//! # Important Notes
//!
//! - Always use `continue` after processing an event if you **do not** want the
//!   event to be passed to the EventHandler as well.
//! - Avoid long-running operations inside match arms; they block the event
//!   loop. Offload heavy work with spawn_blocking where needed.
//! - When sending responses back to libscoop via tx.send().await, ignore
//!   potential errors (the receiver may have closed) — use let _ = ... .
//! - Blocking stdin reads must not run directly in the async task (they would
//!   occupy a runtime worker thread); they are wrapped in
//!   tokio::task::spawn_blocking.

use crate::{cui, format, output};
use crossterm::{ExecutableCommand, cursor};
use libscoop::{Event, EventHandler, Session};
use std::io::Write;

/// RAII guard: hides cursor on construction, restores on drop.
struct CursorGuard;
impl CursorGuard {
    fn hide() -> Self {
        let _ = std::io::stdout().execute(cursor::Hide);
        CursorGuard
    }
}
impl Drop for CursorGuard {
    fn drop(&mut self) {
        let _ = std::io::stdout().execute(cursor::Show);
    }
}

/// Run the unified event loop for a package sync operation.
///
/// Spawns a tokio task to process events while the calling async task calls
/// package::sync::sync (or equivalent).
///
/// The handler is called for each event to render output.
pub fn run_event_loop(
    session: &Session,
    mut handler: Box<dyn EventHandler>,
) -> tokio::task::JoinHandle<()> {
    // The outbound receiver is single-consumer (tokio::sync::mpsc), so it is
    // handed out exactly once here and moved into the task.
    let mut rx = session
        .event_bus()
        .receiver()
        .expect("event bus outbound receiver already taken");
    let tx = session.event_bus().sender();
    let mut dlprogress = Some(cui::RowUI::new());

    tokio::spawn(async move {
        let _guard = CursorGuard::hide();
        // tokio::select! is the orchestration point between the backend event
        // stream and (in the prompt arms below) the frontend's stdin input.
        loop {
            let event = tokio::select! {
                biased;
                maybe = rx.recv() => match maybe {
                    Some(e) => e,
                    // The session dropped its outbound sender: stop.
                    None => break,
                },
            };
            match event {
                // Download progress — update progress bars (handled here)
                Event::PackageDownloadProgress(ref ctx) => {
                    if let Some(ref mut dp) = dlprogress {
                        dp.set_bar(&ctx.ident, &ctx.url, ctx.dltotal, ctx.dlnow);
                    }
                    continue;
                }

                // Interactive prompt: select from multiple package candidates
                Event::PromptPackageCandidate(ref pkgs) => {
                    // Non-empty by construction: resolve.rs only emits this
                    // when there is more than one candidate.
                    let name = pkgs[0].split_once('/').map(|x| x.1).unwrap_or(&pkgs[0]);
                    println!("Found multiple candidates for package '{}':\n", name);
                    for (i, pkg) in pkgs.iter().enumerate() {
                        println!("  {}: {}", i, pkg);
                    }
                    let pkgs_len = pkgs.len();

                    let _ = std::io::stdout().execute(cursor::Show);
                    let index = tokio::task::spawn_blocking(move || {
                        loop {
                            output::prompt(rust_i18n::t!("output.select_prompt"));
                            std::io::stdout().flush().unwrap();
                            let mut input = String::new();
                            // EOF/read error (piped/no input): fall back to the
                            // first candidate instead of spinning forever.
                            match std::io::stdin().read_line(&mut input) {
                                Ok(0) | Err(_) => break 0,
                                Ok(_) => {}
                            }
                            if let Ok(num) = input.trim().parse::<usize>()
                                && num < pkgs_len
                            {
                                break num;
                            }
                        }
                    })
                    .await
                    .unwrap_or(0);
                    let _ = std::io::stdout().execute(cursor::Hide);
                    let _ = tx.send(Event::PromptPackageCandidateResult(index)).await;
                    continue;
                }

                // Interactive prompt: confirm the transaction
                Event::PromptTransactionNeedConfirm(ref transaction) => {
                    if let Some(install) = transaction.install_view() {
                        output::header(rust_i18n::t!("cmd.header_installed"));
                        let out = install
                            .iter()
                            .map(|p| format!("{}-{}", p.ident(), p.version()))
                            .collect::<Vec<_>>()
                            .join("  ");
                        println!("  {}", out);
                    }

                    if let Some(upgrade) = transaction.upgrade_view() {
                        if transaction.install_view().is_some() {
                            println!();
                        }
                        output::header(rust_i18n::t!("cmd.header_upgraded"));
                        let out = upgrade
                            .iter()
                            // With `--force` packages at their current version
                            // (no upgradable reference) are reinstalled too, so
                            // fall back to the current version for display.
                            .map(|p| {
                                format!(
                                    "{}-{}",
                                    p.ident(),
                                    p.upgradable_version().unwrap_or(p.version())
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("  ");
                        println!("  {}", out);
                    }

                    if let Some(replace) = transaction.replace_view() {
                        if transaction.install_view().is_some()
                            || transaction.upgrade_view().is_some()
                        {
                            println!();
                        }
                        output::header(rust_i18n::t!("cmd.header_replaced"));
                        let out = replace
                            .iter()
                            .map(|p| format!("{}/{}", p.bucket(), p.name()))
                            .collect::<Vec<_>>()
                            .join("  ");
                        println!("  {}", out);
                    }

                    if let Some(remove) = transaction.remove_view() {
                        if transaction.install_view().is_some()
                            || transaction.upgrade_view().is_some()
                            || transaction.replace_view().is_some()
                        {
                            println!();
                        }
                        output::header(rust_i18n::t!("cmd.header_removed"));
                        let out = remove
                            .iter()
                            .map(|p| format!("{}-{}", p.ident(), p.version()))
                            .collect::<Vec<_>>()
                            .join("  ");
                        println!("  {}", out);
                    }

                    if let Some(download_size) = transaction.download_size() {
                        let out = format::humansize(download_size.total, true);
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

                    let _ = std::io::stdout().execute(cursor::Show);
                    let answer = tokio::task::spawn_blocking(output::prompt_yes_no)
                        .await
                        .unwrap_or(false);
                    let _ = tx
                        .send(Event::PromptTransactionNeedConfirmResult(answer))
                        .await;
                    let _ = std::io::stdout().execute(cursor::Hide);
                    continue;
                }

                // Sync done — end the loop. The "all apps are up to date"
                // message is the update command's concern (it knows whether
                // anything was upgradable), not the event loop's.
                Event::PackageSyncDone => break,

                // Standalone download done — no install/update semantics.
                Event::DownloadDone => break,

                // All other events — delegate to the handler for rendering
                _ => {}
            }

            // Let the handler render this event
            handler.handle(&event);
        }

        handler.on_finished();
    })
}

/// Convenience helper: runs the event loop with a default Scoop-style handler.
pub fn run_event_loop_default(session: &Session) -> tokio::task::JoinHandle<()> {
    run_event_loop(session, Box::new(crate::scoop_handler::ScoopHandler::new()))
}
