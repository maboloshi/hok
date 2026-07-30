//! Scoop-style event handler.
//!
//! Renders events using Scoop's classic output style (step-by-step messages
//! with "ok" / "done" indicators). This is the default handler.

use libscoop::{Event, EventHandler};

use crate::output;
use std::io::Write;

/// Scoop-style event handler.
///
/// Displays all sub-steps with status/done messages, matching
/// Scoop's original CLI output style.
pub struct ScoopHandler;

impl EventHandler for ScoopHandler {
    fn handle(&mut self, event: &Event) {
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
                print!("\r  {}", rust_i18n::t!("detail.extracting", ctx = ctx));
                let _ = std::io::stdout().flush();
            }
            Event::PackageExtractProgress(ctx) => {
                print!("\r  {}", rust_i18n::t!("detail.extracting", ctx = ctx));
                let _ = std::io::stdout().flush();
            }
            Event::PackageExtractDone => {
                println!();
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

            // --- Shortcut warnings ---
            Event::PackageShortcutConflict(path) => {
                output::warn(rust_i18n::t!("detail.shortcut_conflict", path = path));
            }
            Event::PackageShortcutNotFound(path) => {
                output::warn(rust_i18n::t!("detail.shortcut_not_found", path = path));
            }

            // --- Session warnings ---
            Event::ConfigLoadFallback => {
                output::warn(rust_i18n::t!("detail.config_fallback"));
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

            // --- Held package skipped ---
            Event::PackageHeld { name, version } => {
                output::warn(rust_i18n::t!("cmd.held_skip", name = name, version = version));
            }

            // --- Sync done ---
            Event::PackageSyncDone => {}

            // --- Interactive prompts (handled by event loop, not here) ---
            Event::PromptPackageCandidate(_) | Event::PromptTransactionNeedConfirm(_) => {}

            // --- Download progress (handled by event loop, not here) ---
            Event::PackageDownloadProgress(_) => {}

            // --- Future events ---
            _ => {}
        }
    }
}

impl ScoopHandler {
    pub fn new() -> Self {
        Self
    }
}
