//! Default event handler for package sync operations.
//!
//! Renders coarse-grained events to stdout (phase headers, package notes,
//! warnings, version info) using the single [`crate::output`] renderer.
//! Granular sub-step details (shim/shortcut/env/persist/ps-module operations)
//! are routed to the `tracing` log channel instead — visible only with
//! `-v` / `-vv` / `-vvv` — keeping stdout minimal and constant.

use libscoop::{Event, EventHandler, PackageStepKind};

use crate::output;

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
                output::status(hok_i18n::t!("status.resolving"));
            }
            Event::PackageResolveDone => {}

            // --- Download sizing ---
            Event::PackageDownloadSizingStart => {
                output::status(hok_i18n::t!("status.sizing"));
            }
            Event::PackageDownloadSizingDone => {}

            // --- Download ---
            Event::PackageDownloadStart => {
                output::status(hok_i18n::t!("status.downloading"));
            }
            Event::PackageDownloadDone => {}

            // --- Integrity check ---
            Event::PackageIntegrityCheckStart => {
                output::status(hok_i18n::t!("status.checking_hash"));
            }
            Event::PackageIntegrityCheckProgress(ctx) => {
                output::status(hok_i18n::t!("detail.checking_hash_item", ctx = ctx));
            }
            Event::PackageIntegrityCheckDone => {}

            // --- Extraction ---
            // Progress lines are redrawn in place with `\r`; `output::status_in_place`
            // clears the whole line first (`\x1b[2K`) so a shorter phrase doesn't
            // leave tail characters of the previous (longer) line behind, and
            // styles the phrase like every other verb-status line (dim) so the
            // row reads as a peer of "resolving/downloading/checking hash".
            // Progress carries a bare `n/total` counter (no filename), prefixed
            // with the package name from the start event.
            Event::PackageExtractStart(ctx) => {
                self.extracting = Some(ctx.clone());
                output::status_in_place(hok_i18n::t!("detail.extracting", ctx = ctx));
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
                output::status_in_place(hok_i18n::t!("detail.extracting", ctx = ctx));
            }
            Event::PackageExtractDone => {
                // The extract progress line is redrawn with `\r` and never
                // carries a trailing newline; terminate it here so the next
                // output (closure or an interleaved tracing line under -vvv)
                // starts on a fresh row instead of gluing onto the counter.
                if self.extracting.take().is_some() {
                    println!();
                }
            }

            // --- Commit (install/update/uninstall) ---
            // Per-package counting + closure: `PackageCommitStart` anchors the
            // counter header (carrying the version change), and
            // `PackageCommitDone` / `PackageCommitFailed` close it with a bare
            // `✓ pkg` / `✗ pkg (err)` line. Intermediate sub-steps are routed
            // to the tracing log channel (see the rest of this handler),
            // keeping stdout minimal and constant.
            Event::PackageCommitStart {
                name,
                index,
                total,
                old_version,
                new_version,
            } => output::commit_header(*index, *total, name, old_version, new_version),
            Event::PackageCommitDone { name, .. } => output::closure_done(name),
            Event::PackageCommitFailed { name, error, .. } => output::closure_fail(name, error),

            // --- Shim operations ---
            Event::PackageShimRemoveStart => {}
            Event::PackageShimRemoveProgress(ctx) => {
                tracing::debug!("{}", hok_i18n::t!("detail.removing_shim", ctx = ctx));
            }
            Event::PackageShimRemoveDone => {
                tracing::debug!("{}", hok_i18n::t!("detail.shim_removed"));
            }

            Event::PackageShimAddStart(_) => {}
            Event::PackageShimAddProgress(ctx) => {
                tracing::debug!("{}", hok_i18n::t!("detail.creating_shim", ctx = ctx));
            }
            Event::PackageShimAddDone => {
                tracing::debug!("{}", hok_i18n::t!("detail.shim_done"));
            }

            // --- Shim warnings ---
            Event::PackageShimConflict(path) => {
                output::warn(hok_i18n::t!("detail.shim_conflict", path = path));
            }

            // --- Shortcut operations ---
            Event::PackageShortcutRemoveStart => {}
            Event::PackageShortcutRemoveProgress(ctx) => {
                tracing::debug!("{}", hok_i18n::t!("detail.removing_shortcut", ctx = ctx));
            }
            Event::PackageShortcutRemoveDone => {
                tracing::debug!("{}", hok_i18n::t!("detail.shortcut_removed"));
            }

            Event::PackageShortcutAddStart => {}
            Event::PackageShortcutAddProgress(ctx) => {
                tracing::debug!("{}", hok_i18n::t!("detail.creating_shortcut", ctx = ctx));
            }
            Event::PackageShortcutAddDone => {
                tracing::debug!("{}", hok_i18n::t!("detail.shortcut_done"));
            }

            // --- Shortcut warnings ---
            Event::PackageShortcutConflict(path) => {
                output::warn(hok_i18n::t!("detail.shortcut_conflict", path = path));
            }
            Event::PackageShortcutNotFound(path) => {
                output::warn(hok_i18n::t!("detail.shortcut_not_found", path = path));
            }

            // --- Session warnings ---
            Event::ConfigLoadFallback => {
                output::warn(hok_i18n::t!("detail.config_fallback"));
            }

            // --- Environment operations ---
            Event::PackageEnvPathAddStart => {
                tracing::debug!("{}", hok_i18n::t!("detail.adding_env_path"));
            }
            Event::PackageEnvPathAddDone => {
                tracing::debug!("{}", hok_i18n::t!("detail.env_path_added"));
            }
            Event::PackageEnvPathRemoveStart => {
                tracing::debug!("{}", hok_i18n::t!("detail.removing_env_path"));
            }
            Event::PackageEnvPathRemoveDone => {
                tracing::debug!("{}", hok_i18n::t!("detail.env_path_removed"));
            }

            Event::PackageEnvVarSetStart => {
                tracing::debug!("{}", hok_i18n::t!("detail.setting_env_var"));
            }
            Event::PackageEnvVarSetDone => {
                tracing::debug!("{}", hok_i18n::t!("detail.env_var_set"));
            }
            Event::PackageEnvVarRemoveStart => {
                tracing::debug!("{}", hok_i18n::t!("detail.removing_env_var"));
            }
            Event::PackageEnvVarRemoveDone => {
                tracing::debug!("{}", hok_i18n::t!("detail.env_var_removed"));
            }

            // --- Persist ---
            Event::PackagePersistPurgeStart => {
                tracing::debug!("{}", hok_i18n::t!("detail.removing_persist"));
            }
            Event::PackagePersistPurgeDone => {
                tracing::debug!("{}", hok_i18n::t!("detail.persist_done"));
            }

            // --- PS module ---
            Event::PackagePsModuleAddStart(ctx) => {
                tracing::debug!("{}", hok_i18n::t!("detail.installing_psmodule", ctx = ctx));
            }
            Event::PackagePsModuleAddDone => {
                tracing::debug!("{}", hok_i18n::t!("detail.psmodule_installed"));
            }
            Event::PackagePsModuleRemoveStart(ctx) => {
                tracing::debug!("{}", hok_i18n::t!("detail.removing_psmodule", ctx = ctx));
            }
            Event::PackagePsModuleRemoveDone => {
                tracing::debug!("{}", hok_i18n::t!("detail.psmodule_removed"));
            }

            // --- PowerShell script output (split by stream) ---
            // Stdout/warnings/stderr render with the same two-space indent;
            // each is coloured by its stream via the convention helper unless
            // the line carries its own script-embedded ANSI.
            Event::ScriptStdout(line) => {
                println!(
                    "  {}",
                    output::script_line(output::ScriptStream::Info, line)
                );
            }
            Event::ScriptWarning(line) => {
                println!(
                    "  {}",
                    output::script_line(output::ScriptStream::Warning, line)
                );
            }
            Event::ScriptStderr(line) => {
                println!(
                    "  {}",
                    output::script_line(output::ScriptStream::Error, line)
                );
            }
            Event::ScriptDone { success, stderr } => {
                if !success && !stderr.is_empty() {
                    eprintln!("  {}", hok_i18n::t!("cmd.script_failed", stderr = stderr));
                }
            }

            // --- Verbose lifecycle steps (-v) ---
            // Each PackageStep names a Scoop-ordered stage the current
            // commit just passed. Rendering is gated inside `output::step`
            // on the global show_steps toggle, so at the default verbosity
            // these lines never reach stdout.
            Event::PackageStep {
                name: _,
                kind,
                object,
            } => self.render_step(*kind, object.as_deref()),

            // --- Post-install notes ---
            Event::PackageNotes(note) => {
                output::info(note);
            }

            // --- Cache hit ---
            Event::PackageCacheHit(filename) => {
                output::status(hok_i18n::t!("detail.loading_cache", filename = filename));
            }

            // --- Symlink operations ---
            Event::PackageSymlinkRemove(path) => {
                tracing::debug!("{}", hok_i18n::t!("detail.unlinking", path = path));
            }
            Event::PackageSymlinkCreate { from, to } => {
                tracing::debug!("{}", hok_i18n::t!("detail.linking", from = from, to = to));
            }

            // --- Held package skipped ---
            Event::PackageHeld { name, version } => {
                output::warn(hok_i18n::t!(
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

    /// Render a single verbose lifecycle step as a localized "verb + object"
    /// line. The localized sentence is built with the object substituted (so
    /// both languages can place it anywhere); `output::step` then colourises
    /// the object in the default foreground and greys the rest. Phrase-only
    /// kinds carry no object and render fully grey.
    fn render_step(&self, kind: PackageStepKind, object: Option<&str>) {
        let o = object.unwrap_or("");
        let (sentence, highlight): (String, bool) = match kind {
            PackageStepKind::Extract => (hok_i18n::t!("step.extract", object = o), true),
            PackageStepKind::RunHookScript => (hok_i18n::t!("step.hook_script", object = o), true),
            PackageStepKind::RunInstallerFile => (hok_i18n::t!("step.installer", object = o), true),
            PackageStepKind::LinkCurrent => (hok_i18n::t!("step.link_current"), false),
            PackageStepKind::UnlinkCurrent => (hok_i18n::t!("step.unlink_current"), false),
            PackageStepKind::RefreshShim => (hok_i18n::t!("step.refresh_shim", object = o), true),
            PackageStepKind::RefreshShortcut => {
                (hok_i18n::t!("step.refresh_shortcut", object = o), true)
            }
            PackageStepKind::CreateShim => (hok_i18n::t!("step.create_shim", object = o), true),
            PackageStepKind::RemoveShim => (hok_i18n::t!("step.remove_shim", object = o), true),
            PackageStepKind::CreateShortcut => {
                (hok_i18n::t!("step.create_shortcut", object = o), true)
            }
            PackageStepKind::RemoveShortcut => {
                (hok_i18n::t!("step.remove_shortcut", object = o), true)
            }
            PackageStepKind::AddPsModule => (hok_i18n::t!("step.add_psmodule", object = o), true),
            PackageStepKind::RemovePsModule => {
                (hok_i18n::t!("step.remove_psmodule", object = o), true)
            }
            PackageStepKind::AddEnvPath => (hok_i18n::t!("step.add_env_path", object = o), true),
            PackageStepKind::RemoveEnvPath => {
                (hok_i18n::t!("step.remove_env_path", object = o), true)
            }
            PackageStepKind::SetEnvVar => (hok_i18n::t!("step.set_env_var", object = o), true),
            PackageStepKind::RemoveEnvVar => {
                (hok_i18n::t!("step.remove_env_var", object = o), true)
            }
            PackageStepKind::PersistLink => (hok_i18n::t!("step.persist_link", object = o), true),
            PackageStepKind::PersistUnlink => {
                (hok_i18n::t!("step.persist_unlink", object = o), true)
            }
            PackageStepKind::RemoveDir => (hok_i18n::t!("step.remove_dir", object = o), true),
            PackageStepKind::PersistPurge => (hok_i18n::t!("step.persist_purge"), false),
            _ => return, // non_exhaustive: ignore future kinds until rendered
        };
        let obj = if highlight && !o.is_empty() {
            Some(o.to_owned())
        } else {
            None
        };
        crate::output::step(sentence, obj);
    }
}
