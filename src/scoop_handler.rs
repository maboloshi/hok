//! Default event handler for package sync operations.
//!
//! Renders coarse-grained events to stdout (phase headers, package notes,
//! warnings, version info) using the single [`crate::output`] renderer.
//! Granular sub-step details (shim/shortcut/env/persist/ps-module operations)
//! are routed to the `tracing` log channel instead — visible only with
//! `-v` / `-vv` / `-vvv` — keeping stdout minimal and constant.

use libscoop::{Event, EventHandler};

use crate::output;
use std::io::Write;

/// Default event handler.
pub struct ScoopHandler {
    /// The package currently being extracted (set by PackageExtractStart),
    /// shown alongside the throttled file counter on progress events.
    extracting: Option<String>,
}

impl EventHandler for ScoopHandler {
    fn handle(&mut self, event: &Event) {
        match event {
            // --- Resolve phase ---
            Event::PackageResolveStart => {
                output::status(rust_i18n::t!("status.resolving"));
            }
            Event::PackageResolveDone => {}

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
            Event::PackageIntegrityCheckDone => {}

            // --- Extraction ---
            // Progress lines are redrawn in place with `\r`; clear the whole
            // line first (`\x1b[2K`) so a shorter name doesn't leave tail
            // characters of the previous (longer) line behind. Progress
            // carries a bare `n/total` counter (no filename), prefixed with
            // the package name from the start event.
            Event::PackageExtractStart(ctx) => {
                self.extracting = Some(ctx.clone());
                print!(
                    "\r\x1b[2K  {}",
                    rust_i18n::t!("detail.extracting", ctx = ctx)
                );
                let _ = std::io::stdout().flush();
            }
            Event::PackageExtractProgress(ctx) => {
                // `ctx` is "n/total" for native extraction or "7z NN%" for the
                // external fallback — the latter already carries a label.
                let ctx = if ctx.contains('/') {
                    match self.extracting.as_deref() {
                        Some(label) => format!("{label} {ctx}"),
                        None => ctx.clone(),
                    }
                } else {
                    ctx.clone()
                };
                print!(
                    "\r\x1b[2K  {}",
                    rust_i18n::t!("detail.extracting", ctx = ctx)
                );
                let _ = std::io::stdout().flush();
            }
            Event::PackageExtractDone => {
                self.extracting = None;
            }

            // --- Commit (install/update/uninstall) ---
            Event::PackageCommitStart(_) => {}
            Event::PackageCommitDone(_) => {}

            // --- Shim operations ---
            Event::PackageShimRemoveStart => {}
            Event::PackageShimRemoveProgress(ctx) => {
                tracing::debug!("{}", rust_i18n::t!("detail.removing_shim", ctx = ctx));
            }
            Event::PackageShimRemoveDone => {
                tracing::debug!("{}", rust_i18n::t!("detail.shim_removed"));
            }

            Event::PackageShimAddStart(_) => {}
            Event::PackageShimAddProgress(ctx) => {
                tracing::debug!("{}", rust_i18n::t!("detail.creating_shim", ctx = ctx));
            }
            Event::PackageShimAddDone => {
                tracing::debug!("{}", rust_i18n::t!("detail.shim_done"));
            }

            // --- Shim warnings ---
            Event::PackageShimConflict(path) => {
                output::warn(rust_i18n::t!("detail.shim_conflict", path = path));
            }

            // --- Shortcut operations ---
            Event::PackageShortcutRemoveStart => {}
            Event::PackageShortcutRemoveProgress(ctx) => {
                tracing::debug!("{}", rust_i18n::t!("detail.removing_shortcut", ctx = ctx));
            }
            Event::PackageShortcutRemoveDone => {
                tracing::debug!("{}", rust_i18n::t!("detail.shortcut_removed"));
            }

            Event::PackageShortcutAddStart => {}
            Event::PackageShortcutAddProgress(ctx) => {
                tracing::debug!("{}", rust_i18n::t!("detail.creating_shortcut", ctx = ctx));
            }
            Event::PackageShortcutAddDone => {
                tracing::debug!("{}", rust_i18n::t!("detail.shortcut_done"));
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
            Event::PackageEnvPathAddStart => {
                tracing::debug!("{}", rust_i18n::t!("detail.adding_env_path"));
            }
            Event::PackageEnvPathAddDone => {
                tracing::debug!("{}", rust_i18n::t!("detail.env_path_added"));
            }
            Event::PackageEnvPathRemoveStart => {
                tracing::debug!("{}", rust_i18n::t!("detail.removing_env_path"));
            }
            Event::PackageEnvPathRemoveDone => {
                tracing::debug!("{}", rust_i18n::t!("detail.env_path_removed"));
            }

            Event::PackageEnvVarSetStart => {
                tracing::debug!("{}", rust_i18n::t!("detail.setting_env_var"));
            }
            Event::PackageEnvVarSetDone => {
                tracing::debug!("{}", rust_i18n::t!("detail.env_var_set"));
            }
            Event::PackageEnvVarRemoveStart => {
                tracing::debug!("{}", rust_i18n::t!("detail.removing_env_var"));
            }
            Event::PackageEnvVarRemoveDone => {
                tracing::debug!("{}", rust_i18n::t!("detail.env_var_removed"));
            }

            // --- Persist ---
            Event::PackagePersistPurgeStart => {
                tracing::debug!("{}", rust_i18n::t!("detail.removing_persist"));
            }
            Event::PackagePersistPurgeDone => {
                tracing::debug!("{}", rust_i18n::t!("detail.persist_done"));
            }

            // --- PS module ---
            Event::PackagePsModuleAddStart(ctx) => {
                tracing::debug!("{}", rust_i18n::t!("detail.installing_psmodule", ctx = ctx));
            }
            Event::PackagePsModuleAddDone => {
                tracing::debug!("{}", rust_i18n::t!("detail.psmodule_installed"));
            }
            Event::PackagePsModuleRemoveStart(ctx) => {
                tracing::debug!("{}", rust_i18n::t!("detail.removing_psmodule", ctx = ctx));
            }
            Event::PackagePsModuleRemoveDone => {
                tracing::debug!("{}", rust_i18n::t!("detail.psmodule_removed"));
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
            Event::PackageVersionKnown {
                name,
                old_version,
                new_version,
            } => {
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
                tracing::debug!("{}", rust_i18n::t!("detail.unlinking", path = path));
            }
            Event::PackageSymlinkCreate { from, to } => {
                tracing::debug!("{}", rust_i18n::t!("detail.linking", from = from, to = to));
            }

            // --- Held package skipped ---
            Event::PackageHeld { name, version } => {
                output::warn(rust_i18n::t!(
                    "cmd.held_skip",
                    name = name,
                    version = version
                ));
            }

            // --- Sync done ---
            Event::PackageSyncDone => {}

            // --- Interactive prompts (handled by event loop, not here) ---
            Event::PromptPackageCandidate(_) | Event::PromptTransactionNeedConfirm { .. } => {}

            // --- Download progress (handled by event loop, not here) ---
            Event::PackageDownloadProgress(_) => {}

            // --- Future events ---
            _ => {}
        }
    }
}

impl ScoopHandler {
    pub fn new() -> Self {
        Self { extracting: None }
    }
}
