//! Scoop configuration management.
//!
//! Provides [`Config`] — a thin wrapper around a JSON config file — and
//! [`ConfigBuilder`] for loading config from a path.
//!
//! # Design
//!
//! - **One file, one-time migration**: hok's own config file
//!   (`~/.config/hok/config.json`) is the only config file hok reads or
//!   writes. [`ConfigInner`] contains only the keys hok actually supports,
//!   so the serialized file is exactly the supported set by construction.
//!   On the first run, when hok's file does not exist yet, the supported
//!   keys are migrated once from Scoop's `config.json` (found via
//!   `possible_config_paths()`); afterwards Scoop's file is never
//!   consulted again and never modified.
//! - **Builder pattern**: [`ConfigBuilder`] sets the hok config path (and an
//!   optional read-only Scoop path used only for the first-run migration);
//!   [`Config`] provides typed accessors for supported keys.
//! - **Path discovery**: `possible_config_paths()` returns the known Scoop
//!   config file locations (user-local and global); [`crate::Session::new()`] picks
//!   the first one that exists as the migration source.
//! - **Defaults**: If no config file exists, `config::init_default()` creates a
//!   blank hok config with sensible defaults.
//!
//! # Thread safety
//!
//! `Config` is typically accessed via `Session::config()` which returns a
//! read guard. For mutation, `Session::config_mut()` provides a write guard.
//! Both enforce Rust's borrowing rules at runtime.

use std::collections::HashMap;
use std::io::Read;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::error::{Error, Fallible};
use crate::internal;

/// Builder pattern for generating [`Config`].
pub struct ConfigBuilder {
    /// Path of hok's own config file (the only file hok writes to).
    ///
    /// Default is [`default::hok_config_path()`].
    path: PathBuf,

    /// Optional path of Scoop's config file, used as a read-only fallback:
    /// values from this file are merged in first, then hok's own file
    /// overrides them. Never written to.
    scoop_path: Option<PathBuf>,
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigBuilder {
    pub fn new() -> ConfigBuilder {
        Self {
            path: default::hok_config_path(),
            scoop_path: None,
        }
    }

    pub fn path<P: AsRef<Path>>(&mut self, path: P) -> ConfigBuilder {
        Self {
            path: path.as_ref().to_owned(),
            scoop_path: self.scoop_path.clone(),
        }
    }

    /// Set the read-only Scoop config file to merge as a fallback.
    pub fn scoop_path<P: AsRef<Path>>(&mut self, path: P) -> ConfigBuilder {
        Self {
            path: self.path.clone(),
            scoop_path: Some(path.as_ref().to_owned()),
        }
    }

    /// Load hok's own config file.
    ///
    /// On first run (hok's file does not exist yet), the supported keys are
    /// migrated once from Scoop's read-only config file into a new hok
    /// config file; afterwards only hok's file is used.
    ///
    /// # Errors
    ///
    /// Returns an error when neither file exists (matching the previous
    /// single-file behavior), or when the config fails to parse.
    pub fn load(&self) -> Fallible<Config> {
        // hok's own file exists: load it directly. Scoop's file is only
        // consulted on first run.
        if let Ok(value) = read_json_file(&self.path) {
            let inner = serde_json::from_value(value)?;
            return Ok(Config {
                path: self.path.clone(),
                inner,
            });
        }

        // First run: migrate the supported keys from Scoop's read-only file
        // into a new hok config file, then use only that from now on.
        // Unsupported Scoop keys are dropped by deserialization.
        if let Some(scoop_path) = &self.scoop_path
            && let Ok(value) = read_json_file(scoop_path)
        {
            let inner: ConfigInner = serde_json::from_value(value)?;
            internal::fs::write_json(&self.path, &inner)?;
            return Ok(Config {
                path: self.path.clone(),
                inner,
            });
        }

        // Neither file exists: report the missing hok config file.
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("config file not found: {}", self.path.display()),
        )
        .into())
    }
}

/// Read a JSON config file into a [`serde_json::Value`], returning `Ok`
/// only when the file exists and parses.
///
/// The decode is lossy on purpose: upstream reads the config with
/// `[System.IO.File]::ReadAllLines` (`lib/core.ps1:82-95`), whose UTF-8
/// reader substitutes invalid bytes instead of failing. Parsing then
/// follows the shared tolerance chain ([`internal::json::parse_str`]).
fn read_json_file(path: &Path) -> Fallible<serde_json::Value> {
    let mut buf = vec![];
    std::fs::File::open(path)?.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    internal::json::parse_str(text.as_ref())
}

/// Scoop Configuration representation.
///
/// **NOTE**: `ConfigInner` contains only the fields hok actually supports;
/// Scoop-only settings (e.g. `use_external_7zip`, `scoop_repo`) are not
/// modeled and are silently dropped when a Scoop config is read.
#[derive(Clone, Debug)]
pub struct Config {
    /// The file path of this [`Config`].
    pub path: PathBuf,

    /// Inner config data.
    inner: ConfigInner,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConfigInner {
    #[serde(skip_serializing_if = "Option::is_none")]
    alias: Option<HashMap<String, String>>,

    #[serde(rename = "aria2-enabled")]
    #[serde(skip_serializing_if = "Option::is_none")]
    aria2_enabled: Option<bool>,

    #[serde(rename = "aria2-max-connection-per-server")]
    #[serde(skip_serializing_if = "Option::is_none")]
    aria2_max_connection_per_server: Option<u32>,

    #[serde(rename = "aria2-min-split-size")]
    #[serde(skip_serializing_if = "Option::is_none")]
    aria2_min_split_size: Option<String>,

    #[serde(rename = "aria2-split")]
    #[serde(skip_serializing_if = "Option::is_none")]
    aria2_split: Option<u32>,

    /// Concurrent package downloads (cross-package). Defaults to 4.
    #[serde(skip_serializing_if = "Option::is_none")]
    jobs: Option<u32>,

    #[serde(default = "default::cache_path")]
    #[serde(skip_serializing_if = "default::is_default_cache_path")]
    cache_path: PathBuf,

    #[serde(skip_serializing_if = "Option::is_none")]
    cat_style: Option<String>,

    /// The text editor used by `hok config edit` (a hok-specific key: upstream
    /// has no editor launch at all and never reads `EDITOR`). When unset,
    /// `$EDITOR` is used, falling back to the system default handler.
    #[serde(skip_serializing_if = "Option::is_none")]
    editor: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    no_color: Option<bool>,

    /// Whether to show the interactive transaction confirmation gate by
    /// default (`false` — sync commands don't wait for Y/n unless `--confirm`
    /// is given or this is `true`).
    #[serde(skip_serializing_if = "Option::is_none")]
    confirm: Option<bool>,

    /// Route package script stages through a resident PowerShell engine
    /// (reusing one long-lived pwsh worker across stages) instead of spawning
    /// a fresh `pwsh -File` per stage. Defaults to `false` — the well-trodden
    /// per-stage spawn path stays the default; the engine path is opt-in.
    #[serde(skip_serializing_if = "Option::is_none")]
    script_engine: Option<bool>,

    /// The default architecture to use (Scoop's `default_architecture`
    /// config, see `Get-DefaultArchitecture`). When set, it overrides the
    /// runtime-detected host architecture for manifest field selection.
    #[serde(skip_serializing_if = "Option::is_none")]
    default_architecture: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gh_token: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub virustotal_api_key: Option<String>,

    #[serde(default = "default::global_path")]
    #[serde(skip_serializing_if = "default::is_default_global_path")]
    global_path: PathBuf,

    #[serde(skip_serializing_if = "Option::is_none")]
    last_update: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    use_isolated_path: Option<IsolatedPath>,

    /// Use SQLite to cache manifests.
    ///
    /// This config was introduced in Scoop v0.5.0 (Jul, 2024)
    #[serde(skip_serializing_if = "Option::is_none")]
    use_sqlite_cache: Option<bool>,

    /// Resolve historical manifests from the bucket's git history.
    ///
    /// Upstream's `use_git_history` (`lib/manifest.ps1:211`), which defaults
    /// to `true` there. hok gates only the *read* path with it: the index the
    /// read path consults is harmless derived data, exactly as upstream builds
    /// its cache regardless of the switch.
    #[serde(skip_serializing_if = "Option::is_none")]
    use_git_history: Option<bool>,

    /// Disable `current` version junction creation.
    ///
    /// The 'current' version alias will not be used. Shims and shortcuts will
    /// point to specific version instead.
    ///
    /// This config was introduced in Jan, 2017 with the name `NO_JUNCTIONS`:
    /// <https://github.com/ScoopInstaller/Scoop/commit/a14ffdb5>
    ///
    /// It was renamed to `no_junction` in Aug, 2022 (later in release v0.3.0):
    /// <https://github.com/ScoopInstaller/Scoop/pull/5116>
    #[serde(skip_serializing_if = "Option::is_none")]
    no_junction: Option<bool>,

    /// Stash the local changes that block a bucket update instead of aborting
    /// it (upstream's `autostash_on_conflict`; default `false`).
    ///
    /// Upstream applies this key to its *own* repository update
    /// (`libexec/scoop-update.ps1:113-121`); hok is a single binary with no
    /// self-update, so hok gates the equivalent site instead: the bucket sync
    /// (`internal::git::pull`). The stash is left in place, exactly like
    /// upstream — recovering the changes is `git stash pop`.
    #[serde(skip_serializing_if = "Option::is_none")]
    autostash_on_conflict: Option<bool>,

    /// Continue multi-package operations despite individual package failures.
    #[serde(skip_serializing_if = "Option::is_none")]
    ignore_failures: Option<bool>,

    /// When `true`, install/update/uninstall/reset proceed even if the app
    /// is currently running; only a warning is shown instead of aborting
    /// (matches Scoop's `ignore_running_processes` config).
    #[serde(skip_serializing_if = "Option::is_none")]
    ignore_running_processes: Option<bool>,

    /// A list of private hosts.
    ///
    /// # Note
    ///
    /// Array of private hosts that need additional authentication. For example,
    /// if you want to access a private GitHub repository, you need to add the
    /// host to this list with 'match' and 'headers' strings.
    ///
    /// This config was introduced in Feb, 2021:
    /// <https://github.com/ScoopInstaller/Scoop/pull/4254>
    #[serde(skip_serializing_if = "Option::is_none")]
    private_hosts: Option<Vec<PrivateHosts>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    proxy: Option<String>,

    #[serde(default = "default::root_path")]
    #[serde(skip_serializing_if = "default::is_default_root_path")]
    root_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PrivateHosts {
    /// A string defining the host to match.
    #[serde(rename = "match")]
    match_: String,

    /// A string defining HTTP headers.
    headers: String,
}

impl PrivateHosts {
    /// The host match pattern (regex matched against the request URL).
    pub fn match_pattern(&self) -> &str {
        &self.match_
    }

    /// The HTTP headers string (newline-separated key=value pairs).
    pub fn headers(&self) -> &str {
        &self.headers
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum IsolatedPath {
    /// boolean type of `use_isolated_path`
    Boolean(bool),

    /// string type of `use_isolated_path` indicating the environment variable name
    Named(String),
}

impl FromStr for IsolatedPath {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_ascii_lowercase();

        // `=` is not a valid character in environment variable names
        // ref: https://learn.microsoft.com/en-us/windows/win32/procthread/environment-variables
        if s.contains('=') {
            return Err(Error::ConfigValueInvalid(s));
        }

        match s.as_str() {
            "true" => Ok(IsolatedPath::Boolean(true)),
            "false" => Ok(IsolatedPath::Boolean(false)),
            _ => Ok(IsolatedPath::Named(s)),
        }
    }
}

impl Config {
    /// Get the `cache` directory of Scoop.
    #[inline]
    pub fn cache_path(&self) -> &Path {
        self.cache_path.as_path()
    }

    /// Get the root directory of Scoop.
    ///
    /// This is the root directory of a Scoop installation, by default the value
    /// is `$HOME/scoop`. It may be changed by setting the `SCOOP` environment
    /// variable.
    #[inline]
    pub fn root_path(&self) -> &Path {
        self.root_path.as_path()
    }

    /// Get the root directory of Scoop, or an error when it is unknown.
    ///
    /// Only a stripped environment (no `$SCOOP` and no home directory) leaves
    /// the resolved root empty — see `default::root_path`. Every layout path is
    /// derived from it (`apps`, `shims`, `modules`, `persist`, `cache`), so an
    /// empty root would silently turn them into paths relative to the current
    /// directory. Filesystem-facing entry points must report an error instead
    /// of operating on the current directory.
    pub fn require_root_path(&self) -> Fallible<&Path> {
        let path = self.root_path();
        if path.as_os_str().is_empty() {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "the Scoop root directory is unknown: set the SCOOP environment variable \
                 or a home directory (USERPROFILE/HOME)",
            )));
        }
        Ok(path)
    }

    /// Get the global root directory of Scoop.
    ///
    /// Globally installed apps are stored here. By default the value is
    /// `$env:SCOOP_GLOBAL` or `C:\ProgramData\scoop`.
    #[inline]
    pub fn global_path(&self) -> &Path {
        self.global_path.as_path()
    }

    /// The built-in default root path: `$env:SCOOP`, else `$HOME/scoop`.
    ///
    /// [`Config::root_path`] only reports it while the key is unset; this is
    /// for the reader that must show the current value **and** the default
    /// side by side (`hok config --all`'s default column).
    #[inline]
    pub fn default_root_path() -> PathBuf {
        default::root_path()
    }

    /// The built-in default cache path: `$env:SCOOP_CACHE`, else
    /// `<default root>/cache`.
    #[inline]
    pub fn default_cache_path() -> PathBuf {
        default::cache_path()
    }

    /// The built-in default global path: `$env:SCOOP_GLOBAL`, else
    /// `$ProgramData/scoop`.
    #[inline]
    pub fn default_global_path() -> PathBuf {
        default::global_path()
    }

    /// Get the `no_junction` config.
    #[inline]
    pub fn no_junction(&self) -> bool {
        self.no_junction.unwrap_or_default()
    }

    /// Get the `autostash_on_conflict` config. Defaults to `false` (matching
    /// upstream Scoop): a bucket update blocked by local changes aborts. When
    /// `true`, those changes are stashed (and left stashed) so the update can
    /// proceed.
    #[inline]
    pub fn autostash_on_conflict(&self) -> bool {
        self.autostash_on_conflict.unwrap_or(false)
    }

    /// Get the `ignore_failures` config. Defaults to `false` (matching
    /// upstream Scoop): a package failure aborts the batch unless the
    /// config or the `-f/--ignore-failure` flag opts in to skipping.
    #[inline]
    pub fn ignore_failures(&self) -> bool {
        self.inner.ignore_failures.unwrap_or(false)
    }

    /// Get the `ignore_running_processes` config. Defaults to `false`.
    #[inline]
    pub fn ignore_running_processes(&self) -> bool {
        self.inner.ignore_running_processes.unwrap_or(false)
    }

    /// Get the configured aliases (`alias` field in config).
    #[inline]
    pub fn aliases(&self) -> Option<&std::collections::HashMap<String, String>> {
        self.inner.alias.as_ref()
    }

    /// Set or remove an alias.
    ///
    /// Pure mutation (no file I/O): the caller persists the result via
    /// [`write`].
    pub fn set_alias(&mut self, name: &str, command: Option<&str>) -> Fallible<()> {
        self.inner
            .alias
            .get_or_insert_with(std::collections::HashMap::new);
        let map = self.inner.alias.as_mut().unwrap();
        match command {
            Some(cmd) => {
                map.insert(name.to_string(), cmd.to_string());
            }
            None => {
                map.remove(name);
            }
        }
        Ok(())
    }

    /// Get the `proxy` config.
    #[inline]
    pub fn proxy(&self) -> Option<&str> {
        self.proxy.as_deref()
    }

    /// Get the `PRIVATE_HOSTS` config (private hosts with custom headers).
    #[inline]
    pub fn private_hosts(&self) -> Option<&Vec<PrivateHosts>> {
        self.private_hosts.as_ref()
    }

    /// Get the `cat_style` config.
    #[inline]
    pub fn cat_style(&self) -> &str {
        self.cat_style.as_deref().unwrap_or_default()
    }

    /// Get the `editor` config (the text editor used by `config edit`).
    #[inline]
    pub fn editor(&self) -> Option<&str> {
        self.inner.editor.as_deref()
    }

    /// Get the `language` config ("auto", "en", or "zh").
    #[inline]
    pub fn language(&self) -> &str {
        self.language.as_deref().unwrap_or("auto")
    }

    /// Get the `no_color` config.
    #[inline]
    pub fn no_color(&self) -> bool {
        self.no_color.unwrap_or(false)
    }

    /// Get the `confirm` config (whether the transaction gate shows by default).
    #[inline]
    pub fn confirm(&self) -> bool {
        self.confirm.unwrap_or(false)
    }

    /// Whether package script stages run through the resident PowerShell
    /// engine. Defaults to `false` (each stage spawns a fresh pwsh), matching
    /// Scoop's per-stage spawn and keeping the well-tested path as default.
    #[inline]
    pub fn script_engine(&self) -> bool {
        self.script_engine.unwrap_or(false)
    }

    /// Get the `use_isoloated_path` config.
    #[inline]
    pub fn use_isolated_path(&self) -> Option<&IsolatedPath> {
        self.use_isolated_path.as_ref()
    }

    /// Whether to use SQLite manifest cache.
    #[inline]
    pub fn use_sqlite_cache(&self) -> bool {
        self.inner.use_sqlite_cache.unwrap_or(false)
    }

    /// Whether historical manifests are resolved from the bucket's git
    /// history (upstream's `use_git_history`, on by default).
    #[inline]
    pub fn use_git_history(&self) -> bool {
        self.inner.use_git_history.unwrap_or(true)
    }

    /// The user-configured default architecture, if any.
    ///
    /// Maps to Scoop's `default_architecture` config; when set it overrides
    /// the runtime-detected host architecture.
    #[inline]
    pub fn default_architecture(&self) -> Option<&str> {
        self.inner.default_architecture.as_deref()
    }

    /// Get the `last_update` config: Scoop's last bucket-update timestamp, as
    /// stored (upstream sets it in `lib/core.ps1`'s update check).
    #[inline]
    pub fn last_update(&self) -> Option<&str> {
        self.inner.last_update.as_deref()
    }

    /// Whether Aria2 (or equivalent curl fragmentation) is enabled.
    /// Defaults to `true` if not set.
    #[inline]
    pub fn aria2_enabled(&self) -> bool {
        self.inner.aria2_enabled.unwrap_or(true)
    }

    /// Number of connections per server for fragmented download.
    /// Maps from `aria2-split`. Defaults to 5.
    #[inline]
    pub fn aria2_split(&self) -> u32 {
        self.inner.aria2_split.unwrap_or(5).max(1)
    }

    /// Concurrent package downloads (cross-package) for the download phase
    /// (HEAD sizing, download, verify). Maps from `jobs`. Defaults to 4.
    #[inline]
    pub fn jobs(&self) -> u32 {
        self.inner.jobs.unwrap_or(4).max(1)
    }

    /// Max connections per server (cap for fragmentation count).
    /// Maps from `aria2-max-connection-per-server`. Defaults to 5.
    #[inline]
    pub fn aria2_max_connection_per_server(&self) -> u32 {
        self.inner
            .aria2_max_connection_per_server
            .unwrap_or(5)
            .max(1)
    }

    /// Minimum file size to trigger fragmented download.
    /// Maps from `aria2-min-split-size`. Parses strings like "5M", "10M".
    /// Returns bytes. Defaults to 20MB.
    pub fn aria2_min_split_size(&self) -> u64 {
        const DEFAULT: u64 = 20 * 1024 * 1024;
        let raw = match self.inner.aria2_min_split_size.as_deref() {
            Some(s) => s,
            None => return DEFAULT,
        };
        let raw = raw.trim().to_lowercase();
        let (num_str, multiplier) = if raw.ends_with('g') {
            (&raw[..raw.len() - 1], 1024u64 * 1024 * 1024)
        } else if raw.ends_with('m') {
            (&raw[..raw.len() - 1], 1024u64 * 1024)
        } else if raw.ends_with('k') {
            (&raw[..raw.len() - 1], 1024u64)
        } else {
            (raw.as_str(), 1u64)
        };
        num_str
            .parse::<u64>()
            .ok()
            .map(|v| v * multiplier)
            .unwrap_or(DEFAULT)
    }

    /// Update a config key with a new value.
    ///
    /// Pure mutation (no file I/O): the caller persists the result via
    /// [`write`].
    pub(crate) fn set(&mut self, key: &str, value: &str) -> Fallible<()> {
        let is_unset = value.is_empty();
        // Resolve the spelling once: `set`, the read path and the rejection
        // message all go through the same table, so a new key cannot drift
        // away from the field it writes.
        let Some(canonical) = canonical_key(key) else {
            return Err(if is_official_only_key(key) {
                Error::ConfigKeyUnsupported(key.to_owned())
            } else {
                Error::ConfigKeyInvalid(key.to_owned())
            });
        };
        match canonical {
            "aria2-enabled" => match is_unset {
                true => self.inner.aria2_enabled = None,
                false => match value.parse::<bool>() {
                    Ok(value) => self.inner.aria2_enabled = Some(value),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "cat_style" => {
                self.inner.cat_style = match is_unset {
                    true => None,
                    false => Some(value.to_string()),
                }
            }
            "confirm" => match is_unset {
                true => self.inner.confirm = None,
                false => match value.parse::<bool>() {
                    Ok(v) => self.inner.confirm = Some(v),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "script_engine" => match is_unset {
                true => self.inner.script_engine = None,
                false => match value.parse::<bool>() {
                    Ok(v) => self.inner.script_engine = Some(v),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "editor" => {
                self.inner.editor = match is_unset {
                    true => None,
                    false => Some(value.to_string()),
                }
            }
            "ignore_failures" => match is_unset {
                true => self.inner.ignore_failures = None,
                false => match value.parse::<bool>() {
                    Ok(v) => self.inner.ignore_failures = Some(v),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "ignore_running_processes" => match is_unset {
                true => self.inner.ignore_running_processes = None,
                false => match value.parse::<bool>() {
                    Ok(v) => self.inner.ignore_running_processes = Some(v),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "gh_token" => {
                self.inner.gh_token = match is_unset {
                    true => None,
                    false => Some(value.to_string()),
                }
            }
            "last_update" => {
                self.inner.last_update = match is_unset {
                    true => None,
                    false => Some(value.to_string()),
                }
            }
            "use_isolated_path" => match is_unset {
                true => self.inner.use_isolated_path = None,
                false => match value.parse::<IsolatedPath>() {
                    Ok(value) => self.inner.use_isolated_path = Some(value),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "default_architecture" => match is_unset {
                true => self.inner.default_architecture = None,
                false => match crate::internal::arch::Arch::parse(value) {
                    Ok(_) => self.inner.default_architecture = Some(value.to_owned()),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "use_sqlite_cache" => match is_unset {
                true => self.inner.use_sqlite_cache = None,
                false => match value.parse::<bool>() {
                    Ok(value) => self.inner.use_sqlite_cache = Some(value),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "use_git_history" => match is_unset {
                true => self.inner.use_git_history = None,
                false => match value.parse::<bool>() {
                    Ok(value) => self.inner.use_git_history = Some(value),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "proxy" => match value {
                "" | "none" => self.inner.proxy = None,
                _ => self.inner.proxy = Some(value.to_string()),
            },
            "no_junction" => match is_unset {
                true => self.inner.no_junction = None,
                false => match value.parse::<bool>() {
                    Ok(v) => self.inner.no_junction = Some(v),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "autostash_on_conflict" => match is_unset {
                true => self.inner.autostash_on_conflict = None,
                false => match value.parse::<bool>() {
                    Ok(v) => self.inner.autostash_on_conflict = Some(v),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "no_color" => match is_unset {
                true => self.inner.no_color = None,
                false => match value.parse::<bool>() {
                    Ok(v) => self.inner.no_color = Some(v),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "aria2-split" => match is_unset {
                true => self.inner.aria2_split = None,
                false => match value.parse::<u32>() {
                    Ok(v) => self.inner.aria2_split = Some(v),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "jobs" => match is_unset {
                true => self.inner.jobs = None,
                false => match value.parse::<u32>() {
                    Ok(v) => self.inner.jobs = Some(v),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "aria2-max-connection-per-server" => match is_unset {
                true => self.inner.aria2_max_connection_per_server = None,
                false => match value.parse::<u32>() {
                    Ok(v) => self.inner.aria2_max_connection_per_server = Some(v),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "aria2-min-split-size" => {
                self.inner.aria2_min_split_size = match is_unset {
                    true => None,
                    false => Some(value.to_string()),
                }
            }
            "language" => {
                self.inner.language = match is_unset {
                    true => None,
                    false => Some(value.to_string()),
                }
            }
            "virustotal_api_key" => {
                self.inner.virustotal_api_key = match is_unset {
                    true => None,
                    false => Some(value.to_string()),
                }
            }
            "root_path" => {
                self.inner.root_path = if is_unset {
                    default::root_path()
                } else {
                    PathBuf::from(value)
                }
            }
            "global_path" => {
                self.inner.global_path = if is_unset {
                    default::global_path()
                } else {
                    PathBuf::from(value)
                }
            }
            "cache_path" => {
                self.inner.cache_path = if is_unset {
                    default::cache_path()
                } else {
                    PathBuf::from(value)
                }
            }
            // [`canonical_key`] only yields the arms above; this arm keeps the
            // compiler happy and a future table entry without a body honest.
            _ => return Err(Error::ConfigKeyInvalid(key.to_owned())),
        }

        Ok(())
    }

    /// Pretty print the config
    pub(crate) fn pretty(&self) -> Fallible<String> {
        Ok(serde_json::to_string_pretty(&self.inner)?)
    }
}

impl Default for Config {
    fn default() -> Self {
        let inner = ConfigInner {
            alias: Default::default(),
            aria2_enabled: Default::default(),
            aria2_max_connection_per_server: Default::default(),
            aria2_min_split_size: Default::default(),
            aria2_split: Default::default(),
            jobs: Default::default(),
            // default_cache_path: default::cache_path(),
            cache_path: default::cache_path(),
            cat_style: Default::default(),
            confirm: Default::default(),
            script_engine: Default::default(),
            editor: Default::default(),
            default_architecture: Default::default(),
            gh_token: Default::default(),
            virustotal_api_key: Default::default(),
            // default_global_path: default::global_path(),
            global_path: default::global_path(),
            ignore_failures: Default::default(),
            ignore_running_processes: Default::default(),
            last_update: Default::default(),
            use_isolated_path: Default::default(),
            use_sqlite_cache: Default::default(),
            use_git_history: Default::default(),
            no_junction: Default::default(),
            autostash_on_conflict: Default::default(),
            language: Default::default(),
            no_color: Default::default(),
            private_hosts: Default::default(),
            proxy: Default::default(),
            // default_root_path: default::root_path(),
            root_path: default::root_path(),
        };
        Config {
            path: default::hok_config_path(),
            inner,
        }
    }
}

impl Deref for Config {
    type Target = ConfigInner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// Initialize the default config, writing it to the config path best-effort
/// (a fresh install gets a config file on first run).
///
/// The persistence boundary for [`Config::default`]; called once at session
/// creation. The [`Config`] type itself performs no file I/O.
pub(crate) fn init_default() -> Config {
    let config = Config::default();
    // try to write the default config to the default path, error is ignored
    //
    // A stripped environment (no `$HOME`/`USERPROFILE`) resolves the default
    // path to an empty one — see `default::hok_config_path`. There is nowhere
    // to write then, so the best-effort write is skipped: the session keeps
    // the in-memory defaults instead of failing (or, formerly, panicking).
    if !config.path.as_os_str().is_empty() {
        let _ = write(&config);
    }
    config
}

/// Write the config back to its file (`~/.config/hok/config.json`).
///
/// The file-I/O boundary for the pure [`Config`] type: the mutation methods
/// (`set`, `set_alias`) only update in-memory state — callers persist
/// explicitly with this function.
pub(crate) fn write(config: &Config) -> Fallible<()> {
    internal::fs::write_json(&config.path, &config.inner)
}

/// Get a list of possible config paths.
///
/// There are 3 possible locations for the `config.json` file:
///   1) Side-by-side with the real executable (symlink resolved);
///   2) Located in the `root` directory of Scoop;
///   3) Located in the XDG_CONFIG_HOME directory.
pub(crate) fn possible_config_paths() -> Vec<PathBuf> {
    let mut ret = vec![];

    if let Ok(exe_path) = std::env::current_exe()
        && let Ok(metadata) = std::fs::symlink_metadata(&exe_path)
    {
        let is_symlink = metadata.is_symlink();
        let mut path = exe_path.clone();

        if is_symlink {
            // since the executable is a symlink, we can use `read_link`
            // to get the real path of the executable
            if let Ok(real_path) = std::fs::read_link(&exe_path) {
                path = real_path;
            }
        }

        path.pop();
        path.push("config.json");

        // 1) config.json side-by-side with the real executable
        ret.push(path.clone());

        // this pop is ok, it removes `config.json` we just pushed
        // <app>\<current>\<app_name>\apps\<root> (in theory)
        //       ^^^^^^^^^
        path.pop();
        // <app>\<current>\<app_name>\apps\<root> (in theory)
        //                 ^^^^^^^^^^
        if path.pop() {
            // <app>\<current>\<app_name>\apps\<root> (in theory)
            //                            ^^^^
            if path.pop() {
                let check = internal::path::leaf(&path)
                    .map(|n| n == "apps")
                    .unwrap_or_default();
                // <app>\<current>\<app_name>\apps\<root> (in theory)
                //                                 ^^^^^
                if check && path.pop() {
                    path.push("config.json");

                    // 2) config.json located in the `root` directory of
                    // Scoop, i.e., the portable config.json
                    ret.push(path);
                }
            }
        }
    }

    // 3) config.json located in the XDG_CONFIG_HOME directory, i.e.,
    // `~/.config/scoop/config.json`
    ret.push(default::scoop_config_path());

    ret
}

/// This private module contains functions of constructing default paths used
/// to create the default Scoop `Config`, with system's environment variables.
mod default {
    use std::path::{Path, PathBuf};

    use crate::internal::path::{home_dir, normalize_path};

    /// Read an environment variable as a path, treating an empty value as
    /// unset.
    ///
    /// Mirrors upstream's candidate chain
    /// (`$env:SCOOP, (get_config ROOT_PATH), ..., | Where-Object { $_ } |
    /// Select-Object -First 1`, `lib/core.ps1:1291-1301`), where an empty
    /// variable is falsy and falls through to the next candidate — and matches
    /// the empty-value filtering in `internal::path::{home_dir, config_dir}`.
    #[inline]
    fn env_path(name: &str) -> Option<PathBuf> {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }

    /// Join the given `path` to `$HOME` and return a new [`PathBuf`].
    ///
    /// Falls back to an empty path when `$HOME`/`USERPROFILE` is unset
    /// (service accounts, stripped environments); the config load chain
    /// then reverts to defaults instead of panicking.
    #[inline]
    fn home_join<P: AsRef<Path>>(path: P) -> PathBuf {
        home_dir()
            .map(|p| p.join(path.as_ref()))
            .unwrap_or_default()
    }

    /// Get the default Scoop config path: `$HOME/.config/scoop/config.json`.
    ///
    /// This is the *read-only compatibility source*: Scoop's own config file,
    /// used only for the first-run migration of supported keys into hok's
    /// own config file.
    #[inline]
    pub(super) fn scoop_config_path() -> PathBuf {
        normalize_path(home_join(".config/scoop/config.json"))
    }

    /// Get the default hok config path: `$HOME/.config/hok/config.json`.
    ///
    /// hok's own config file — the only file hok ever writes to. It stores
    /// only the config keys hok actually supports.
    #[inline]
    pub(super) fn hok_config_path() -> PathBuf {
        normalize_path(home_join(".config/hok/config.json"))
    }

    /// Get the default Scoop root path.
    ///
    /// `$SCOOP` wins, then `$HOME/scoop` — an empty `$SCOOP` counts as unset
    /// (see [`env_path`]).
    #[inline]
    pub(super) fn root_path() -> PathBuf {
        let path = env_path("SCOOP").unwrap_or_else(|| home_join("scoop"));

        normalize_path(path)
    }

    /// Get the default Scoop cache path.
    ///
    /// `$SCOOP_CACHE` wins, then `<root>/cache` — an empty `$SCOOP_CACHE`
    /// counts as unset (see [`env_path`]).
    #[inline]
    pub(super) fn cache_path() -> PathBuf {
        let path = env_path("SCOOP_CACHE").unwrap_or_else(|| root_path().join("cache"));

        normalize_path(path)
    }

    /// Get the default Scoop global path.
    ///
    /// `$SCOOP_GLOBAL` wins, then `$ProgramData/scoop`, then
    /// `C:/ProgramData/scoop` — an empty `$SCOOP_GLOBAL`/`$ProgramData` counts
    /// as unset (see [`env_path`]), so this never resolves to an empty path.
    #[inline]
    pub(super) fn global_path() -> PathBuf {
        let path = if let Some(path) = env_path("SCOOP_GLOBAL") {
            return path;
        } else {
            env_path("ProgramData")
                .map(|p| p.join("scoop"))
                .unwrap_or(PathBuf::from("C:/ProgramData/scoop"))
        };

        normalize_path(path)
    }

    /// Check if the given `path` is equal to the `default` one.
    #[inline]
    fn is_default(default: &Path, path: &Path) -> bool {
        path.eq(default)
    }

    /// Generate an `is_default_*` accessor that compares a path to a default path function.
    macro_rules! is_default_accessor {
        ($name:ident, $path_fn:ident) => {
            #[inline]
            pub(super) fn $name<P: AsRef<Path>>(path: P) -> bool {
                is_default($path_fn().as_path(), path.as_ref())
            }
        };
    }

    is_default_accessor!(is_default_root_path, root_path);
    is_default_accessor!(is_default_cache_path, cache_path);
    is_default_accessor!(is_default_global_path, global_path);
}

// ─── Session operations ────────────────────────────────────────────────────

/// Get the configuration list.
///
/// # Returns
///
/// A string of the configuration list in pretty-printed JSON format.
///
/// # Errors
///
/// Serde errors will be returned if the config cannot be serialized.
pub fn list(session: &crate::Session) -> Fallible<String> {
    let config = session.config();
    config.pretty()
}

/// Every setting `config` accepts, under the one spelling it accepts it by.
///
/// There are no aliases: the spelling is the key's `serde` name in
/// [`ConfigInner`] — which is also the name the config file holds, and, for a
/// setting upstream shares, the name upstream's own `get_config` calls read
/// (`libexec/scoop-config.ps1:21-153`) — so an alternate spelling
/// (`aria2_split`, `no_junctions`, `ignore_failure`, ...) is refused like any
/// other unknown key.
///
/// [`Config::set`], [`value`] and the rejection message all resolve through this
/// one list, so a setting cannot drift out of the CLI's vocabulary.
const SETTINGS: &[&str] = &[
    // Upstream's aria2 family, hyphenated there too.
    "aria2-enabled",
    "aria2-max-connection-per-server",
    "aria2-min-split-size",
    "aria2-split",
    // Shared with upstream, under upstream's spelling.
    "autostash_on_conflict",
    "cache_path",
    "cat_style",
    "default_architecture",
    "gh_token",
    "global_path",
    "ignore_running_processes",
    "last_update",
    "no_junction",
    "proxy",
    "root_path",
    "use_git_history",
    "use_isolated_path",
    "use_sqlite_cache",
    "virustotal_api_key",
    // hok's own settings (upstream has no such key).
    "confirm",
    "editor",
    "ignore_failures",
    "jobs",
    "language",
    "no_color",
    "script_engine",
];

/// The canonical spelling of `key`, or `None` when `key` is not a setting.
pub(crate) fn canonical_key(key: &str) -> Option<&'static str> {
    SETTINGS.iter().copied().find(|name| *name == key)
}

/// The value a setting currently holds in the config **file**, as stored — not
/// the resolved default (`hok config --all`'s default column carries those, and
/// the difference matters: `scoop config aria2-split` answers "not set" while
/// the downloader still uses the built-in 5).
///
/// Returns `None` when the key is unset, when it is not a setting hok writes
/// (upstream's `scoop config <name>` answers the same way for a key it does not
/// know), and for a path key left at its built-in default — those are not
/// serialized, exactly as upstream's `get_config` returns `$null`.
///
/// # Errors
///
/// Returns an error if the config is currently borrowed elsewhere or cannot be
/// serialized.
pub fn value(session: &crate::Session, key: &str) -> Fallible<Option<serde_json::Value>> {
    // A key outside the table is looked up verbatim: `private_hosts` and the
    // legacy `alias` index are file-only settings that read back but are not
    // writable from the CLI.
    let name = canonical_key(key).unwrap_or(key);
    // Re-reading hok's own serialization goes through the shared JSON entry
    // point like every other read (`internal::json`), not a bare `serde_json`.
    let document: serde_json::Value = internal::json::parse_str(&list(session)?)?;
    Ok(document.get(name).cloned())
}

/// Settings that exist in **both** configs under the same name.
///
/// `hok export -c` emits exactly these: upstream's own `-c` dumps `$scoopConfig`
/// minus the machine-specific paths (`libexec/scoop-export.ps1:13`). hok-only
/// settings (`confirm`, `editor`, `jobs`, ...) stay out of upstream's section,
/// and the upstream-only ones hok has no value for cannot be emitted at all.
const SHARED_KEYS: &[&str] = &[
    "aria2-enabled",
    "aria2-max-connection-per-server",
    "aria2-min-split-size",
    "aria2-split",
    "autostash_on_conflict",
    "cat_style",
    "default_architecture",
    "gh_token",
    "ignore_running_processes",
    "no_junction",
    "private_hosts",
    "proxy",
    "use_git_history",
    "use_isolated_path",
    "use_sqlite_cache",
    "virustotal_api_key",
];

/// The upstream name of a setting hok stores under `hok_key`, or `None` when
/// upstream has no such setting.
pub fn official_name(hok_key: &str) -> Option<&'static str> {
    SHARED_KEYS.iter().copied().find(|name| *name == hok_key)
}

/// How an upstream config key (`scoop export -c`, `scoop config <name>`) maps
/// onto hok's config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfficialKey {
    /// Write the value under this hok key.
    Supported(&'static str),
    /// A path (or the alias index) that only makes sense on the machine it was
    /// exported from; upstream's own `-c` strips these.
    MachineSpecific,
    /// hok has no equivalent setting.
    Unsupported,
}

/// Upstream keys hok must never import: the paths (and the alias index) only
/// make sense on the machine that exported them, and upstream's own `-c`
/// strips them (`libexec/scoop-export.ps1:13`).
const MACHINE_SPECIFIC_KEYS: &[&str] = &[
    "alias",
    "cache_path",
    "global_path",
    "last_update",
    "root_path",
];

/// Map an upstream config key onto hok's config.
///
/// Upstream stores a key exactly as the user typed it (`set_config` only
/// lower-cases it, `libexec/scoop-config.ps1:112`) and reads it back through
/// `get_config NAME`, so the canonical upstream spellings are the ones its own
/// calls use: underscores for everything but the `aria2-*` family, which is
/// hyphenated (`libexec/scoop-config.ps1:21-153`). Only those spellings are
/// recognized — the same strictness `config` itself applies.
pub fn map_official_key(key: &str) -> OfficialKey {
    let key = key.to_ascii_lowercase();
    // Checked before the settings: a path captured on the exporting machine
    // (hok records them in its `--hok` extension) must not clobber this one's.
    if MACHINE_SPECIFIC_KEYS.contains(&key.as_str()) {
        return OfficialKey::MachineSpecific;
    }
    canonical_key(&key).map_or(OfficialKey::Unsupported, OfficialKey::Supported)
}

/// Upstream settings hok deliberately has no equivalent for.
///
/// The list exists so a refusal can say *why*: an upstream setting hok does not
/// implement is a different answer from a key that is not a setting at all (and
/// `hok import` reports the two apart for the same reason). Upstream's own
/// settings list is `libexec/scoop-config.ps1:21-153`.
const OFFICIAL_ONLY_KEYS: &[&str] = &[
    "aria2-fallback-enabled",
    "aria2-options",
    "aria2-retry-wait",
    "aria2-warning-enabled",
    "debug",
    "force_update",
    "hold_update_until",
    "scoop_branch",
    "scoop_repo",
    "shim",
    "show_manifest",
    "show_update_log",
    "update_nightly",
    "use_external_7zip",
    "use_lessmsi",
];

/// Whether `key` names an upstream setting hok has no equivalent for, under
/// upstream's own spelling (case aside).
fn is_official_only_key(key: &str) -> bool {
    OFFICIAL_ONLY_KEYS
        .iter()
        .any(|name| name.eq_ignore_ascii_case(key))
}

/// Set a configuration key. *
///
/// # Errors
///
/// A [`ConfigInUse`][1] error will be returned if the config is borrowed
/// elsewhere.
///
/// A [`ConfigKeyInvalid`][2] error will be returned if the key is invalid, or
/// [`ConfigKeyUnsupported`][4] when it is an upstream setting hok has no
/// equivalent for.
///
/// A [`ConfigValueInvalid`][3] error will be returned if the value is invalid.
///
/// [1]: crate::Error::ConfigInUse
/// [2]: crate::Error::ConfigKeyInvalid
/// [3]: crate::Error::ConfigValueInvalid
/// [4]: crate::Error::ConfigKeyUnsupported
pub fn set(session: &crate::Session, key: &str, value: &str) -> Fallible<()> {
    {
        let mut config = session.config_mut()?;
        config.set(key, value)?;
    }
    write(&session.config())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_session(name: &str) -> crate::Session {
        let root = crate::test_utils::tmpdir(name);
        crate::test_utils::test_session(&root)
    }

    fn bool_field(config: &ConfigInner, key: &str) -> Option<bool> {
        match key {
            "no_junction" => config.no_junction,
            "autostash_on_conflict" => config.autostash_on_conflict,
            "no_color" => config.no_color,
            "ignore_running_processes" => config.ignore_running_processes,
            "script_engine" => config.script_engine,
            _ => panic!("not a bool key: {key}"),
        }
    }

    fn u32_field(config: &ConfigInner, key: &str) -> Option<u32> {
        match key {
            "aria2-split" => config.aria2_split,
            "aria2-max-connection-per-server" => config.aria2_max_connection_per_server,
            "jobs" => config.jobs,
            _ => panic!("not a u32 key: {key}"),
        }
    }

    fn string_field<'a>(config: &'a ConfigInner, key: &str) -> Option<&'a str> {
        match key {
            "aria2-min-split-size" => config.aria2_min_split_size.as_deref(),
            "language" => config.language.as_deref(),
            "virustotal_api_key" => config.virustotal_api_key.as_deref(),
            "editor" => config.editor.as_deref(),
            _ => panic!("not a string key: {key}"),
        }
    }

    /// Every setting `config` accepts must round-trip: set a value, verify it
    /// landed, then unset and verify it cleared.
    #[test]
    fn set_unset_roundtrip() {
        let session = test_session("config_set_roundtrip");

        // bool keys
        for key in [
            "no_junction",
            "autostash_on_conflict",
            "no_color",
            "ignore_running_processes",
            "script_engine",
        ] {
            crate::config::set(&session, key, "true").unwrap();
            assert_eq!(bool_field(&session.config().inner, key), Some(true));
            crate::config::set(&session, key, "false").unwrap();
            assert_eq!(bool_field(&session.config().inner, key), Some(false));
            crate::config::set(&session, key, "").unwrap();
            assert_eq!(bool_field(&session.config().inner, key), None);
        }

        // u32 keys
        for key in ["aria2-split", "aria2-max-connection-per-server", "jobs"] {
            crate::config::set(&session, key, "7").unwrap();
            assert_eq!(u32_field(&session.config().inner, key), Some(7));
            crate::config::set(&session, key, "").unwrap();
            assert_eq!(u32_field(&session.config().inner, key), None);
        }

        // string keys
        for key in [
            "aria2-min-split-size",
            "language",
            "virustotal_api_key",
            "editor",
        ] {
            crate::config::set(&session, key, "value").unwrap();
            assert_eq!(string_field(&session.config().inner, key), Some("value"));
            crate::config::set(&session, key, "").unwrap();
            assert_eq!(string_field(&session.config().inner, key), None);
        }

        // path keys: unset restores the default
        for key in ["root_path", "global_path", "cache_path"] {
            crate::config::set(&session, key, "C:\\custom\\path").unwrap();
            {
                let config = &session.config().inner;
                let field = match key {
                    "root_path" => &config.root_path,
                    "global_path" => &config.global_path,
                    _ => &config.cache_path,
                };
                assert_eq!(field, &PathBuf::from("C:\\custom\\path"));
            }
            crate::config::set(&session, key, "").unwrap();
            {
                let config = &session.config().inner;
                let field = match key {
                    "root_path" => &config.root_path,
                    "global_path" => &config.global_path,
                    _ => &config.cache_path,
                };
                let expected = match key {
                    "root_path" => default::root_path(),
                    "global_path" => default::global_path(),
                    _ => default::cache_path(),
                };
                assert_eq!(field, &expected);
            }
        }
    }

    /// Every setting reads back through [`crate::config::value`] under its own
    /// spelling, an unknown key answers `None` (upstream `scoop config <name>`),
    /// and an alternate spelling is refused like any other unknown key.
    #[test]
    fn value_reads_back_every_setting() {
        use serde_json::json;
        let session = test_session("config_value_read");
        let cases: Vec<(&str, &str, serde_json::Value)> = vec![
            ("aria2-enabled", "true", json!(true)),
            ("aria2-split", "7", json!(7)),
            ("aria2-max-connection-per-server", "9", json!(9)),
            ("aria2-min-split-size", "5M", json!("5M")),
            ("script_engine", "true", json!(true)),
            ("ignore_failures", "true", json!(true)),
            ("no_junction", "true", json!(true)),
            ("autostash_on_conflict", "true", json!(true)),
            ("no_color", "true", json!(true)),
        ];
        for (key, value, expected) in cases {
            crate::config::set(&session, key, value).unwrap();
            assert_eq!(
                crate::config::value(&session, key).unwrap(),
                Some(expected),
                "{key} after setting it to {value}"
            );
            crate::config::set(&session, key, "").unwrap();
            assert_eq!(
                crate::config::value(&session, key).unwrap(),
                None,
                "{key} after unset"
            );
        }

        // An alternate spelling is not a setting: `set` refuses it instead of
        // silently writing the field behind the canonical key ...
        for alias in [
            "aria2_split",
            "no_junctions",
            "no-color",
            "script-engine",
            "ignore-failures",
            "ignore_failure",
        ] {
            let err = crate::config::set(&session, alias, "true").unwrap_err();
            assert!(
                matches!(err, Error::ConfigKeyInvalid(_)),
                "{alias}: unexpected error {err}"
            );
        }

        // ... and an unknown key reads as unset rather than erroring, like
        // `scoop config <unknown>`.
        assert_eq!(
            crate::config::value(&session, "totally_unknown").unwrap(),
            None
        );
    }

    /// Invalid values for typed keys are rejected; unknown keys are rejected.
    #[test]
    fn invalid_values_and_keys_rejected() {
        let session = test_session("config_set_invalid");
        let err = crate::config::set(&session, "no_junction", "notabool").unwrap_err();
        assert!(matches!(err, Error::ConfigValueInvalid(_)));
        let err = crate::config::set(&session, "ignore_running_processes", "notabool").unwrap_err();
        assert!(matches!(err, Error::ConfigValueInvalid(_)));
        let err = crate::config::set(&session, "aria2-split", "abc").unwrap_err();
        assert!(matches!(err, Error::ConfigValueInvalid(_)));
        let err = crate::config::set(&session, "totally_unknown_key", "x").unwrap_err();
        assert!(matches!(err, Error::ConfigKeyInvalid(_)));
    }

    /// `autostash_on_conflict` is a supported setting now: it round-trips
    /// through `set` and `value`, defaults to `false`, and is no longer
    /// answered as an upstream-only key.
    #[test]
    fn autostash_on_conflict_is_supported_and_round_trips() {
        use serde_json::json;
        let session = test_session("config_autostash");

        // The key left the "upstream setting hok has no equivalent for" list,
        // so an import from Scoop maps it instead of refusing it.
        assert_eq!(
            crate::config::map_official_key("autostash_on_conflict"),
            OfficialKey::Supported("autostash_on_conflict")
        );

        // Default is `false`: `set` accepts the key and the runtime getter
        // starts out off.
        assert!(!session.config().autostash_on_conflict());

        crate::config::set(&session, "autostash_on_conflict", "true").unwrap();
        assert_eq!(
            crate::config::value(&session, "autostash_on_conflict").unwrap(),
            Some(json!(true))
        );
        assert!(session.config().autostash_on_conflict());

        crate::config::set(&session, "autostash_on_conflict", "").unwrap();
        assert_eq!(
            crate::config::value(&session, "autostash_on_conflict").unwrap(),
            None
        );
        assert!(!session.config().autostash_on_conflict());
    }

    /// Scoop-only settings that hok does not support must be refused by the CLI
    /// setter, with the "upstream setting, no hok equivalent" error rather than
    /// the generic unknown-key one (they can still be read from Scoop's
    /// read-only file, but never written to hok's own file).
    #[test]
    fn unsupported_keys_rejected() {
        let session = test_session("config_unsupported");
        for key in [
            "use_external_7zip",
            "use_lessmsi",
            "scoop_repo",
            "scoop_branch",
            "show_update_log",
            "show_manifest",
            "aria2-warning-enabled",
            "aria2-retry-wait",
            "aria2-options",
            "aria2-fallback-enabled",
            "debug",
            "force_update",
            "shim",
        ] {
            let err = crate::config::set(&session, key, "x").unwrap_err();
            assert!(
                matches!(err, Error::ConfigKeyUnsupported(_)),
                "{key}: unexpected error {err}"
            );
        }

        // A key that is not a setting at all keeps the plain unknown-key error.
        let err = crate::config::set(&session, "totally_unknown_key", "x").unwrap_err();
        assert!(
            matches!(err, Error::ConfigKeyInvalid(_)),
            "unexpected error {err}"
        );
    }

    /// First run migrates supported keys from Scoop's read-only file into a
    /// new hok config file; unsupported keys are not migrated.
    #[test]
    fn first_run_migrates_supported_keys_from_scoop() {
        let root = crate::test_utils::tmpdir("config_migrate");
        let scoop = root.join("scoop.json");
        let hok = root.join("hok.json");
        std::fs::write(
            &scoop,
            r#"{"root_path": "C:\\scoop", "proxy": "scoop-proxy", "use_external_7zip": true}"#,
        )
        .unwrap();

        let config = ConfigBuilder::new()
            .path(&hok)
            .scoop_path(&scoop)
            .load()
            .unwrap();
        // supported keys migrated
        assert_eq!(config.proxy(), Some("scoop-proxy"));
        assert_eq!(config.root_path(), Path::new("C:\\scoop"));
        // hok's file was created with only supported keys: the Scoop-only
        // `use_external_7zip` key is silently dropped, not migrated
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&hok).unwrap()).unwrap();
        let obj = written.as_object().unwrap();
        assert!(obj.contains_key("proxy"));
        assert!(!obj.contains_key("use_external_7zip"));
    }

    /// Once hok's own file exists, Scoop's file is no longer consulted.
    #[test]
    fn existing_hok_file_ignores_scoop() {
        let root = crate::test_utils::tmpdir("config_existing_hok");
        let scoop = root.join("scoop.json");
        let hok = root.join("hok.json");
        std::fs::write(
            &scoop,
            r#"{"proxy": "scoop-proxy", "use_external_7zip": true}"#,
        )
        .unwrap();
        std::fs::write(&hok, r#"{"proxy": "hok-proxy"}"#).unwrap();

        let config = ConfigBuilder::new()
            .path(&hok)
            .scoop_path(&scoop)
            .load()
            .unwrap();
        // hok's file wins; Scoop's file is not even consulted
        assert_eq!(config.proxy(), Some("hok-proxy"));
    }

    /// Loading fails when neither hok's nor Scoop's config file exists.
    #[test]
    fn load_fails_when_no_config_exists() {
        let root = crate::test_utils::tmpdir("config_missing");
        let err = ConfigBuilder::new()
            .path(root.join("hok.json"))
            .load()
            .unwrap_err();
        assert!(matches!(err, Error::Io(_)));
    }

    /// A hand-edited config saved with a UTF-8 BOM loads fine.
    #[test]
    fn load_tolerates_bom_config() {
        let root = crate::test_utils::tmpdir("config_bom");
        let path = root.join("hok.json");
        std::fs::write(&path, format!("\u{FEFF}{}", r#"{"proxy": "bom-proxy"}"#)).unwrap();
        let config = ConfigBuilder::new().path(&path).load().unwrap();
        assert_eq!(config.proxy(), Some("bom-proxy"));
    }

    /// Writing (commit) writes the whole `ConfigInner`, which by construction
    /// contains only hok-supported keys.
    #[test]
    fn commit_writes_only_supported_keys() {
        let root = crate::test_utils::tmpdir("config_write_filter");
        let hok = root.join("hok.json");
        std::fs::write(&hok, "{}").unwrap();
        let mut config = ConfigBuilder::new().path(&hok).load().unwrap();
        config.inner.proxy = Some("p".to_string());
        config.inner.no_junction = Some(true);
        write(&config).unwrap();

        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&hok).unwrap()).unwrap();
        let obj = written.as_object().unwrap();
        assert_eq!(obj.get("proxy").and_then(|v| v.as_str()), Some("p"));
        assert_eq!(obj.get("no_junction").and_then(|v| v.as_bool()), Some(true));
        // no Scoop-only keys leak into the written file
        for unsupported in ["use_external_7zip", "show_manifest", "debug"] {
            assert!(
                !obj.contains_key(unsupported),
                "unsupported key {unsupported} must not be written"
            );
        }
    }

    /// A config whose path is empty (no home directory to resolve
    /// `default::hok_config_path`) cannot be persisted: the write reports an
    /// error instead of panicking. This is the premise of `init_default`'s
    /// skip-the-write branch.
    #[test]
    fn write_refuses_a_config_without_a_path() {
        let config = Config {
            path: PathBuf::new(),
            ..Config::default()
        };
        let err = write(&config).unwrap_err();
        assert!(matches!(err, Error::Io(_)), "got: {err:?}");
        assert!(
            err.to_string().contains("no parent directory"),
            "unexpected message: {err}"
        );
    }

    /// An unknown Scoop root (no `$SCOOP`, no home directory) is reported
    /// instead of being returned as an empty path that would resolve every
    /// layout directory against the current directory.
    #[test]
    fn require_root_path_rejects_an_empty_root() {
        let mut config = Config::default();

        config.inner.root_path = PathBuf::new();
        let err = config.require_root_path().unwrap_err();
        assert!(matches!(err, Error::Io(_)), "got: {err:?}");
        assert!(
            err.to_string().contains("Scoop root directory is unknown"),
            "unexpected message: {err}"
        );

        config.inner.root_path = PathBuf::from(r"C:\scoop");
        assert_eq!(config.require_root_path().unwrap(), Path::new(r"C:\scoop"));
    }
}
