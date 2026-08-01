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
//! - **Config borrowing**: `config()` returns a `Ref<Config>` (immutable);
//!   `config_mut()` (crate-internal) returns a `RefMut<Config>` and fails
//!   with `ConfigInUse` if the config is already borrowed.
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
use tracing::{debug, info};

use crate::{
    config::{possible_config_paths, Config, ConfigBuilder},
    error::{Error, Fallible},
    event::{Event, EventBus},
};

/// A handle representing a Scoop session.
#[derive(Debug)]
pub struct Session {
    /// [`Config`][1] for the session
    ///
    /// [1]: crate::config::Config
    config: RefCell<Config>,

    /// Full duplex channel for event transmission back and forth
    event_bus: OnceCell<EventBus>,

    /// User agent for the session
    pub(crate) user_agent: OnceCell<String>,

    /// Whether operations should target the global Scoop root.
    global: Cell<bool>,
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
        // Try to load config from the possible config paths, once a successful
        // load is done, return the session immediately.
        for path in possible_config_paths() {
            debug!("trying to load config from {}", path.display());
            if let Ok(session) = Self::new_with(&path) {
                info!("config loaded from {}", path.display());
                return session;
            }
        }

        // Config loading failed, create a new default config and return.
        let config = RefCell::new(Config::init());
        let session = Session {
            config,
            event_bus: OnceCell::new(),
            user_agent: OnceCell::new(),
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

        Ok(Session {
            config,
            event_bus: OnceCell::new(),
            user_agent: OnceCell::new(),
            global: Cell::new(false),
        })
    }

    /// Get an immutable reference to the config held by the session.
    ///
    /// This method is primarily used for doing a fine-grained read to the
    /// config aside from reading it as a whole via [`config_list`][1]. Caller
    /// of this method may not be able to perform some [`operations`][2], which
    /// will internally alter the config, before the reference is dropped.
    ///
    /// [1]: crate::operation::config_list
    /// [2]: crate::operation
    pub fn config(&self) -> Ref<'_, Config> {
        self.config.borrow()
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
    /// 根据 [`is_global()`] 的值返回对应的 Scoop 根目录：
    /// - `is_global() == true` → 返回全局安装根目录 (`global_path`)
    /// - `is_global() == false` → 返回用户级安装根目录 (`root_path`)
    ///
    /// # ⚠️ 推荐使用此方法
    ///
    /// 凡是需要解析包目录（`apps/`）、bucket 路径等的代码，应优先调用
    /// `effective_root_path()` 而不是手动判断 `is_global()` 后分别调用
    /// `config().root_path()` 或 `config().global_path()`，以避免遗漏全局模式。
    ///
    /// # Example
    ///
    /// ```no_run
    /// use libscoop::Session;
    ///
    /// let session = Session::new();
    /// let apps_dir = session.effective_root_path().join("apps");
    /// ```
    pub fn effective_root_path(&self) -> std::path::PathBuf {
        let config = self.config();
        if self.global.get() {
            config.global_path().to_path_buf()
        } else {
            config.root_path().to_path_buf()
        }
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
}
