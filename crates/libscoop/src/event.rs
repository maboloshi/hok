//! Event types and the event bus for session-backend communication.
//!
//! Defines the [`Event`] enum — all possible events that can be emitted
//! during Scoop operations — and the [`EventBus`] that carries them.
//!
//! # Design
//!
//! - **Full-duplex channel**: The event bus wraps two `flume` channels:
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

use flume::{bounded, Receiver, Sender};

use crate::{
    bucket::BucketUpdateProgressContext,
    constant::EVENT_BUS_CAPACITY,
    package::{download::PackageDownloadProgressContext, sync::Transaction},
};

/// Event bus for event transmission.
#[derive(Debug)]
pub struct EventBus {
    // Outbound channel, used to send events out from the session
    inner_tx: Sender<Event>,
    outer_rx: Receiver<Event>,

    // Inbound channel, used to receive events from outside
    outer_tx: Sender<Event>,
    inner_rx: Receiver<Event>,
}

impl EventBus {
    /// Create a new event bus.
    pub fn new() -> EventBus {
        let (inner_tx, outer_rx) = bounded(EVENT_BUS_CAPACITY);
        let (outer_tx, inner_rx) = bounded(EVENT_BUS_CAPACITY);
        Self {
            inner_tx,
            outer_rx,
            outer_tx,
            inner_rx,
        }
    }

    /// Get the sender of the event bus.
    ///
    /// This sender is used to send events into the session.
    pub fn sender(&self) -> Sender<Event> {
        self.outer_tx.clone()
    }

    /// Get the receiver of the event bus.
    ///
    /// This receiver is used to receive events from the session.
    pub fn receiver(&self) -> Receiver<Event> {
        self.outer_rx.clone()
    }

    /// Get the outbound sender of the event bus.
    pub(crate) fn inner_sender(&self) -> Sender<Event> {
        self.inner_tx.clone()
    }

    /// Get the inbound receiver of the event bus.
    pub(crate) fn inner_receiver(&self) -> &Receiver<Event> {
        &self.inner_rx
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

    /// Package environment path(s) removal has started.
    PackageEnvPathRemoveStart,

    /// Package environment path(s) removal has finished.
    PackageEnvPathRemoveDone,

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
