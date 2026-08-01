//! Unified event loop for package sync operations.
//!
//! Spawns a dedicated thread to receive events from libscoop's event bus
//! while the main thread runs the sync operation. Download progress bars,
//! cursor management, and interactive prompts are handled here; all other
//! event rendering is delegated to an [`EventHandler`].
//!
//! # Design
//!
//! - **Separation of concerns**: The loop handles only UI‑critical interactions
//!   (progress bars, confirmations, candidate selection). All other events are
//!   forwarded to a user‑provided [`EventHandler`] trait object.
//! - **Non‑blocking**: Runs in a dedicated thread, so the sync operation
//!   (e.g. `package_sync`) can proceed concurrently.
//! - **Cursor safety**: Uses [`CursorGuard`] (RAII) to hide the cursor during
//!   progress updates and restore it on panic or normal exit.
//!
//! # Extending the Event Loop
//!
//! - **To add new interactive logic** (e.g., a new user prompt) **for all commands**:
//!   add a new match arm in `run_event_loop` for the corresponding [`Event`] variant.
//!   Remember to temporarily show the cursor (`cursor::Show`) before reading input.
//! - **To customise output for existing events** (e.g., log formatting) **without
//!   changing interactive behaviour**: implement a custom [`EventHandler`] and pass
//!   it to `run_event_loop` instead of using `run_event_loop_default`.
//! - **To change global defaults** (progress bars, auto‑confirm): modify
//!   [`EventLoopConfig`] or `run_event_loop_default` as needed.
//!
//! # Important Notes
//!
//! - Always use `continue` after processing an event if you **do not** want the
//!   event to be passed to the [`EventHandler`] as well.
//! - Avoid long‑running operations inside match arms; they block the event loop.
//!   Offload heavy work to separate threads if necessary.
//! - When sending responses back to `libscoop` via `tx.send()`, ignore potential
//!   errors (the receiver may have closed) – use `let _ = tx.send(...)`.
//! - If you add new fields to [`EventLoopConfig`], ensure all callers (like `update`,
//!   `remove` commands) set them or rely on `Default`.
//!
//! For detailed examples, see the documentation of [`EventHandler`] and the
//! implementations in the `scoop_handler` module.

use crate::{cui, output, util};
use crossterm::{cursor, ExecutableCommand};
use libscoop::{Event, EventHandler, Session};
use std::io::Write;

/// Controls event loop behavior.
pub struct EventLoopConfig {
    /// Whether to show download progress bars (install/update).
    pub show_progress_bars: bool,
    /// Auto-confirm transaction prompts (for reinstall).
    pub auto_confirm: bool,
}

impl Default for EventLoopConfig {
    fn default() -> Self {
        Self {
            show_progress_bars: true,
            auto_confirm: false,
        }
    }
}

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
/// Spawns a dedicated thread to process events while the current thread
/// calls `package::sync::sync` (or equivalent).
///
/// The `handler` is called for each event to render output.
pub fn run_event_loop(
    session: &Session,
    config: EventLoopConfig,
    mut handler: Box<dyn EventHandler>,
) -> std::thread::JoinHandle<()> {
    let rx = session.event_bus().receiver();
    let tx = session.event_bus().sender();
    let mut dlprogress = if config.show_progress_bars {
        Some(cui::MultiProgressUI::new())
    } else {
        None
    };

    std::thread::spawn(move || {
        let _guard = CursorGuard::hide();
        let mut committed = 0;
        let mut user_cancelled = false;
        while let Ok(event) = rx.recv() {
            match event {
                // Download progress — update progress bars (handled here)
                Event::PackageDownloadProgress(ref ctx) => {
                    if let Some(ref mut dp) = dlprogress {
                        dp.update(
                            ctx.ident.to_owned(),
                            ctx.url.to_owned(),
                            ctx.filename.to_owned(),
                            ctx.dltotal,
                            ctx.dlnow,
                        );
                    }
                    continue;
                }

                // Interactive prompt: select from multiple package candidates
                Event::PromptPackageCandidate(ref pkgs) => {
                    let name = pkgs[0].split_once('/').map(|x| x.1).unwrap_or(&pkgs[0]);
                    println!("Found multiple candidates for package '{}':\n", name);
                    for (i, pkg) in pkgs.iter().enumerate() {
                        println!("  {}: {}", i, pkg);
                    }

                    let _ = std::io::stdout().execute(cursor::Show);
                    let index = loop {
                        output::prompt(rust_i18n::t!("output.select_prompt"));
                        std::io::stdout().flush().unwrap();
                        let mut input = String::new();
                        std::io::stdin().read_line(&mut input).unwrap();
                        if let Ok(num) = input.trim().parse::<usize>() {
                            if num < pkgs.len() {
                                break num;
                            }
                        }
                    };
                    let _ = std::io::stdout().execute(cursor::Hide);
                    let _ = tx.send(Event::PromptPackageCandidateResult(index));
                    continue;
                }

                // Interactive prompt: confirm the transaction
                Event::PromptTransactionNeedConfirm(ref transaction) => {
                    if config.auto_confirm {
                        let _ = tx.send(Event::PromptTransactionNeedConfirmResult(true));
                        continue;
                    }

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
                            .map(|p| format!("{}-{}", p.ident(), p.upgradable_version().unwrap()))
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

                    let _ = std::io::stdout().execute(cursor::Show);
                    let answer = output::prompt_yes_no();
                    user_cancelled = !answer;
                    let _ = tx.send(Event::PromptTransactionNeedConfirmResult(answer));
                    let _ = std::io::stdout().execute(cursor::Hide);
                    continue;
                }

                // Sync done — track committed count
                Event::PackageSyncDone => {
                    if committed == 0 && !user_cancelled {
                        output::info(rust_i18n::t!("cmd.outdated"));
                    }
                    break;
                }

                // Commit tracking
                Event::PackageCommitDone(_) => {
                    committed += 1;
                }

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
pub fn run_event_loop_default(session: &Session) -> std::thread::JoinHandle<()> {
    run_event_loop(
        session,
        EventLoopConfig::default(),
        Box::new(crate::scoop_handler::ScoopHandler::new()),
    )
}
