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
//! - **Defaults**: If no config file exists, `Config::init()` creates a
//!   blank hok config with sensible defaults.
//!
//! # Thread safety
//!
//! `Config` is typically accessed via `Session::config()` which returns a
//! `Ref<Config>`. For mutation, `Session::config_mut()` provides `RefMut`.
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
        if let Some(scoop_path) = &self.scoop_path {
            if let Ok(value) = read_json_file(scoop_path) {
                let inner: ConfigInner = serde_json::from_value(value)?;
                internal::fs::write_json(&self.path, &inner)?;
                return Ok(Config {
                    path: self.path.clone(),
                    inner,
                });
            }
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
/// Tolerant of a leading UTF-8 BOM (hand-edited config files saved by
/// notepad) and, as a fallback, JSON5 (comments / trailing commas) —
/// matching the manifest parse tolerance.
fn read_json_file(path: &Path) -> Fallible<serde_json::Value> {
    let mut buf = vec![];
    std::fs::File::open(path)?.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    let text = text.trim_start_matches('\u{FEFF}');
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => Ok(value),
        Err(strict_err) => json5::from_str::<serde_json::Value>(text)
            .map_err(|_| crate::Error::Custom(strict_err.to_string())),
    }
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

    #[serde(alias = "aria2_enabled")]
    #[serde(rename = "aria2-enabled")]
    #[serde(skip_serializing_if = "Option::is_none")]
    aria2_enabled: Option<bool>,

    #[serde(alias = "aria2_max_connection_per_server")]
    #[serde(rename = "aria2-max-connection-per-server")]
    #[serde(skip_serializing_if = "Option::is_none")]
    aria2_max_connection_per_server: Option<u32>,

    #[serde(alias = "aria2_min_split_size")]
    #[serde(rename = "aria2-min-split-size")]
    #[serde(skip_serializing_if = "Option::is_none")]
    aria2_min_split_size: Option<String>,

    #[serde(alias = "aria2_split")]
    #[serde(rename = "aria2-split")]
    #[serde(skip_serializing_if = "Option::is_none")]
    aria2_split: Option<u32>,

    #[serde(alias = "cachePath")]
    #[serde(default = "default::cache_path")]
    #[serde(skip_serializing_if = "default::is_default_cache_path")]
    cache_path: PathBuf,

    #[serde(skip_serializing_if = "Option::is_none")]
    cat_style: Option<String>,

    #[serde(alias = "output-style")]
    #[serde(skip_serializing_if = "Option::is_none")]
    output_style: Option<String>,

    #[serde(alias = "language")]
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<String>,

    #[serde(alias = "no-color")]
    #[serde(rename = "no-color")]
    #[serde(skip_serializing_if = "Option::is_none")]
    no_color: Option<bool>,

    /// The default architecture to use (Scoop's `default_architecture`
    /// config, see `Get-DefaultArchitecture`). When set, it overrides the
    /// runtime-detected host architecture for manifest field selection.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "deafult_architecture")]
    default_architecture: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gh_token: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub virustotal_api_key: Option<String>,

    #[serde(alias = "globalPath")]
    #[serde(default = "default::global_path")]
    #[serde(skip_serializing_if = "default::is_default_global_path")]
    global_path: PathBuf,

    #[serde(alias = "lastupdate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    last_update: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    use_isolated_path: Option<IsolatedPath>,

    /// Use SQLite to cache manifests.
    ///
    /// This config was introduced in Scoop v0.5.0 (Jul, 2024)
    #[serde(skip_serializing_if = "Option::is_none")]
    use_sqlite_cache: Option<bool>,

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
    #[serde(alias = "no_junctions")]
    #[serde(skip_serializing_if = "Option::is_none")]
    no_junction: Option<bool>,

    /// Continue multi-package operations despite individual package failures.
    #[serde(alias = "ignore_failure")]
    #[serde(rename = "ignore-failures")]
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

    #[serde(alias = "rootPath")]
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
    /// Initialize the config with default values.
    ///
    /// This function will try to write the default config to hok's own config
    /// path (`~/.config/hok/config.json`).
    pub(crate) fn init() -> Config {
        let config = Config::default();
        // try to write the default config to the default path, error is ignored
        let _ = internal::fs::write_json(default::hok_config_path(), &config.inner);
        config
    }

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

    /// Get the global root directory of Scoop.
    ///
    /// Globally installed apps are stored here. By default the value is
    /// `$env:SCOOP_GLOBAL` or `C:\ProgramData\scoop`.
    #[inline]
    pub fn global_path(&self) -> &Path {
        self.global_path.as_path()
    }

    /// Get the `no_junction` config.
    #[inline]
    pub fn no_junction(&self) -> bool {
        self.no_junction.unwrap_or_default()
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
        self.commit()
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

    /// Get the `output_style` config ("scoop" or "pacman").
    #[inline]
    pub fn output_style(&self) -> &str {
        self.output_style.as_deref().unwrap_or("scoop")
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

    /// Returns the cooldown duration (in seconds) remaining before the next
    /// bucket update is allowed. Returns `None` if no last update recorded.
    /// The default cooldown is 15 minutes (900 seconds).
    pub fn update_cooldown_remaining(&self) -> Option<i64> {
        const COOLDOWN_SECS: i64 = 900; // 15 minutes
        let last_ts = internal::time::parse_last_update(self.inner.last_update.as_ref()?)?;
        let elapsed = time::OffsetDateTime::now_utc().unix_timestamp() - last_ts.unix_timestamp();
        let remaining = COOLDOWN_SECS - elapsed;
        (remaining > 0).then_some(remaining)
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

    /// The user-configured default architecture, if any.
    ///
    /// Maps to Scoop's `default_architecture` config; when set it overrides
    /// the runtime-detected host architecture.
    #[inline]
    pub fn default_architecture(&self) -> Option<&str> {
        self.inner.default_architecture.as_deref()
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
    /// Returns bytes. Defaults to 5MB.
    pub fn aria2_min_split_size(&self) -> u64 {
        const DEFAULT: u64 = 5 * 1024 * 1024;
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

    /// Update config key with new value.
    pub(crate) fn set(&mut self, key: &str, value: &str) -> Fallible<()> {
        let is_unset = value.is_empty();
        match key {
            "aria2_enabled" | "aria2-enabled" => match is_unset {
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
            "output_style" | "output-style" => {
                self.inner.output_style = match is_unset {
                    true => None,
                    false => Some(value.to_string()),
                }
            }
            "ignore_failures" | "ignore-failures" | "ignore_failure" => match is_unset {
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
            "proxy" => match value {
                "" | "none" => self.inner.proxy = None,
                _ => self.inner.proxy = Some(value.to_string()),
            },
            "no_junction" | "no_junctions" => match is_unset {
                true => self.inner.no_junction = None,
                false => match value.parse::<bool>() {
                    Ok(v) => self.inner.no_junction = Some(v),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "no_color" | "no-color" => match is_unset {
                true => self.inner.no_color = None,
                false => match value.parse::<bool>() {
                    Ok(v) => self.inner.no_color = Some(v),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "aria2_split" | "aria2-split" => match is_unset {
                true => self.inner.aria2_split = None,
                false => match value.parse::<u32>() {
                    Ok(v) => self.inner.aria2_split = Some(v),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "aria2_max_connection_per_server" | "aria2-max-connection-per-server" => match is_unset
            {
                true => self.inner.aria2_max_connection_per_server = None,
                false => match value.parse::<u32>() {
                    Ok(v) => self.inner.aria2_max_connection_per_server = Some(v),
                    Err(_) => return Err(Error::ConfigValueInvalid(value.to_owned())),
                },
            },
            "aria2_min_split_size" | "aria2-min-split-size" => {
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
            key => return Err(Error::ConfigKeyInvalid(key.to_owned())),
        }

        self.commit()
    }

    /// Commit config changes and save to hok's own config file.
    ///
    /// `ConfigInner` only contains hok-supported keys, so the serialized
    /// file is exactly the supported set; Scoop-only settings are never
    /// copied over.
    pub(crate) fn commit(&self) -> Fallible<()> {
        internal::fs::write_json(&self.path, &self.inner)
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
            // default_cache_path: default::cache_path(),
            cache_path: default::cache_path(),
            cat_style: Default::default(),
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
            no_junction: Default::default(),
            output_style: Default::default(),
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

/// Get a list of possible config paths.
///
/// There are 3 possible locations for the `config.json` file:
///   1) Side-by-side with the real executable (symlink resolved);
///   2) Located in the `root` directory of Scoop;
///   3) Located in the XDG_CONFIG_HOME directory.
pub(crate) fn possible_config_paths() -> Vec<PathBuf> {
    let mut ret = vec![];

    if let Ok(exe_path) = std::env::current_exe() {
        if let Ok(metadata) = std::fs::symlink_metadata(&exe_path) {
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

    use crate::internal::path::normalize_path;

    /// Join the given `path` to `$HOME` and return a new [`PathBuf`].
    #[inline]
    fn home_join<P: AsRef<Path>>(path: P) -> PathBuf {
        dirs::home_dir().map(|p| p.join(path.as_ref())).unwrap()
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
    #[inline]
    pub(super) fn root_path() -> PathBuf {
        let path = if let Some(path) = std::env::var_os("SCOOP") {
            PathBuf::from(path)
        } else {
            home_join("scoop")
        };

        normalize_path(path)
    }

    /// Get the default Scoop cache path.
    #[inline]
    pub(super) fn cache_path() -> PathBuf {
        let path = if let Some(path) = std::env::var_os("SCOOP_CACHE") {
            PathBuf::from(path)
        } else {
            root_path().join("cache")
        };

        normalize_path(path)
    }

    /// Get the default Scoop global path.
    #[inline]
    pub(super) fn global_path() -> PathBuf {
        let path = if let Some(path) = std::env::var_os("SCOOP_GLOBAL") {
            return PathBuf::from(path);
        } else {
            std::env::var_os("ProgramData")
                .map(PathBuf::from)
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

/// List every supported setting with its current value and effective default.
///
/// Returns an aligned three-column table (`key`, `value`, `default`). The
/// current value is what hok actually uses at runtime (defaults apply when
/// the key is unset); the default column shows the built-in default,
/// including environment-resolved paths (`$SCOOP`, `$SCOOP_CACHE`, ...).
///
/// # Errors
///
/// Returns an error if the config is currently borrowed elsewhere.
pub fn list_all(session: &crate::Session) -> Fallible<String> {
    let config = session.config();

    let no = || "none".to_owned();
    let mut rows: Vec<[String; 3]> = vec![];

    rows.push([
        "aria2-enabled".into(),
        fmt_bool(config.aria2_enabled()).into(),
        "true".into(),
    ]);
    rows.push([
        "aria2-split".into(),
        config.aria2_split().to_string(),
        "5".into(),
    ]);
    rows.push([
        "aria2-max-connection-per-server".into(),
        config.aria2_max_connection_per_server().to_string(),
        "5".into(),
    ]);
    rows.push([
        "aria2-min-split-size".into(),
        fmt_size(config.aria2_min_split_size()),
        "5M".into(),
    ]);
    rows.push([
        "cache_path".into(),
        config.cache_path().display().to_string(),
        default::cache_path().display().to_string(),
    ]);
    let cat_style = config.cat_style();
    rows.push([
        "cat_style".into(),
        if cat_style.is_empty() {
            no()
        } else {
            cat_style.to_owned()
        },
        no(),
    ]);
    rows.push([
        "default_architecture".into(),
        config
            .default_architecture()
            .unwrap_or("auto-detected")
            .to_owned(),
        "auto-detected".into(),
    ]);
    rows.push([
        "global_path".into(),
        config.global_path().display().to_string(),
        default::global_path().display().to_string(),
    ]);
    rows.push([
        "gh_token".into(),
        config.gh_token.as_deref().unwrap_or("none").to_owned(),
        no(),
    ]);
    rows.push([
        "ignore-failures".into(),
        fmt_bool(config.ignore_failures()).into(),
        "true".into(),
    ]);
    rows.push([
        "ignore_running_processes".into(),
        fmt_bool(config.ignore_running_processes()).into(),
        "false".into(),
    ]);
    rows.push([
        "language".into(),
        config.language().to_owned(),
        "auto".into(),
    ]);
    rows.push([
        "last_update".into(),
        config
            .inner
            .last_update
            .as_deref()
            .unwrap_or("none")
            .to_owned(),
        no(),
    ]);
    rows.push([
        "no-color".into(),
        fmt_bool(config.no_color()).into(),
        "false".into(),
    ]);
    rows.push([
        "no_junction".into(),
        fmt_bool(config.no_junction()).into(),
        "false".into(),
    ]);
    rows.push([
        "output-style".into(),
        config.output_style().to_owned(),
        "scoop".into(),
    ]);
    rows.push([
        "private_hosts".into(),
        config
            .private_hosts()
            .map(|v| format!("{} host(s)", v.len()))
            .unwrap_or_else(no),
        no(),
    ]);
    rows.push([
        "proxy".into(),
        config.proxy().unwrap_or("none").to_owned(),
        no(),
    ]);
    rows.push([
        "root_path".into(),
        config.root_path().display().to_string(),
        default::root_path().display().to_string(),
    ]);
    let isolated = config.use_isolated_path();
    rows.push([
        "use_isolated_path".into(),
        match isolated {
            None => no(),
            Some(IsolatedPath::Boolean(b)) => fmt_bool(*b).to_owned(),
            Some(IsolatedPath::Named(name)) => name.clone(),
        },
        no(),
    ]);
    rows.push([
        "use_sqlite_cache".into(),
        fmt_bool(config.use_sqlite_cache()).into(),
        "false".into(),
    ]);
    rows.push([
        "virustotal_api_key".into(),
        config
            .virustotal_api_key
            .as_deref()
            .unwrap_or("none")
            .to_owned(),
        no(),
    ]);

    let key_w = rows
        .iter()
        .map(|r| r[0].len())
        .max()
        .unwrap_or(0)
        .max("key".len());
    let val_w = rows
        .iter()
        .map(|r| r[1].len())
        .max()
        .unwrap_or(0)
        .max("value".len());
    let mut out = format!("{:<key_w$}  {:<val_w$}  default\n", "key", "value");
    for row in &rows {
        out.push_str(&format!(
            "{:<key_w$}  {:<val_w$}  {}\n",
            row[0], row[1], row[2]
        ));
    }
    Ok(out)
}

fn fmt_bool(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

/// Format a byte size back into the `aria2-min-split-size` style (5M, 10M).
fn fmt_size(bytes: u64) -> String {
    const G: u64 = 1024 * 1024 * 1024;
    const M: u64 = 1024 * 1024;
    const K: u64 = 1024;
    if bytes >= G && bytes % G == 0 {
        format!("{}G", bytes / G)
    } else if bytes >= M && bytes % M == 0 {
        format!("{}M", bytes / M)
    } else if bytes >= K && bytes % K == 0 {
        format!("{}K", bytes / K)
    } else {
        bytes.to_string()
    }
}

/// Open the config file in the user's configured editor.
///
/// Launches `$EDITOR` (if set) with the config file path as its argument and
/// waits for the editor to exit. Returns `Ok(false)` when the `EDITOR`
/// environment variable is not set, so the caller can fall back to opening
/// the file with the system default handler.
///
/// # Errors
///
/// I/O errors from spawning or waiting on the editor process are returned.
pub fn edit(session: &crate::Session) -> Fallible<bool> {
    match std::env::var("EDITOR") {
        Ok(editor) => {
            let mut child = std::process::Command::new(editor.as_str())
                .arg(&session.config().path)
                .spawn()?;
            child.wait()?;
            Ok(true)
        }
        Err(_) => Ok(false),
    }
}

/// Set a configuration key. *
///
/// # Errors
///
/// A [`ConfigInUse`][1] error will be returned if the config is borrowed
/// elsewhere.
///
/// A [`ConfigKeyInvalid`][2] error will be returned if the key is invalid.
///
/// A [`ConfigValueInvalid`][3] error will be returned if the value is invalid.
///
/// [1]: crate::Error::ConfigInUse
/// [2]: crate::Error::ConfigKeyInvalid
/// [3]: crate::Error::ConfigValueInvalid
pub fn set(session: &crate::Session, key: &str, value: &str) -> Fallible<()> {
    session.config_mut()?.set(key, value)
}

/// Add an alias to the config.
pub fn alias_add(session: &crate::Session, name: &str, command: &str) -> Fallible<()> {
    let mut config = session.config_mut()?;
    config.set_alias(name, Some(command))
}

/// Remove an alias from the config.
pub fn alias_remove(session: &crate::Session, name: &str) -> Fallible<()> {
    let mut config = session.config_mut()?;
    config.set_alias(name, None)
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
            "no-color" => config.no_color,
            "ignore_running_processes" => config.ignore_running_processes,
            _ => panic!("not a bool key: {key}"),
        }
    }

    fn u32_field(config: &ConfigInner, key: &str) -> Option<u32> {
        match key {
            "aria2-split" | "aria2_split" => config.aria2_split,
            "aria2-max-connection-per-server" => config.aria2_max_connection_per_server,
            _ => panic!("not a u32 key: {key}"),
        }
    }

    fn string_field<'a>(config: &'a ConfigInner, key: &str) -> Option<&'a str> {
        match key {
            "aria2-min-split-size" => config.aria2_min_split_size.as_deref(),
            "language" => config.language.as_deref(),
            "virustotal_api_key" => config.virustotal_api_key.as_deref(),
            _ => panic!("not a string key: {key}"),
        }
    }

    /// Every key reachable through `hok config set` must round-trip:
    /// set a value, verify it landed, then unset and verify it cleared.
    #[test]
    fn set_unset_roundtrip() {
        let session = test_session("config_set_roundtrip");

        // bool keys
        for key in ["no_junction", "no-color", "ignore_running_processes"] {
            crate::config::set(&session, key, "true").unwrap();
            assert_eq!(bool_field(&session.config().inner, key), Some(true));
            crate::config::set(&session, key, "false").unwrap();
            assert_eq!(bool_field(&session.config().inner, key), Some(false));
            crate::config::set(&session, key, "").unwrap();
            assert_eq!(bool_field(&session.config().inner, key), None);
        }

        // u32 keys (including underscore alias)
        for key in [
            "aria2-split",
            "aria2_split",
            "aria2-max-connection-per-server",
        ] {
            crate::config::set(&session, key, "7").unwrap();
            assert_eq!(u32_field(&session.config().inner, key), Some(7));
            crate::config::set(&session, key, "").unwrap();
            assert_eq!(u32_field(&session.config().inner, key), None);
        }

        // string keys
        for key in ["aria2-min-split-size", "language", "virustotal_api_key"] {
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

    /// `list_all` shows every supported key with its current value and
    /// effective default; after `config set` the current value updates.
    #[test]
    fn list_all_shows_supported_keys_with_current_and_default() {
        let session = test_session("config_list_all");

        let table = crate::config::list_all(&session).unwrap();
        // header row
        assert!(
            table.lines().next().unwrap().contains("default"),
            "expected header row, got:\n{table}"
        );
        // every key reachable through `hok config set` is listed
        for key in [
            "aria2-enabled",
            "aria2-split",
            "aria2-max-connection-per-server",
            "aria2-min-split-size",
            "cache_path",
            "cat_style",
            "default_architecture",
            "global_path",
            "gh_token",
            "ignore-failures",
            "ignore_running_processes",
            "language",
            "last_update",
            "no-color",
            "no_junction",
            "output-style",
            "private_hosts",
            "proxy",
            "root_path",
            "use_isolated_path",
            "use_sqlite_cache",
            "virustotal_api_key",
        ] {
            assert!(
                table.lines().any(|l| l.starts_with(key)),
                "missing key {key} in:\n{table}"
            );
        }

        // unset bool key: current value equals the default
        assert!(no_junction_row(&table).contains("false"), "{table}");

        // after setting, the current value column shows the new value
        crate::config::set(&session, "no_junction", "true").unwrap();
        let table = crate::config::list_all(&session).unwrap();
        assert!(no_junction_row(&table).contains("true"), "{table}");
    }

    fn no_junction_row(table: &str) -> &str {
        table
            .lines()
            .find(|l| l.starts_with("no_junction"))
            .expect("no_junction row")
    }

    /// Scoop-only settings that hok does not support must be rejected by the
    /// CLI setter (they can still be read from Scoop's read-only file, but
    /// never written to hok's own file).
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
            "debug",
            "force_update",
            "shim",
        ] {
            let err = crate::config::set(&session, key, "x").unwrap_err();
            assert!(
                matches!(err, Error::ConfigKeyInvalid(_)),
                "{key}: unexpected error {err}"
            );
        }
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
        config.commit().unwrap();

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
}
