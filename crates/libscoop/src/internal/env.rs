//! Environment variable management via the Windows Registry.
//!
//! Reads and writes persistent environment variables stored in the
//! registry — user scope (`HKCU\Environment`) or machine scope
//! (`HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment`,
//! used by `--global` installs and requiring administrator privileges).
//! Used by package operations to manage `PATH` entries and other
//! environment changes.
//!
//! # Design
//!
//! - **Registry-backed**: Uses `winreg` to access Windows' persistent
//!   environment storage — the only reliable way to make environment
//!   changes that survive a process restart.
//! - **Two scopes**: [`EnvScope`] selects the hive; the global scope maps
//!   to Scoop's `-Global` env operations.
//! - **Value-type preservation**: [`set()`] keeps the existing registry
//!   value type (or picks `REG_EXPAND_SZ` for values containing `%`),
//!   mirroring Scoop's `Set-EnvVar` — so a `REG_EXPAND_SZ` `PATH` with
//!   `%SystemRoot%` is not downgraded to a literal `REG_SZ`.
//! - **Sub-module split**: The `windows` sub-module contains the raw
//!   Registry access; the parent module wraps it in higher-level helpers
//!   like [`get_path_like_env()`].

use std::path::PathBuf;

use crate::error::Fallible;

pub use windows::{get, set};

/// Scope of persistent environment storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvScope {
    /// Per-user: `HKCU\Environment`.
    User,
    /// Machine-wide (global installs):
    /// `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment`.
    Global,
}

/// Get the value of a path-like environment variable.
pub fn get_path_like_env(name: &str, scope: EnvScope) -> Fallible<Vec<PathBuf>> {
    let paths = get(name, scope)?;
    Ok(std::env::split_paths(&paths).collect())
}

mod windows {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::sync::LazyLock;
    use winreg::enums::{RegType, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::{RegKey, RegValue};

    use crate::error::Fallible;
    use crate::internal::env::EnvScope;

    /// `HKEY_CURRENT_USER` registry key handle.
    static HKCU: LazyLock<RegKey> = LazyLock::new(|| RegKey::predef(HKEY_CURRENT_USER));

    /// `HKEY_LOCAL_MACHINE` registry key handle.
    static HKLM: LazyLock<RegKey> = LazyLock::new(|| RegKey::predef(HKEY_LOCAL_MACHINE));

    /// Registry path of the machine-wide environment variables.
    const GLOBAL_ENV_KEY: &str = "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment";

    /// Open the `Environment` subkey for reading.
    fn open_env_key(scope: EnvScope) -> Fallible<RegKey> {
        match scope {
            EnvScope::User => Ok(HKCU.open_subkey(Path::new("Environment"))?),
            EnvScope::Global => Ok(HKLM.open_subkey(Path::new(GLOBAL_ENV_KEY))?),
        }
    }

    /// Open the `Environment` subkey for writing, creating it when missing.
    /// Machine-wide access fails with an explicit error unless the process
    /// has administrator privileges.
    fn create_env_key(scope: EnvScope) -> Fallible<RegKey> {
        match scope {
            EnvScope::User => {
                let (env, _) = HKCU.create_subkey(Path::new("Environment"))?;
                Ok(env)
            }
            EnvScope::Global => {
                let (env, _) = HKLM.create_subkey(Path::new(GLOBAL_ENV_KEY)).map_err(|e| {
                    let hint = if e.kind() == std::io::ErrorKind::PermissionDenied {
                        " — global installs require administrator privileges"
                    } else {
                        ""
                    };
                    crate::Error::Custom(format!(
                        "cannot open machine-wide environment variables (HKLM\\{GLOBAL_ENV_KEY}): {e}{hint}"
                    ))
                })?;
                Ok(env)
            }
        }
    }

    /// Get the value of an environment variable.
    /// Returns an empty string if the variable is not set.
    pub fn get(key: &str, scope: EnvScope) -> Fallible<OsString> {
        let env = open_env_key(scope)?;
        match env.get_value(key) {
            Ok(value) => Ok(value),
            // Scoop's Get-EnvVar returns $null for unset keys. Treat a missing
            // value as empty so PATH reads don't fail on fresh accounts that
            // never had a user-scope PATH in the registry.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(OsString::new()),
            Err(e) => Err(e.into()),
        }
    }

    /// Set the value of an environment variable, preserving/selecting the
    /// registry value type the way Scoop's `Set-EnvVar` does:
    /// - value containing `%` → `REG_EXPAND_SZ`
    /// - existing value → keep its current type
    /// - otherwise → `REG_SZ`
    /// If `value` is `None`, the variable is deleted (missing keys are ignored).
    pub fn set(key: &str, value: Option<&OsString>, scope: EnvScope) -> Fallible<()> {
        let env = create_env_key(scope)?;

        match value {
            Some(value) => {
                let vtype = if value.to_string_lossy().contains('%') {
                    RegType::REG_EXPAND_SZ
                } else if let Ok(existing) = env.get_raw_value(key) {
                    existing.vtype
                } else {
                    RegType::REG_SZ
                };
                let raw = RegValue {
                    bytes: to_utf16_bytes(value),
                    vtype,
                };
                env.set_raw_value(key, &raw)?;
            }
            None => {
                // ignore error of deleting non-existent value
                let _ = env.delete_value(key);
            }
        }
        Ok(())
    }

    /// Encode an `OsString` as UTF-16LE bytes with a trailing null, matching
    /// winreg's own string encoding (`to_utf16` + `v16_to_v8`).
    fn to_utf16_bytes(s: &OsString) -> Vec<u8> {
        let words: Vec<u16> = s.encode_wide().chain(Some(0)).collect();
        // SAFETY: u16 is 2 bytes; reinterpreting the buffer as u8 bytes is
        // what winreg does internally (`v16_to_v8`).
        unsafe { std::slice::from_raw_parts(words.as_ptr() as *const u8, words.len() * 2).to_vec() }
    }
}
