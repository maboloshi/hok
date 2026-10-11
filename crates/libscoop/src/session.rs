//! Core session type — a handle to global Scoop state.
//!
//! [`Session`] is the primary entry point of `libscoop`. Almost every public
//! operation takes a `&Session` reference as its first argument. A session
//! owns the configuration, the event bus, and global flags — but **no**
//! operation state (which is ephemeral and held by the callers).
//!
//! # Design
//!
//! - **Handle, not a singleton**: Multiple sessions can coexist, though
//!   typically only one is created per process (by the binary entry point).
//!   Each session loads its config independently.
//! - **Config fallback chain**: [`Session::new()`] iterates over
//!   [`possible_config_paths()`]; the first successful load wins. If all
//!   paths fail, a default config is created and a `ConfigLoadFallback`
//!   event is emitted.
//! - **Lazy event bus**: The event bus (`EventBus`) is created on first
//!   access via [`OnceCell`], so sessions that never interact with the
//!   frontend avoid the allocation.
//! - **Global flag**: [`Session::set_global()`] / [`is_global()`] control
//!   whether operations use the system-wide Scoop root (`global_path`) or
//!   the user-local root (`root_path`). Checked by operations at runtime.
//! - **Output sink**: structured output is emitted through
//!   [`Session::output()`] and forwarded to the sink injected by the frontend
//!   via [`Session::set_output()`]. Without a sink, output is silently
//!   dropped — the library never writes to stdout/stderr itself. The sink is
//!   synchronous (unlike the async [`Event`][1] bus), so commands that do not
//!   run an event loop (e.g. `ci-auto-pr`, `cache`, `cleanup`) can still
//!   emit output safely.
//! - **Config borrowing**: `config()` returns a read guard (immutable);
//!   `config_mut()` (crate-internal) returns a write guard and fails
//!   with `ConfigInUse` if the config is already locked for writing.
//!
//! [1]: crate::Event
//!
//! # Thread safety
//!
//! `Session` is `Sync + Send`: the config is guarded by a `RwLock`,
//! the global flag is an `AtomicBool`, and write-once slots (event bus,
//! user agent, output sink, infra capabilities) use `OnceLock` — so a
//! `Session` can be shared across threads and tokio tasks. Callers must
//! not call `config_mut()` while a `config()` guard is still held
//! (it fails with `ConfigInUse`).

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::{
    config::{Config, ConfigBuilder, possible_config_paths},
    error::{Error, Fallible},
    event::{Event, EventBus},
    internal::archive::{Archive, RealArchive},
    internal::git::{Git, RealGit},
    internal::network::{Network, RealNetwork},
    output::{OutputHandle, OutputSink},
};

/// A handle representing a Scoop session.
///
/// [`Session`] implements [`Debug`] manually (see below) because the
/// [`output`][1] sink is not `Debug`.
///
/// [1]: crate::output::OutputSink
pub struct Session {
    /// [`Config`][1] for the session
    ///
    /// [1]: crate::config::Config
    config: RwLock<Config>,

    /// Full duplex channel for event transmission back and forth
    event_bus: OnceLock<EventBus>,

    /// User agent for the session
    pub(crate) user_agent: OnceLock<String>,

    /// Structured output sink injected by the frontend
    ///
    /// [`OutputSink`][1] is not `Debug`, so [`Session`] implements `Debug`
    /// manually (see below).
    ///
    /// [1]: crate::output::OutputSink
    output: OnceLock<OutputSink>,

    /// Whether operations should target the global Scoop root.
    global: AtomicBool,

    /// Whether the user forced the transaction confirmation gate on
    /// (`--confirm`); overrides the `confirm` config default.
    force_confirm: AtomicBool,

    /// Whether the "manifest cache may be stale" notice has already gone out.
    ///
    /// Per session, not per process: the notice is written to this session's
    /// output sink, so the session that served the rows is the one that
    /// remembers having said so (see [`Session::mark_stale_warned`]).
    /// The per-key once-notices live in [`Session::mark_warned_once`].
    stale_warned: AtomicBool,

    /// Keys of the once-per-session notices already emitted through this
    /// session's output sink: manifest-cache mismatches (`bucket/package`) and
    /// manifest parse failures (`parse:bucket/package`) — see
    /// [`Session::mark_warned_once`].
    warned_once: Mutex<HashSet<String>>,

    /// Injectable network capability, defaulting to the real implementation.
    network: OnceLock<Arc<dyn Network + Send + Sync>>,

    /// Injectable git capability, defaulting to the real implementation.
    git: OnceLock<Arc<dyn Git + Send + Sync>>,

    /// Injectable archive capability, defaulting to the real implementation.
    archive: OnceLock<Arc<dyn Archive + Send + Sync>>,

    /// Cooperative cancellation token (purity P2): wired to Ctrl-C by the CLI
    /// entry point and checked by long-running operations (downloads, scripts,
    /// waits) so an interrupt aborts cleanly (temp files, cursor, subprocess).
    /// Defaults to a never-cancelled token when the frontend never sets one.
    cancellation: OnceLock<CancellationToken>,

    /// Resident PowerShell engine slot for package scripts (the `script_engine`
    /// config path). Lazily created on first use; `None` until a submission
    /// runs. Accessed crate-internally by [`package::operations::script`]; the
    /// engine is taken out for a blocking submission and put back afterwards.
    script_engine: OnceLock<Arc<Mutex<Option<crate::package::operations::psengine::PsEngine>>>>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("config", &self.config)
            .field("event_bus", &self.event_bus)
            .field("user_agent", &self.user_agent)
            .field("output", &"<sink>")
            .field("global", &self.global)
            .field("network", &"<infra>")
            .field("git", &"<infra>")
            .field("archive", &"<infra>")
            .field("cancellation", &"<cancellation>")
            .field("script_engine", &"<script_engine>")
            .finish()
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// Create a new session.
    ///
    /// The default config path will be used to locate the config file for the
    /// session.
    ///
    /// # Returns
    ///
    /// A new session.
    pub fn new() -> Session {
        // Find the first existing Scoop config file to use as a read-only
        // fallback source; hok's own config file overlays it.
        let scoop_path = possible_config_paths().into_iter().find(|p| p.exists());
        if let Some(path) = &scoop_path {
            debug!("using scoop config as fallback: {}", path.display());
        }

        let mut builder = ConfigBuilder::new();
        if let Some(path) = &scoop_path {
            builder = builder.scoop_path(path);
        }
        if let Ok(config) = builder.load() {
            info!("config loaded from {}", config.path.display());
            apply_default_architecture(&config);
            let session = Session {
                config: RwLock::new(config),
                event_bus: OnceLock::new(),
                user_agent: OnceLock::new(),
                output: OnceLock::new(),
                global: AtomicBool::new(false),
                force_confirm: AtomicBool::new(false),
                stale_warned: AtomicBool::new(false),
                warned_once: Mutex::new(HashSet::new()),
                network: OnceLock::new(),
                git: OnceLock::new(),
                archive: OnceLock::new(),
                cancellation: OnceLock::new(),
                script_engine: OnceLock::new(),
            };
            let _ = session.event_bus();
            return session;
        }

        // Config loading failed, create a new default config and return.
        let config = RwLock::new(crate::config::init_default());
        let session = Session {
            config,
            event_bus: OnceLock::new(),
            user_agent: OnceLock::new(),
            output: OnceLock::new(),
            global: AtomicBool::new(false),
            force_confirm: AtomicBool::new(false),
            stale_warned: AtomicBool::new(false),
            warned_once: Mutex::new(HashSet::new()),
            network: OnceLock::new(),
            git: OnceLock::new(),
            archive: OnceLock::new(),
            cancellation: OnceLock::new(),
            script_engine: OnceLock::new(),
        };
        // Initialize event bus and emit fallback warning
        let _ = session.event_bus();
        if let Some(tx) = session.emitter() {
            let _ = tx.try_send(Event::ConfigLoadFallback);
        }
        session
    }

    /// Create a new session with the given config path.
    ///
    /// # Returns
    ///
    /// A new session.
    ///
    /// # Errors
    ///
    /// This method will return an error if the config file is not found or
    /// cannot be parsed.
    pub fn new_with<P>(config_path: P) -> Fallible<Session>
    where
        P: AsRef<Path>,
    {
        let config = RwLock::new(ConfigBuilder::new().path(config_path).load()?);

        // Apply the `default_architecture` config as the process-wide
        // architecture override (Scoop's `Get-DefaultArchitecture` config path).
        apply_default_architecture(&config.read().unwrap_or_else(|e| e.into_inner()));

        Ok(Session {
            config,
            event_bus: OnceLock::new(),
            user_agent: OnceLock::new(),
            output: OnceLock::new(),
            global: AtomicBool::new(false),
            force_confirm: AtomicBool::new(false),
            stale_warned: AtomicBool::new(false),
            warned_once: Mutex::new(HashSet::new()),
            network: OnceLock::new(),
            git: OnceLock::new(),
            archive: OnceLock::new(),
            cancellation: OnceLock::new(),
            script_engine: OnceLock::new(),
        })
    }

    /// Get an immutable reference to the config held by the session.
    ///
    /// This method is primarily used for doing a fine-grained read to the
    /// config aside from reading it as a whole via [`config_list`][1]. Caller
    /// of this method may not be able to perform some operations, which
    /// will internally alter the config, before the reference is dropped.
    ///
    /// [1]: crate::config::list
    pub fn config(&self) -> std::sync::RwLockReadGuard<'_, Config> {
        self.config.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Check whether the current process has administrator privileges.
    ///
    /// Uses `IsUserAnAdmin()` from `shell32.dll` on Windows; always returns
    /// `false` on other platforms.
    pub fn is_admin(&self) -> bool {
        crate::internal::os::is_admin()
    }

    /// Set whether operations should target the global Scoop root.
    pub fn set_global(&self, global: bool) {
        self.global.store(global, Ordering::Relaxed);
    }

    /// Check whether operations should target the global Scoop root.
    pub fn is_global(&self) -> bool {
        self.global.load(Ordering::Relaxed)
    }

    /// Force the transaction confirmation gate on (`--confirm`).
    pub fn set_force_confirm(&self, force: bool) {
        self.force_confirm.store(force, Ordering::Relaxed);
    }

    /// Whether the transaction confirmation gate was forced on by `--confirm`.
    pub fn force_confirm(&self) -> bool {
        self.force_confirm.load(Ordering::Relaxed)
    }

    /// Claim this session's single "the manifest cache may be stale" notice.
    ///
    /// Returns `true` for the caller that must emit it, `false` for every
    /// later caller in the same session. The claim is per *session* because the
    /// notice is written through [`Session::output()`]: one process can hold
    /// several sessions (tests, embedders), each with its own sink, and a
    /// notice one sink never saw must not be suppressed by another sink having
    /// seen it.
    pub(crate) fn mark_stale_warned(&self) -> bool {
        !self.stale_warned.swap(true, Ordering::Relaxed)
    }

    /// Claim this session's notice for `key`, an opaque identifier of the
    /// thing being reported (the manifest-cache notices pass
    /// `bucket/package`).
    ///
    /// Returns `true` for the caller that must emit it, `false` when this
    /// session already reported that key: one command can reach the same
    /// package through several code paths (`depends` resolves the package and
    /// then resolves its tree; an explicit query both re-reads the file behind
    /// a cache row and probes it by name), and the notice is about the thing,
    /// not about the code path that found it.
    pub(crate) fn mark_warned_once(&self, key: &str) -> bool {
        let mut warned = self
            .warned_once
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if warned.contains(key) {
            return false;
        }
        warned.insert(key.to_owned());
        true
    }

    /// Get the effective root path based on the global flag.
    ///
    /// Returns the corresponding Scoop root directory based on the value of [`Session::is_global()`]:
    /// - `is_global() == true` → returns the global installation root directory (`global_path`)
    /// - `is_global() == false` → returns the user-level installation root directory (`root_path`)
    ///
    /// # ⚠️ Recommended to use this method
    ///
    /// Whenever code needs to resolve package directories (`apps/`), bucket paths, etc., it should prioritize calling
    /// `effective_root_path()` rather than manually checking `is_global()` and then separately calling
    /// `config().root_path()` or `config().global_path()`, to avoid missing the global mode.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use libscoop::Session;
    ///
    /// let session = Session::new();
    /// let apps_dir = session.apps_dir();
    /// ```
    pub fn effective_root_path(&self) -> std::path::PathBuf {
        let config = self.config();
        if self.global.load(Ordering::Relaxed) {
            config.global_path().to_path_buf()
        } else {
            config.root_path().to_path_buf()
        }
    }

    // ─── Layout directories ─────────────────────────────────────────────
    // Scoop splits its layout into two root families (mirrors upstream
    // `core.ps1`): `apps`/`shims`/`modules`/`persist` follow the *effective*
    // root (user or global per `-g`; upstream `appsdir`/`shimdir`/
    // `persistdir` take `$global`), while `buckets` are pinned to the
    // *user-level* root (`$scoopdir\buckets`, upstream `buckets.ps1:1`).
    // These accessors hide that distinction from callers.

    /// Get the `apps` directory under the effective root: `<root>/apps`.
    ///
    /// The root follows [`Session::effective_root_path()`], so this already
    /// accounts for global mode. Use [`Session::app_dir`] / [`Session::versioned_dir`]
    /// for the per-package/per-version variants.
    #[inline]
    pub fn apps_dir(&self) -> std::path::PathBuf {
        self.effective_root_path().join("apps")
    }

    /// Get the install directory of a package: `<root>/apps/<name>`.
    #[inline]
    pub fn app_dir(&self, name: &str) -> std::path::PathBuf {
        self.effective_root_path().join("apps").join(name)
    }

    /// Get the directory a package's current install actually lives in:
    /// `<root>/apps/<name>/current` when junctions are enabled, otherwise the
    /// resolved version dir.
    ///
    /// Mirrors upstream `currentdir` (lib/core.ps1:355-362) verbatim: the
    /// `NO_JUNCTION` branch resolves the version through
    /// `Select-CurrentVersion` and returns the version dir, because there is
    /// no junction to point at. Callers that need "the install dir of this
    /// package" (upstream's `$dir`, e.g. `prefix`, `hold`) must use this:
    /// building `<root>/apps/<name>/current` by hand is the same mistake as
    /// assuming a junction, and it silently targets a path that does not exist
    /// under `NO_JUNCTION`.
    ///
    /// Returns `None` when no version resolves (not installed, or a broken
    /// install with no version dir).
    ///
    /// Upstream divergence, deliberate: when `Select-CurrentVersion` returns
    /// `$null` upstream still builds `"$appdir\"` (a half-empty path); here
    /// the caller gets `None` and reports a broken install instead.
    pub fn app_current_dir(&self, name: &str) -> Option<std::path::PathBuf> {
        self.app_current_dir_in(name, self.is_global())
    }

    /// [`Session::app_current_dir`] for an explicit scope.
    ///
    /// Upstream `currentdir $app $global` takes the scope as an argument and
    /// callers pass each one in turn — `prefix` tries the local root and then
    /// the global one (`libexec/scoop-prefix.ps1:12-15`). Using
    /// [`Session::app_current_dir`] for that would silently resolve both
    /// probes against whichever scope the session happens to be in.
    pub fn app_current_dir_in(&self, name: &str, global: bool) -> Option<std::path::PathBuf> {
        let root = if global {
            self.config().global_path().to_path_buf()
        } else {
            self.config().root_path().to_path_buf()
        };
        let pkg_dir = root.join("apps").join(name);
        if !self.config().no_junction() {
            return Some(pkg_dir.join("current"));
        }
        crate::package::install_state::resolve_current_version(&pkg_dir, true)
            .map(|resolved| pkg_dir.join(resolved.version))
    }

    /// Get the directory name a package's runtime entries (shims, shortcuts,
    /// env) should resolve to: `current` when junctions are enabled,
    /// otherwise the actual version directory name.
    ///
    /// Mirrors upstream Scoop's `currentdir` (lib/core.ps1), which resolves
    /// `NO_JUNCTION` to `Select-CurrentVersion` and otherwise returns
    /// `current`; `link_current` (lib/install.ps1) likewise returns the
    /// version dir under `NO_JUNCTION`.
    #[inline]
    pub fn current_dir_name<'a>(&self, version: &'a str) -> &'a str {
        if self.config().no_junction() {
            version
        } else {
            "current"
        }
    }

    /// Get the versioned directory of a package: `<root>/apps/<name>/<version>`.
    ///
    /// This is the *real* per-version directory — extraction target,
    /// `link_current` target, and uninstall/removal location. It is distinct
    /// from [`Session::app_current_dir`] (the entry directory, which is the
    /// `current` junction only in junction mode) and
    /// [`Session::current_dir_name`] (the runtime entry directory *name*).
    #[inline]
    pub fn versioned_dir(&self, name: &str, version: &str) -> std::path::PathBuf {
        self.app_dir(name).join(version)
    }

    /// Get the `shims` directory under the effective root: `<root>/shims`.
    #[inline]
    pub fn shims_dir(&self) -> std::path::PathBuf {
        self.effective_root_path().join("shims")
    }

    /// Get the PowerShell `modules` directory: `<root>/modules`.
    #[inline]
    pub fn modules_dir(&self) -> std::path::PathBuf {
        self.effective_root_path().join("modules")
    }

    /// Get the persist directory of a package: `<root>/persist/<name>`.
    ///
    /// Follows the *effective* root (upstream `persistdir $app $global`),
    /// so global installs keep their persist data under the global root.
    #[inline]
    pub fn persist_dir(&self, name: &str) -> std::path::PathBuf {
        self.persist_root().join(name)
    }

    /// Get the persist root directory: `<root>/persist`.
    ///
    /// Follows the *effective* root (upstream `persistdir $null $global`),
    /// used by `persist_permission` to grant Users write access on the
    /// global persist root for non-admin users.
    #[inline]
    pub fn persist_root(&self) -> std::path::PathBuf {
        self.effective_root_path().join("persist")
    }

    /// Get the `buckets` directory: `<root>/buckets`.
    ///
    /// Pinned to the *user-level* root (upstream `$scoopdir\buckets`) even
    /// for global installs — buckets never move with `-g`.
    #[inline]
    pub fn buckets_dir(&self) -> std::path::PathBuf {
        self.config().root_path().join("buckets")
    }

    /// Get a single bucket directory: `<root>/buckets/<name>`.
    ///
    /// User-level root, same as [`Session::buckets_dir`].
    #[inline]
    pub fn bucket_dir(&self, name: &str) -> std::path::PathBuf {
        self.config().root_path().join("buckets").join(name)
    }

    /// Get the `workspace` directory: `<root>/workspace`.
    ///
    /// Mirrors upstream `usermanifestsdir` (lib/core.ps1) — generated
    /// user manifests (e.g. `app@version` installs) are written here.
    /// User-level root, same as [`Session::buckets_dir`].
    #[inline]
    pub fn workspace_dir(&self) -> std::path::PathBuf {
        self.config().root_path().join("workspace")
    }

    /// Get a mutable reference to the config held by the session.
    ///
    /// This method is only directly accessible from within the crate itself.
    /// It maybe indirectly used by other public available APIs to (indirectly)
    /// mutate the config. See [`Session::config`] for more details.
    pub(crate) fn config_mut(&self) -> Fallible<std::sync::RwLockWriteGuard<'_, Config>> {
        self.config.try_write().map_err(|_| Error::ConfigInUse)
    }

    /// Get the event bus for the session.
    ///
    /// The event bus is used for transmitting [`events`][1] between the session
    /// backend and the caller frontend.
    ///
    /// # Returns
    ///
    /// The event bus for the session.
    ///
    /// [1]: crate::Event
    pub fn event_bus(&self) -> &EventBus {
        self.event_bus.get_or_init(EventBus::new)
    }

    /// Get an outbound sender to emit events.
    pub(crate) fn emitter(&self) -> Option<mpsc::Sender<Event>> {
        self.event_bus.get().map(|bus| bus.inner_sender())
    }

    /// Await the next inbound (frontend → backend) event.
    ///
    /// Used to wait for the response to a prompt emitted on the outbound
    /// channel (e.g. a transaction confirmation or candidate selection).
    pub(crate) async fn receive_response(&self) -> Option<Event> {
        self.event_bus().receive_response().await
    }

    /// Set the user agent for the session.
    ///
    /// User agent is used when performing network related operations such as
    /// downloading packages. User agent for a session can only be set once.
    /// If not set, [`DEFAULT_USER_AGENT`][crate::constant::DEFAULT_USER_AGENT]
    /// is used. The hok CLI never overrides it, so every hok request carries the
    /// same User-Agent; the setter exists for embedders that need their own.
    ///
    /// # Errors
    ///
    /// This method will return an error if the user agent has already been set.
    pub fn set_user_agent(&self, user_agent: &str) -> Fallible<()> {
        self.user_agent
            .set(user_agent.to_owned())
            .map_err(|_| Error::UserAgentAlreadySet)
    }

    /// Get the custom user agent, if set.
    pub fn user_agent(&self) -> Option<&str> {
        self.user_agent.get().map(|s| s.as_str())
    }

    /// Set the output sink for this session.
    ///
    /// All structured output emitted by library operations (via
    /// [`Session::output`]) is forwarded to the sink. When no sink is set,
    /// output is silently dropped — the library never writes to
    /// stdout/stderr itself.
    ///
    /// # Errors
    ///
    /// This method will return an error if the output sink has already been
    /// set.
    pub fn set_output(&self, sink: OutputSink) -> Fallible<()> {
        self.output.set(sink).map_err(|_| Error::OutputAlreadySet)
    }

    /// Get a handle for emitting structured output.
    ///
    /// Each method on the returned handle forwards the corresponding
    /// [`Output`][1] request to the session's sink; without a sink the
    /// request is silently dropped.
    ///
    /// [1]: crate::output::Output
    pub fn output(&self) -> OutputHandle<'_> {
        OutputHandle::new(self.output.get())
    }

    /// Get the session's network capability.
    ///
    /// Defaults to the real HTTP implementation; tests inject an in-memory
    /// fake via [`Session::set_network`] to avoid touching the network.
    /// Returns a cloneable [`Arc`] so worker threads can share it (a session
    /// itself is `!Sync`).
    pub fn network(&self) -> Arc<dyn Network + Send + Sync> {
        self.network.get_or_init(|| Arc::new(RealNetwork)).clone()
    }

    /// Inject a custom network implementation.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InfraAlreadySet`] if one has already been injected.
    pub fn set_network(&self, net: Arc<dyn Network + Send + Sync>) -> Fallible<()> {
        self.network.set(net).map_err(|_| Error::InfraAlreadySet)
    }

    /// Get the session's git capability (default: real `git2`-based).
    pub fn git(&self) -> Arc<dyn Git + Send + Sync> {
        self.git.get_or_init(|| Arc::new(RealGit)).clone()
    }

    /// Inject a custom git implementation.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InfraAlreadySet`] if one has already been injected.
    pub fn set_git(&self, git: Arc<dyn Git + Send + Sync>) -> Fallible<()> {
        self.git.set(git).map_err(|_| Error::InfraAlreadySet)
    }

    /// Get the session's archive capability (default: real extractors).
    pub fn archive(&self) -> Arc<dyn Archive + Send + Sync> {
        self.archive.get_or_init(|| Arc::new(RealArchive)).clone()
    }

    /// Inject a custom archive implementation.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InfraAlreadySet`] if one has already been injected.
    pub fn set_archive(&self, archive: Arc<dyn Archive + Send + Sync>) -> Fallible<()> {
        self.archive
            .set(archive)
            .map_err(|_| Error::InfraAlreadySet)
    }

    /// Get the session's cooperative cancellation token.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation
            .get_or_init(CancellationToken::new)
            .clone()
    }

    /// Inject a cooperative cancellation token (wired to Ctrl-C by the CLI).
    pub fn set_cancellation_token(&self, token: CancellationToken) -> Fallible<()> {
        self.cancellation
            .set(token)
            .map_err(|_| Error::CancellationTokenAlreadySet)
    }

    /// The session's resident PowerShell engine slot (the `script_engine`
    /// config path). Lazily created on first access. The caller takes the
    /// engine out for a blocking submission and returns it afterwards.
    pub(crate) fn script_engine_slot(
        &self,
    ) -> &Arc<Mutex<Option<crate::package::operations::psengine::PsEngine>>> {
        self.script_engine
            .get_or_init(|| Arc::new(Mutex::new(None)))
    }

    /// Apply a CLI `-a/--arch` value as the process-wide architecture
    /// override (beats the `default_architecture` config, mirroring Scoop's
    /// `-a` flag parsed after session creation).
    pub fn set_default_architecture(&self, value: &str) -> Fallible<()> {
        let arch = crate::internal::arch::Arch::parse(value)?;
        crate::internal::arch::Arch::set_default_architecture(arch);
        Ok(())
    }
}

/// Apply the `default_architecture` config as the process-wide architecture
/// override, mirroring Scoop's `Get-DefaultArchitecture` config path. An
/// invalid value is logged and the runtime-detected architecture is kept.
fn apply_default_architecture(config: &Config) {
    if let Some(raw) = config.default_architecture() {
        match crate::internal::arch::Arch::parse(raw) {
            Ok(arch) => crate::internal::arch::Arch::set_default_architecture(arch),
            Err(_) => warn!(
                "invalid default architecture configured: {raw}; using the system architecture"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils;

    // Lock in that Session is shareable across threads/tokio tasks
    // (the refactor that replaced RefCell/Cell/OnceCell with
    // RwLock/AtomicBool/OnceLock).
    fn assert_sync_send<T: Sync + Send>() {}

    #[test]
    fn session_is_sync_send() {
        assert_sync_send::<Session>();
    }

    #[test]
    fn cancellation_token_defaults_to_never_cancelled() {
        let root = test_utils::tmpdir("session_cancel_default");
        let session = test_utils::test_session(&root);
        assert!(!session.cancellation_token().is_cancelled());
    }

    #[test]
    fn cancellation_token_can_be_injected_and_cancelled() {
        let root = test_utils::tmpdir("session_cancel_set");
        let session = test_utils::test_session(&root);
        let token = CancellationToken::new();
        session.set_cancellation_token(token.clone()).unwrap();
        // A second injection is rejected.
        assert!(
            session
                .set_cancellation_token(CancellationToken::new())
                .is_err()
        );
        assert!(!session.cancellation_token().is_cancelled());
        token.cancel();
        assert!(session.cancellation_token().is_cancelled());
    }

    #[test]
    fn current_dir_name_defaults_to_current() {
        let root = test_utils::tmpdir("session_current_dir_name");
        let session = test_utils::test_session(&root);
        assert_eq!(session.current_dir_name("1.2.3"), "current");
        assert_eq!(
            session.versioned_dir("git", "1.2.3"),
            root.join("apps/git/1.2.3")
        );
    }

    #[test]
    fn current_dir_name_uses_version_with_no_junction() {
        let root = test_utils::tmpdir("session_current_dir_name_no_junction");
        let config_path = root.join("hok.json");
        let root_escaped = root.to_string_lossy().replace('\\', "\\\\");
        let cache_escaped = root.join("cache").to_string_lossy().replace('\\', "\\\\");
        std::fs::write(
            &config_path,
            format!(
                r#"{{"root_path": "{}", "cache_path": "{}", "no_junction": true}}"#,
                root_escaped, cache_escaped
            ),
        )
        .unwrap();
        let session = Session::new_with(&config_path).unwrap();
        assert_eq!(session.current_dir_name("1.2.3"), "1.2.3");
    }

    /// `app_current_dir` mirrors upstream `currentdir`: the junction path in
    /// junction mode, the resolved version dir under NO_JUNCTION.
    #[test]
    fn app_current_dir_follows_junction_mode() {
        let root = test_utils::tmpdir("session_app_current_dir");
        let session = test_utils::test_session(&root);
        assert_eq!(
            session.app_current_dir("git"),
            Some(root.join("apps/git/current"))
        );

        let no_junction_root = test_utils::tmpdir("session_app_current_dir_no_junction");
        let no_junction_session = test_utils::test_session_with_junctions(&no_junction_root, true);
        let vdir = no_junction_root.join("apps").join("git").join("1.2.3");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("install.json"), r#"{"architecture": "64bit"}"#).unwrap();

        assert_eq!(no_junction_session.app_current_dir("git"), Some(vdir));
        // No resolvable version dir → broken, not a fabricated path.
        assert_eq!(no_junction_session.app_current_dir("ghost"), None);
    }
}
