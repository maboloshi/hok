//! Transaction engine for install / upgrade / uninstall / reset.
//!
//! This module owns the transaction model ([`Transaction`], [`SyncOption`]),
//! the size pre-check and confirmation prompt, and the public entry points
//! ([`install`], [`remove`], [`reset`]). The actual install and remove
//! pipelines live in the private sub-modules [`sync_install`] and
//! [`sync_remove`]; `sync.rs` itself is the orchestration shell, not the
//! pipeline.
//!
//! # Design
//!
//! - **Event-driven pipeline**: Every sub-step (download, extract, shim,
//!   etc.) emits start/progress/done events. The frontend (event loop)
//!   receives these to update UI and send back responses (confirm,
//!   select candidate).
//! - **Transaction model**: A [`Transaction`] is built upfront containing
//!   the full list of packages to install/upgrade/remove, along with
//!   download sizes and version info. The user confirms the transaction
//!   before any destructive action.
//! - **SyncOption flags**: [`SyncOption`] controls behaviour: `AssumeYes`
//!   skips prompts, `DownloadOnly` stops after caching, `SkipHashCheck`
//!   bypasses integrity verification, etc.
//! - **Lifecycle phases**: Each operation (install, upgrade, uninstall)
//!   follows a sequence: resolve → download → check → extract → config
//!   → shim → shortcut → persist → cleanup. Some phases are skipped
//!   depending on the operation type.
//! - **Concurrent downloads**: Multiple package downloads run concurrently
//!   via the `download` module, with progress aggregated per-package.

use std::cell::OnceCell;

use crate::{Error, Event, Session, error::Fallible};

use super::{Package, download::DownloadSize};

#[path = "sync_install.rs"]
mod sync_install;
#[path = "sync_remove.rs"]
mod sync_remove;

pub use sync_install::install;
pub use sync_install::{RunningAction, RunningCheck, check_not_running};
pub use sync_remove::{remove, reset};

/// Options that may be used to tweak behavior of package sync operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum SyncOption {
    /// Assume YES on all prompts.
    ///
    /// # Note
    ///
    /// This option will also suppress the prompt for package candidate selection.
    /// A built-in candidate selection algorithm will be used to select the
    /// proper candidate. This may not be the desired behavior in some cases.
    ///
    /// Enabling this option will also suppress the calculation of download size.
    AssumeYes,

    /// Download package only.
    ///
    /// # Note
    ///
    /// To sync packages by just downloading and caching them without installing
    /// or upgrading, this option can be used. Transcation will be stopped after
    /// the download is done.
    DownloadOnly,

    /// Ignore local cache and force package download.
    ///
    /// # Note
    ///
    /// This option is not intended to be used with the [`Offline`][1]
    /// option.
    ///
    /// [1]: SyncOption::Offline
    NoCache,

    /// Ignore transaction failure.
    ///
    /// The sync operation processes packages in the transaction one by one
    /// according to the dependency order. By default, the transaction will be
    /// aborted if any failure occurs during the operation.
    ///
    /// # Note
    ///
    /// This option can be used to ignore the failure and continue the operation
    /// to commit the remaining packages in the transaction.
    ///
    /// When a failure occurs, the operation will be stopped immediately and
    /// a rollback will be performed on the exact package causing the failure
    /// while successfully committed packages will be kept be as they are. The
    /// rest of the unpocessed packages will be skipped, and the error will be
    /// returned.
    ///
    /// **NO rollback will be performed if this option is enabled**, which means
    /// there may be broken packages being committed to the system.
    IgnoreFailures,

    /// Do not install dependencies.
    ///
    /// # Note
    ///
    /// By default, dependencies of the pending installation package will be
    /// resolved and installed **recursively** if they are not installed yet.
    /// One can opt in this option to disable the default behavior. However,
    /// it is not recommended to do so since it clearly breaks the dependency
    /// relationship, and may stop the dependents from working properly.
    NoDependencies,

    /// Stop checking hash of downloaded packages.
    ///
    /// # Note
    ///
    /// Integrity check helps to ensure the downloaded packages are not corrupted
    /// or tampered. Hash check will be performed by default. In some cases, user
    /// may want to skip the check to force the installation or upgrade of the
    /// packages. By opting in this option, the hash check will be skipped.
    ///
    /// It is highly **NOT** recommended to use this option unless you really
    /// know what you are doing.
    SkipHashCheck,

    /// Do not upgrade packages.
    ///
    /// This option is not intended to be used with the [`OnlyUpgrade`][1] option.
    ///
    /// [1]: SyncOption::OnlyUpgrade
    NoUpgrade,

    /// Do not replace packages.
    ///
    /// # Note
    ///
    /// When a package is installed and a same-named package is proposed to be
    /// installed, a replace operation will be performed if the proposed package
    /// is from a different bucket from the installed one.
    ///
    /// By opting in this option, the replace operation will be suppressed.
    NoReplace,

    /// Offline mode.
    ///
    /// # Note
    ///
    /// This option is useful when user wants to install or upgrade packages
    /// with existing local cached packages. By opting in this option and having
    /// valid caches prepared, network access can be avoided to perform the sync
    /// operation. However, the transaction may fail if there is any package file
    /// missing or invalid cache.
    ///
    /// This option is basically the opposite of the [`NoCache`][1] option.
    ///
    /// [1]: SyncOption::NoCache
    Offline,

    /// Upgrade packages only.
    ///
    /// Use this option to specify a sync operation of only upgrading packages.
    ///
    /// This option is not intended to be used with the [`NoUpgrade`][1] option.
    ///
    /// [1]: SyncOption::NoUpgrade
    OnlyUpgrade,

    /// Force reinstall of already-installed packages (used with
    /// [`OnlyUpgrade`][1]): the update is restricted to installed packages
    /// but skips the up-to-date filter, so even packages at their current
    /// version are reinstalled — matching `scoop update --force`.
    ///
    /// [1]: SyncOption::OnlyUpgrade
    Force,

    /// Uninstall packages.
    ///
    /// Use this option to specify a sync operation of only uninstalling packages.
    Remove,

    /// Purge uninstall.
    ///
    /// # Note
    ///
    /// By enabling this option, persistent data of the pending removal packages
    /// will be removed simultaneously.
    ///
    /// This option only takes effect with the [`Remove`][1] option.
    ///
    /// [1]: SyncOption::Remove
    Purge,

    /// Cascade uninstall.
    ///
    /// # Note
    ///
    /// By opt in this option, dependencies of the pending removal package
    /// will also be removed **recursively** if they are not required by other
    /// installed packages.
    ///
    /// This option only takes effect with the [`Remove`][1] option.
    ///
    /// [1]: SyncOption::Remove
    Cascade,

    /// Disable dependent check.
    ///
    /// # Note
    ///
    /// By default, a reverse dependencies check will be performed on the pending
    /// removal package. If any installed package depends on the pending removal
    /// package, the removal operation will be aborted.
    ///
    /// The default behavior can be modified by opting in this option, however,
    /// it is not recommended to do so since it clearly breaks the dependency
    /// relationship, and may stop the dependents from working properly.
    ///
    /// This option only takes effect with the [`Remove`][1] option.
    ///
    /// [1]: SyncOption::Remove
    NoDependentCheck,
}

/// Transaction of sync operation.
///
/// # Note
///
/// A transaction is a set of packages that will be installed, upgraded, replaced
/// or removed. The transaction is calculated by the sync operation and can be
/// used to prompt the user to confirm the operation.
#[derive(Clone)]
pub struct Transaction {
    /// Packages that will be installed with the transaction.
    install: OnceCell<Vec<Package>>,

    /// Packages that will be upgraded with the transaction.
    upgrade: OnceCell<Vec<Package>>,

    /// Packages that will be replaced with the transaction.
    replace: OnceCell<Vec<Package>>,

    /// Packages that will be removed with the transaction.
    remove: OnceCell<Vec<Package>>,

    /// Total download size of the transaction.
    download_size: OnceCell<DownloadSize>,
}

impl Transaction {
    fn new() -> Transaction {
        Transaction {
            install: OnceCell::new(),
            upgrade: OnceCell::new(),
            replace: OnceCell::new(),
            remove: OnceCell::new(),
            download_size: OnceCell::new(),
        }
    }

    fn set_install(&self, packages: Vec<Package>) {
        let _ = self.install.set(packages);
    }

    fn set_upgrade(&self, packages: Vec<Package>) {
        let _ = self.upgrade.set(packages);
    }

    fn set_replace(&self, packages: Vec<Package>) {
        let _ = self.replace.set(packages);
    }

    fn set_remove(&self, packages: Vec<Package>) {
        let _ = self.remove.set(packages);
    }

    fn set_download_size(&self, download_size: DownloadSize) -> bool {
        self.download_size.set(download_size).is_ok()
    }

    fn add_view(&self) -> Vec<&Package> {
        self.install_view()
            .into_iter()
            .chain(self.upgrade_view())
            .chain(self.replace_view())
            .flatten()
            .collect::<Vec<_>>()
    }

    /// Get packages that will be installed with the transaction.
    ///
    /// # Returns
    ///
    /// A reference to the vector of packages that will be installed or `None`
    /// if no packages will be installed.
    pub fn install_view(&self) -> Option<&Vec<Package>> {
        self.install.get()
    }

    /// Get packages that will be upgraded with the transaction.
    ///
    /// # Returns
    ///
    /// A reference to the vector of packages that will be upgraded or `None`
    /// if no packages will be upgraded.
    pub fn upgrade_view(&self) -> Option<&Vec<Package>> {
        self.upgrade.get()
    }

    /// Get packages that will be replaced with the transaction.
    ///
    /// # Returns
    ///
    /// A reference to the vector of packages that will be replaced or `None`
    /// if no packages will be replaced.
    pub fn replace_view(&self) -> Option<&Vec<Package>> {
        self.replace.get()
    }

    /// Get packages that will be removed with the transaction.
    ///
    /// # Returns
    ///
    /// A reference to the vector of packages that will be removed or `None`
    /// if no packages will be removed.
    pub fn remove_view(&self) -> Option<&Vec<Package>> {
        self.remove.get()
    }

    /// Get the total download size of the transaction.
    ///
    /// # Returns
    ///
    /// A `DownloadSize` reference that contains the total download size of the
    /// transaction.
    pub fn download_size(&self) -> Option<&DownloadSize> {
        self.download_size.get()
    }
}

impl Default for Transaction {
    fn default() -> Self {
        Self::new()
    }
}

/// Unified confirmation-gate decision (three layers + built-in TTY rule).
///
/// The gate controls whether the frontend waits for an interactive Y/n answer:
///
/// - `-y/--yes` ([`SyncOption::AssumeYes`]): force off — never gate.
/// - `tty == false` (non-interactive stdin): built-in "no gate".
/// - `--confirm` (`Session::force_confirm`): force on.
/// - `config.confirm`: global default (`false` → no gate unless forced).
///
/// The transaction *plan* is always rendered regardless of this decision; only
/// the wait-for-answer is gated.
pub(crate) fn should_gate(session: &Session, options: &[SyncOption], tty: bool) -> bool {
    if options.contains(&SyncOption::AssumeYes) {
        return false;
    }
    if !tty {
        return false;
    }
    if session.force_confirm() {
        return true;
    }
    session.config().confirm()
}

/// Send the transaction to the frontend for confirmation and wait for the result.
///
/// The plan is always rendered (the event is always sent); `gate` tells the
/// frontend whether to wait for an interactive Y/n answer or auto-approve.
///
/// Returns `Ok(true)` if confirmed, `Ok(false)` if rejected or no emitter is
/// available (the channel sender is dropped before the response arrives).
async fn confirm_transaction(
    session: &Session,
    transaction: &Transaction,
    gate: bool,
) -> Fallible<bool> {
    let tx = match session.emitter() {
        Some(tx) => tx,
        None => return Ok(true),
    };

    if tx
        .try_send(Event::PromptTransactionNeedConfirm {
            transaction: transaction.clone(),
            gate,
        })
        .is_err()
    {
        return Ok(true);
    }

    while let Some(event) = session.receive_response().await {
        if let Event::PromptTransactionNeedConfirmResult(ret) = event {
            return Ok(ret);
        }
    }

    Err(Error::Custom("event bus closed unexpectedly".to_owned()))
}

// ─── Session-level sync operation ───────────────────────────────────────────

/// Sync packages.
///
/// # Note
/// The meaning of `sync` packages is to download, (un)install and/or upgrade
/// packages.
///
/// # Errors
///
/// I/O errors will be returned if the `apps`/`buckets` directory is not readable.
///
/// A [`PackageNotFound`][1] error will be returned if no package is found for
/// the given query.
///
/// A [`PackageMultipleCandidates`][2] error will be returned if multiple
/// candidates are found for the given query and not able to ask for a selection.
///
/// [1]: crate::Error::PackageNotFound
/// [2]: crate::Error::PackageMultipleCandidates
pub async fn sync(session: &Session, queries: Vec<&str>, options: Vec<SyncOption>) -> Fallible<()> {
    // remove possible duplicates
    let queries = std::collections::HashSet::<&str>::from_iter(queries)
        .into_iter()
        .collect::<Vec<_>>();

    if let Some(tx) = session.emitter() {
        let _ = tx.try_send(Event::PackageResolveStart);
    }

    let is_op_remove = options.contains(&SyncOption::Remove);
    let result = if is_op_remove {
        remove(session, &queries, &options).await
    } else {
        install(session, &queries, &options).await
    };

    // Always close the resolve/sync phases — including error paths (e.g.
    // a declined confirmation) — so the event loop breaks and the CLI's
    // handle.join() cannot hang.
    if let Some(tx) = session.emitter() {
        let _ = tx.try_send(Event::PackageResolveDone);
        let _ = tx.try_send(Event::PackageSyncDone);
    }

    result?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a session with an active event bus for confirm tests.
    fn setup_confirm_test() -> (
        Session,
        tokio::sync::mpsc::Sender<Event>,
        tokio::sync::mpsc::Receiver<Event>,
    ) {
        let session = Session::new();
        let bus = session.event_bus();
        let frontend_tx = bus.sender(); // outer_tx → session.inner_rx
        let frontend_rx = bus.receiver().expect("fresh event bus receiver"); // outer_rx ← session.inner_tx
        (session, frontend_tx, frontend_rx)
    }

    /// Test that confirm_transaction returns true when the frontend accepts.
    #[tokio::test]
    async fn test_confirm_transaction_accepted() {
        let (session, frontend_tx, mut frontend_rx) = setup_confirm_test();

        // Pre-send the acceptance response (buffered channel)
        frontend_tx
            .send(Event::PromptTransactionNeedConfirmResult(true))
            .await
            .unwrap();

        let transaction = Transaction::default();
        let result = confirm_transaction(&session, &transaction, true)
            .await
            .unwrap();
        assert!(result, "should return true when frontend accepts");

        // Verify the request was actually emitted (with the gate flag)
        let request = frontend_rx.recv().await.unwrap();
        assert!(
            matches!(
                request,
                Event::PromptTransactionNeedConfirm { gate: true, .. }
            ),
            "should have emitted a gated PromptTransactionNeedConfirm"
        );
    }

    /// Test that confirm_transaction returns false when the frontend rejects.
    #[tokio::test]
    async fn test_confirm_transaction_rejected() {
        let (session, frontend_tx, _frontend_rx) = setup_confirm_test();

        // Pre-send the rejection response
        frontend_tx
            .send(Event::PromptTransactionNeedConfirmResult(false))
            .await
            .unwrap();

        let transaction = Transaction::default();
        let result = confirm_transaction(&session, &transaction, true)
            .await
            .unwrap();
        assert!(!result, "should return false when frontend rejects");
    }

    // --- should_gate ---

    /// `-y/--yes` force-off always wins, even on a TTY.
    #[test]
    fn should_gate_assume_yes() {
        let session = Session::new();
        session.set_force_confirm(true);
        assert!(!should_gate(&session, &[SyncOption::AssumeYes], true));
    }

    /// Non-TTY stdin never gates, even with `--confirm` or `config.confirm`.
    #[test]
    fn should_gate_non_tty() {
        let session = Session::new();
        session.set_force_confirm(true);
        assert!(!should_gate(&session, &[], false));
    }

    /// `--confirm` forces the gate on a TTY.
    #[test]
    fn should_gate_force_confirm() {
        let session = Session::new();
        session.set_force_confirm(true);
        assert!(should_gate(&session, &[], true));
    }

    /// `config.confirm` enables the gate on a TTY; the default (`false`) does not.
    #[test]
    fn should_gate_config_confirm() {
        let session = Session::new();
        session
            .config_mut()
            .unwrap()
            .set("confirm", "true")
            .unwrap();
        assert!(should_gate(&session, &[], true));

        let default = Session::new();
        assert!(!should_gate(&default, &[], true));
    }
}
