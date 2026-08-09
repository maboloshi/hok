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
//! - **Config borrowing**: `config()` returns a `Ref<Config>` (immutable);
//!   `config_mut()` (crate-internal) returns a `RefMut<Config>` and fails
//!   with `ConfigInUse` if the config is already borrowed.
//!
//! [1]: crate::Event
//!
//! # Thread safety
//!
//! `Session` is `Sync + Send` because all mutable fields use either
//! `RefCell` (config) with runtime borrow-checking, `Cell` (global flag)
//! for `Copy` types, or `OnceCell` (event bus, user agent) for write-once.
//! Callers must ensure `config_mut()` is not called while `config()` is
//! held on the same thread.

use flume::{Receiver, Sender};
use std::cell::{Cell, OnceCell, Ref, RefCell, RefMut};
use std::path::Path;
use tracing::{debug, info, warn};

use crate::{
    config::{possible_config_paths, Config, ConfigBuilder},
    error::{Error, Fallible},
    event::{Event, EventBus},
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
    config: RefCell<Config>,

    /// Full duplex channel for event transmission back and forth
    event_bus: OnceCell<EventBus>,

    /// User agent for the session
    pub(crate) user_agent: OnceCell<String>,

    /// Structured output sink injected by the frontend
    ///
    /// [`OutputSink`][1] is not `Debug`, so [`Session`] implements `Debug`
    /// manually (see below).
    ///
    /// [1]: crate::output::OutputSink
    output: OnceCell<OutputSink>,

    /// Whether operations should target the global Scoop root.
    global: Cell<bool>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("config", &self.config)
            .field("event_bus", &self.event_bus)
            .field("user_agent", &self.user_agent)
            .field("output", &"<sink>")
            .field("global", &self.global)
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
                config: RefCell::new(config),
                event_bus: OnceCell::new(),
                user_agent: OnceCell::new(),
                output: OnceCell::new(),
                global: Cell::new(false),
            };
            let _ = session.event_bus();
            return session;
        }

        // Config loading failed, create a new default config and return.
        let config = RefCell::new(Config::init());
        let session = Session {
            config,
            event_bus: OnceCell::new(),
            user_agent: OnceCell::new(),
            output: OnceCell::new(),
            global: Cell::new(false),
        };
        // Initialize event bus and emit fallback warning
        let _ = session.event_bus();
        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::ConfigLoadFallback);
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
        let config = RefCell::new(ConfigBuilder::new().path(config_path).load()?);

        // Apply the `default_architecture` config as the process-wide
        // architecture override (Scoop's `Get-DefaultArchitecture` config path).
        apply_default_architecture(&config.borrow());

        Ok(Session {
            config,
            event_bus: OnceCell::new(),
            user_agent: OnceCell::new(),
            output: OnceCell::new(),
            global: Cell::new(false),
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
    pub fn config(&self) -> Ref<'_, Config> {
        self.config.borrow()
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
        self.global.set(global);
    }

    /// Check whether operations should target the global Scoop root.
    pub fn is_global(&self) -> bool {
        self.global.get()
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
        if self.global.get() {
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
    /// accounts for global mode. Use [`Session::app_dir`] / [`Session::version_dir`]
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

    /// Get the `current` junction directory of a package: `<root>/apps/<name>/current`.
    #[inline]
    pub fn current_dir(&self, name: &str) -> std::path::PathBuf {
        self.app_dir(name).join("current")
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
    /// from [`Session::current_dir`] (the junction path) and
    /// [`Session::current_dir_name`] (the runtime entry directory name).
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
        self.effective_root_path().join("persist").join(name)
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

    /// Get a mutable reference to the config held by the session.
    ///
    /// This method is only directly accessible from within the crate itself.
    /// It maybe indirectly used by other public available APIs to (indirectly)
    /// mutate the config. See [`Session::config`] for more details.
    pub(crate) fn config_mut(&self) -> Fallible<RefMut<'_, Config>> {
        self.config.try_borrow_mut().map_err(|_| Error::ConfigInUse)
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
    pub(crate) fn emitter(&self) -> Option<Sender<Event>> {
        self.event_bus.get().map(|bus| bus.inner_sender())
    }

    /// Get an inbound receiver to reveive events.
    pub(crate) fn receiver(&self) -> Option<&Receiver<Event>> {
        self.event_bus.get().map(|bus| bus.inner_receiver())
    }

    /// Set the user agent for the session.
    ///
    /// User agent is used when performing network related operations such as
    /// downloading packages. User agent for a session can only be set once.
    /// If not set, the default user agent will be used. The default user agent
    /// is `Scoop/1.0 (+http://scoop.sh/)`.
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
}
