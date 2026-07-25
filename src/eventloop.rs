//! Unified event loop for package sync operations.
//!
//! Handles all Event variants emitted by libscoop during install, update,
//! upgrade, and uninstall operations. Scoop mode shows all sub-steps;
//! pacman mode shows only major events with detail hidden behind --detail.

use crate::{cui, output, util};
use crossterm::{
    cursor,
    ExecutableCommand,
};
use libscoop::{Event, Session};
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
/// calls `operation::package_sync` (or equivalent).
pub fn run_event_loop(
    session: &Session,
    config: EventLoopConfig,
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
        while let Ok(event) = rx.recv() {
            match event {
                // --- Resolve phase ---
                Event::PackageResolveStart => {
                    output::status(rust_i18n::t!("status.resolving"));
                }
                Event::PackageResolveDone => {
                    output::done(rust_i18n::t!("status.resolving_done"));
                }

                // --- Download sizing ---
                Event::PackageDownloadSizingStart => {
                    output::status(rust_i18n::t!("status.sizing"));
                }
                Event::PackageDownloadSizingDone => {}

                // --- Download ---
                Event::PackageDownloadStart => {
                    output::status(rust_i18n::t!("status.downloading"));
                }
                Event::PackageDownloadProgress(ctx) => {
                    if let Some(ref mut dp) = dlprogress {
                        let ident = ctx.ident.to_owned();
                        let url = ctx.url.to_owned();
                        let filename = ctx.filename.to_owned();
                        dp.update(ident, url, filename, ctx.dltotal, ctx.dlnow);
                    }
                }
                Event::PackageDownloadDone => {}

                // --- Integrity check ---
                Event::PackageIntegrityCheckStart => {
                    output::status(rust_i18n::t!("status.checking_hash"));
                }
                Event::PackageIntegrityCheckProgress(ctx) => {
                    output::status(rust_i18n::t!("detail.checking_hash_item", ctx = ctx));
                }
                Event::PackageIntegrityCheckDone => {
                    output::ok();
                }
                // --- Extraction ---
                Event::PackageExtractStart(ctx) => {
                    output::detail(rust_i18n::t!("detail.extracting", ctx = ctx));
                }
                Event::PackageExtractProgress(ctx) => {
                    output::detail(rust_i18n::t!("detail.extracting", ctx = ctx));
                }
                Event::PackageExtractDone => {
                    output::detail(rust_i18n::t!("detail.extract_done"));
                }

                // --- Commit (install/update/uninstall) ---
                Event::PackageCommitStart(ctx) => {
                    output::status(ctx);
                }
                Event::PackageCommitDone(ctx) => {
                    output::done(ctx);
                }

                // --- Shim operations ---
                Event::PackageShimRemoveStart => {}
                Event::PackageShimRemoveProgress(ctx) => {
                    output::detail(rust_i18n::t!("detail.removing_shim", ctx = ctx));
                }
                Event::PackageShimRemoveDone => {
                    output::detail(rust_i18n::t!("detail.shim_removed"));
                }

                Event::PackageShimAddStart(ctx) => {
                    output::detail(rust_i18n::t!("detail.creating_shim", ctx = ctx));
                }
                Event::PackageShimAddProgress(ctx) => {
                    output::detail(rust_i18n::t!("detail.creating_shim", ctx = ctx));
                }
                Event::PackageShimAddDone => {
                    output::detail(rust_i18n::t!("detail.shim_done"));
                }

                // --- Shortcut operations ---
                Event::PackageShortcutRemoveStart => {}
                Event::PackageShortcutRemoveProgress(ctx) => {
                    output::detail(rust_i18n::t!("detail.removing_shortcut", ctx = ctx));
                }
                Event::PackageShortcutRemoveDone => {
                    output::detail(rust_i18n::t!("detail.shortcut_removed"));
                }

                Event::PackageShortcutAddStart => {}
                Event::PackageShortcutAddProgress(ctx) => {
                    output::detail(rust_i18n::t!("detail.creating_shortcut", ctx = ctx));
                }
                Event::PackageShortcutAddDone => {
                    output::detail(rust_i18n::t!("detail.shortcut_done"));
                }

                // --- Environment operations ---
                Event::PackageEnvPathRemoveStart => {
                    output::detail(rust_i18n::t!("detail.removing_env_path"));
                }
                Event::PackageEnvPathRemoveDone => {
                    output::detail(rust_i18n::t!("detail.env_path_removed"));
                }

                Event::PackageEnvVarRemoveStart => {
                    output::detail(rust_i18n::t!("detail.removing_env_var"));
                }
                Event::PackageEnvVarRemoveDone => {
                    output::detail(rust_i18n::t!("detail.env_var_removed"));
                }

                // --- Persist ---
                Event::PackagePersistPurgeStart => {
                    output::detail(rust_i18n::t!("detail.removing_persist"));
                }
                Event::PackagePersistPurgeDone => {
                    output::detail(rust_i18n::t!("detail.persist_done"));
                }

                // --- PS module ---
                Event::PackagePsModuleRemoveStart(ctx) => {
                    output::detail(rust_i18n::t!("detail.removing_psmodule", ctx = ctx));
                }
                Event::PackagePsModuleRemoveDone => {
                    output::detail(rust_i18n::t!("detail.psmodule_removed"));
                }

                // --- PowerShell script output ---
                Event::ScriptOutput(line) => {
                    println!("  {line}");
                }
                Event::ScriptDone { success, stderr } => {
                    if !success && !stderr.is_empty() {
                        eprintln!("  script failed: {stderr}");
                    }
                }

                // --- Post-install notes ---
                Event::PackageNotes(note) => {
                    output::info(note);
                }

                // --- Interactive prompts ---
                Event::PromptPackageCandidate(pkgs) => {
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
                }
                Event::PromptTransactionNeedConfirm(transaction) => {
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
                            .map(|p| {
                                format!("{}-{}", p.ident(), p.upgradable_version().unwrap())
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
                    let _ = tx.send(Event::PromptTransactionNeedConfirmResult(answer));
                    let _ = std::io::stdout().execute(cursor::Hide);
                }


                // --- Version info ---
                Event::PackageVersionKnown { name, old_version, new_version } => {
                    if old_version.is_empty() {
                        output::info(format!("{}: {}", name, new_version));
                    } else {
                        output::info(format!("{}: {} -> {}", name, old_version, new_version));
                    }
                }

                // --- Cache hit ---
                Event::PackageCacheHit(filename) => {
                    output::status(rust_i18n::t!("detail.loading_cache", filename = filename));
                }

                // --- Symlink operations ---
                Event::PackageSymlinkRemove(path) => {
                    output::detail(rust_i18n::t!("detail.unlinking", path = path));
                }
                Event::PackageSymlinkCreate { from, to } => {
                    output::detail(rust_i18n::t!("detail.linking", from = from, to = to));
                }
                // --- Sync done ---
                Event::PackageSyncDone => break,

                // --- Future events ---
                _ => {}
            }
        }
    })

}
