//! Event types and the event bus for session-backend communication.
//!
//! Defines the [`Event`] enum — all possible events that can be emitted
//! during Scoop operations — and the [`EventBus`] that carries them.
//!
//! # Design
//!
//! - **Full-duplex channel**: The event bus wraps two `tokio::sync::mpsc`
//!   channels:
//!   - *Outbound* (`inner_tx` → `outer_rx`): Session backend → frontend.
//!     Events flow *out* of the session to the caller (e.g. progress, prompts).
//!   - *Inbound* (`outer_tx` → `inner_rx`): Frontend → session backend.
//!     Responses flow *in* (e.g. `PromptTransactionNeedConfirmResult`).
//!     The public API exposes only `sender()` and `receiver()` on the outer
//!     (frontend-facing) side; the inner side is `pub(crate)`.
//!
//! - **Non-exhaustive enum**: `Event` is marked `#[non_exhaustive]` so that
//!   adding new variants is not a breaking change for external implementors
//!   of [`EventHandler`].
//!
//! - **Naming convention**: Every event pair follows `Action + Start / Done`,
//!   with optional `Progress` in between (e.g. `PackageDownloadStart`,
//!   `PackageDownloadProgress`, `PackageDownloadDone`). Prompt events use
//!   a paired `PromptXxx` + `PromptXxxResult` pattern.
//!
//! # Extending
//!
//! To add a new event:
//! 1. Add the variant(s) to the [`Event`] enum.
//! 2. Emit it from the relevant operation via `session.emitter()`.
//! 3. Handle it in [`eventloop::run_event_loop`] and/or in
//!    [`EventHandler::handle`].
//!
//! If the event needs a response from the frontend, also add a `Result`
//! variant and use the inbound channel (`EventBus::sender()`) to send it
//! back.

use tokio::sync::mpsc;

use crate::{
    bucket::BucketUpdateProgressContext,
    constant::EVENT_BUS_CAPACITY,
    package::{download::PackageDownloadProgressContext, sync::Transaction},
};

/// Event bus for event transmission.
#[derive(Debug)]
pub struct EventBus {
    // Outbound channel, used to send events out from the session. Wrapped in a
    // Mutex so the outbound channel can be recreated when a new event loop
    // takes the receiver (reinstall runs two sync transactions back-to-back
    // in one session; the tokio::sync::mpsc receiver is single-consumer).
    inner_tx: std::sync::Mutex<mpsc::Sender<Event>>,
    // The outbound receiver is a single-consumer tokio::sync::mpsc
    // receiver (a single-consumer channel), so it is stored in a
    // Mutex<Option<_>> and handed out exactly once — to the event loop task
    // (or a test that drains it) — via EventBus::receiver().
    outer_rx: std::sync::Mutex<Option<mpsc::Receiver<Event>>>,

    // Inbound channel, used to receive events from outside
    outer_tx: mpsc::Sender<Event>,
    // The inbound receiver is consumed sequentially by the backend while it
    // awaits prompt responses (confirm_transaction, select_candidate), so it
    // lives in a tokio::sync::Mutex and is awaited in place.
    inner_rx: tokio::sync::Mutex<Option<mpsc::Receiver<Event>>>,
}

impl EventBus {
    /// Create a new event bus.
    pub fn new() -> EventBus {
        let (inner_tx, outer_rx) = mpsc::channel(EVENT_BUS_CAPACITY);
        let (outer_tx, inner_rx) = mpsc::channel(EVENT_BUS_CAPACITY);
        Self {
            inner_tx: std::sync::Mutex::new(inner_tx),
            outer_rx: std::sync::Mutex::new(Some(outer_rx)),
            outer_tx,
            inner_rx: tokio::sync::Mutex::new(Some(inner_rx)),
        }
    }

    /// Get the sender of the event bus.
    ///
    /// This sender is used to send events into the session.
    pub fn sender(&self) -> mpsc::Sender<Event> {
        self.outer_tx.clone()
    }

    /// Get the receiver of the event bus.
    ///
    /// This receiver is used to receive events from the session. Because a
    /// tokio::sync::mpsc receiver is single-consumer, it can only be obtained
    /// once per outbound channel — the event loop (or a test) is its sole owner.
    ///
    /// If the receiver was already taken, a fresh outbound channel is created
    /// and returned so a new event loop can run (e.g. reinstall runs a remove
    /// and an install transaction back-to-back in one session).
    pub fn receiver(&self) -> Option<mpsc::Receiver<Event>> {
        let mut guard = self.outer_rx.lock().unwrap();
        if let Some(rx) = guard.take() {
            return Some(rx);
        }
        // Already taken: recreate the outbound channel for the next phase
        // (reinstall runs remove + install in one session; the previous
        // event loop has finished by the time the next phase runs).
        let (inner_tx, outer_rx) = mpsc::channel(EVENT_BUS_CAPACITY);
        *self.inner_tx.lock().unwrap() = inner_tx;
        *guard = Some(outer_rx);
        guard.take()
    }

    /// Get the outbound sender of the event bus.
    pub(crate) fn inner_sender(&self) -> mpsc::Sender<Event> {
        self.inner_tx.lock().unwrap().clone()
    }

    /// Await the next inbound (frontend → backend) event.
    ///
    /// Used by the backend to wait for a response to a prompt it emitted on
    /// the outbound channel. Returns None when the channel is closed.
    pub(crate) async fn receive_response(&self) -> Option<Event> {
        let mut guard = self.inner_rx.lock().await;
        match guard.as_mut() {
            Some(rx) => rx.recv().await,
            None => None,
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Event that may be emitted during the execution of operations.
#[derive(Clone)]
#[non_exhaustive]
pub enum Event {
    /// Bucket update has made some progress.
    BucketUpdateProgress(BucketUpdateProgressContext),

    /// Bucket update has finished.
    BucketUpdateDone,

    /// Package has started to be committed.
    PackageCommitStart(String),

    /// Package has been committed.
    PackageCommitDone(String),

    /// Calculating download size has started.
    PackageDownloadSizingStart,

    /// Calculating download size has finished.
    PackageDownloadSizingDone,

    /// Package download has started.
    PackageDownloadStart,

    /// Package download has made some progress.
    PackageDownloadProgress(PackageDownloadProgressContext),

    /// Package download has finished.
    PackageDownloadDone,

    /// Package extraction has started.
    PackageExtractStart(String),

    /// Package extraction has made some progress.
    PackageExtractProgress(String),

    /// Package extraction has finished.
    PackageExtractDone,

    /// Package shim creation has started.
    PackageShimAddStart(String),

    /// Package shim creation has made some progress.
    PackageShimAddProgress(String),

    /// Package shim creation has finished.
    PackageShimAddDone,

    /// Package shim already exists and belongs to another package; it will
    /// be overwritten.
    PackageShimConflict(String),

    /// Package shortcut creation has started.
    PackageShortcutAddStart,

    /// Package shortcut creation has made some progress.
    PackageShortcutAddProgress(String),

    /// Package shortcut already exists and belongs to another package; it will
    /// be overwritten.
    PackageShortcutConflict(String),

    /// Package shortcut creation has finished.
    PackageShortcutAddDone,

    /// Package environment path(s) addition has started.
    PackageEnvPathAddStart,

    /// Package environment path(s) addition has finished.
    PackageEnvPathAddDone,

    /// Package environment path(s) removal has started.
    PackageEnvPathRemoveStart,

    /// Package environment path(s) removal has finished.
    PackageEnvPathRemoveDone,

    /// Package environment variable(s) setting has started.
    PackageEnvVarSetStart,

    /// Package environment variable(s) setting has finished.
    PackageEnvVarSetDone,

    /// Package environment variable(s) removal has started.
    PackageEnvVarRemoveStart,

    /// Package environment variable(s) removal has finished.
    PackageEnvVarRemoveDone,

    /// Package integrity check has started.
    PackageIntegrityCheckStart,

    /// Package integrity check has made some progress.
    PackageIntegrityCheckProgress(String),

    /// Package integrity check has finished.
    PackageIntegrityCheckDone,

    /// Package persist removal has started.
    PackagePersistPurgeStart,

    /// Package persist removal has finished.
    PackagePersistPurgeDone,

    /// Package PowerShell module installation has started.
    PackagePsModuleAddStart(String),

    /// Package PowerShell module installation has finished.
    PackagePsModuleAddDone,

    /// Package PowerShell module removal has started.
    PackagePsModuleRemoveStart(String),

    /// Package PowerShell module removal has finished.
    PackagePsModuleRemoveDone,

    /// Package resolving has started.
    PackageResolveStart,

    /// Package resolving has finished.
    PackageResolveDone,

    /// Package shim removal has started.
    PackageShimRemoveStart,

    /// Package shim removal has made some progress.
    PackageShimRemoveProgress(String),

    /// Package shim removal has finished.
    PackageShimRemoveDone,

    /// Package shortcut removal has started.
    PackageShortcutRemoveStart,

    /// Package shortcut removal has made some progress.
    PackageShortcutRemoveProgress(String),

    /// Package shortcut was not found during removal.
    PackageShortcutNotFound(String),

    /// Package shortcut removal has finished.
    PackageShortcutRemoveDone,

    /// All config paths failed, falling back to default config.
    ConfigLoadFallback,

    /// Package sync operation has finished.
    PackageSyncDone,

    /// A standalone download operation (`download_apps`) has finished.
    DownloadDone,

    /// PowerShell script emitted output (stdout line).
    ScriptOutput(String),

    /// Version information for a resolved package (old -> new).
    PackageVersionKnown {
        /// Package name
        name: String,
        /// Currently installed version, or empty if new install
        old_version: String,
        /// Version to be installed
        new_version: String,
    },

    /// A download was satisfied from local cache.
    PackageCacheHit(String),

    /// A symlink/junction was removed.
    PackageSymlinkRemove(String),

    /// A symlink/junction was created.
    PackageSymlinkCreate {
        /// Source path
        from: String,
        /// Target path
        to: String,
    },

    /// PowerShell script execution finished.
    ScriptDone {
        /// Whether the script exited successfully.
        success: bool,
        /// Captured stderr output.
        stderr: String,
    },

    /// Post-install notes from the manifest.
    PackageNotes(String),

    /// A package was skipped during upgrade because it is held.
    PackageHeld {
        /// Package name
        name: String,
        /// Current held version
        version: String,
    },

    /// Prompt the user to confirm the transaction.
    PromptTransactionNeedConfirm(Transaction),

    /// Result of [`PromptTransactionNeedConfirm`][1].
    ///
    /// [1]: Event::PromptTransactionNeedConfirm
    PromptTransactionNeedConfirmResult(bool),

    /// Prompt the user to select a package from multiple candidates.
    PromptPackageCandidate(Vec<String>),

    /// Result of [`PromptPackageCandidate`][1].
    ///
    /// [1]: Event::PromptPackageCandidate
    PromptPackageCandidateResult(usize),
}
