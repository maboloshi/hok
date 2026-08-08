//! Environment variable handling for package install/removal.
//!
//! - **Add**: [`add()`] applies `env_set` and `env_add_path` from the
//!   manifest during installation — mirroring Scoop's `env_add_path` /
//!   `env_set` install hooks (`lib/install.ps1`).
//! - **Remove**: [`remove()`] undoes them during uninstall/cleanup. Path
//!   entries are matched case-insensitively against both the default `PATH`
//!   and the isolated env var, mirroring Scoop's `env_rm_path`.
//!
//! # Design
//!
//! - **Uses internal env module**: Delegates to [`internal::env`] for
//!   the actual Windows Registry operations.
//! - **Isolated path support**: Respects Scoop's `use_isolated_path`
//!   config, which can use a custom env var name instead of `PATH`.
//! - **Event emission**: Fires `PackageEnvPathAddStart/Done`,
//!   `PackageEnvVarSetStart/Done` and `PackageEnvPathRemoveStart/Done`,
//!   `PackageEnvVarRemoveStart/Done` events — in Scoop's order (path
//!   entries first, then variables), for the event loop.
//! - **`$dir` expansion**: `env_set` values go through the same
//!   installer-variable expansion as `installer.args` (via
//!   [`expand_scoop_vars`](crate::package::operations::expand_scoop_vars)),
//!   covering the common `$dir` / `$persist_dir` / `$scoopdir`-style tokens
//!   that Scoop's `ExpandString` handles.
//! - **Global support**: In global mode ([`Session::is_global`]) paths are
//!   resolved via `effective_root_path()` and variables are written to the
//!   machine-wide hive (`HKLM\...\Session Manager\Environment`), which
//!   requires administrator privileges.

use std::ffi::OsString;

use crate::{
    config, error::Fallible, internal, internal::env::EnvScope,
    package::operations::expand_scoop_vars, package::{Manifest, Package}, Error, Event, Session,
};

/// Apply all environment variable definitions of a given package.
///
/// Mirrors Scoop's `env_add_path` + `env_set` install hooks:
/// - `env_add_path` entries are prepended to the (possibly isolated) PATH
///   env var, deduplicated case-insensitively, preserving manifest order.
/// - `env_set` entries are written to the registry, with `$dir`-style
///   variables expanded first.
pub fn add(session: &Session, package: &Package) -> Fallible<()> {
    let scope = if session.is_global() {
        EnvScope::Global
    } else {
        EnvScope::User
    };
    let config = session.config();
    let app_path = session
        .app_dir(package.name());
    let version = session.current_dir_name(package.version());
    let working_dir = app_path.join(version);

    // Add environment path (Scoop order: env_add_path first, then env_set)
    if let Some(env_add_path) = package.manifest().env_add_path() {
        require_env_privileges(session, package)?;
        let env_path_name = match config.use_isolated_path() {
            Some(config::IsolatedPath::Named(name)) => name.to_owned(),
            Some(config::IsolatedPath::Boolean(true)) => "SCOOP_PATH".to_owned(),
            _ => "PATH".to_owned(),
        };
        let mut paths = internal::env::get_path_like_env(&env_path_name, scope)?;

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageEnvPathAddStart);
        }

        let add_paths = env_add_path
            .into_iter()
            .map(|p| internal::path::normalize_path(app_path.join(version).join(p)))
            .collect::<Vec<_>>();

        // Lowercase forms for the case-insensitive dedup below (Scoop's
        // Add-Path matches case-insensitively via `-like`).
        let add_path_lower = add_paths
            .iter()
            .map(|p| p.to_string_lossy().to_lowercase())
            .collect::<Vec<_>>();
        let mut existing_lower = paths
            .iter()
            .map(|p| internal::path::normalize_path(p).to_string_lossy().to_lowercase())
            .collect::<Vec<_>>();

        // Prepend each new path once, keeping manifest order (mirrors Scoop's
        // Add-Path, which places the new paths before the existing ones).
        for (idx, p) in add_paths.iter().enumerate() {
            if existing_lower.contains(&add_path_lower[idx]) {
                continue;
            }
            paths.insert(0, p.clone());
            existing_lower.push(add_path_lower[idx].clone());
        }

        let updated = std::env::join_paths(paths).map_err(|e| Error::Custom(e.to_string()))?;

        internal::env::set(&env_path_name, Some(&updated), scope)?;

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageEnvPathAddDone);
        }
    }

    // Set environment variables
    if let Some(env_set) = package.manifest().env_set() {
        require_env_privileges(session, package)?;
        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageEnvVarSetStart);
        }

        for (key, value) in env_set {
            let expanded = expand_scoop_vars(&[value.as_str()], session, package, &working_dir, "");
            internal::env::set(key, Some(&OsString::from(expanded[0].clone())), scope)?;
        }

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageEnvVarSetDone);
        }
    }

    Ok(())
}

/// Unset all environment variables defined by a given package.
pub fn remove(session: &Session, package: &Package) -> Fallible<()> {
    assert!(package.is_installed());

    let version = session.current_dir_name(package.installed_version().unwrap());

    remove_impl(session, package, package.manifest(), version)
}

/// Remove the env entries described by a specific manifest + version dir.
///
/// Used by the upgrade path to clean up the **old** manifest's entries
/// before the new version is installed (mirrors `scoop-update.ps1`, which
/// runs `env_rm_path`/`env_rm` against `$old_manifest`).
pub(crate) fn remove_with_manifest(
    session: &Session,
    package: &Package,
    manifest: &Manifest,
    version: &str,
) -> Fallible<()> {
    remove_impl(session, package, manifest, version)
}

fn remove_impl(
    session: &Session,
    package: &Package,
    manifest: &Manifest,
    version: &str,
) -> Fallible<()> {
    let scope = if session.is_global() {
        EnvScope::Global
    } else {
        EnvScope::User
    };

    // Remove environment path (Scoop order: env_rm_path first, then env_rm)
    if let Some(env_add_path) = manifest.env_add_path() {
        require_env_privileges(session, package)?;
        let config = session.config();
        let env_path_name = match config.use_isolated_path() {
            Some(config::IsolatedPath::Named(name)) => name.to_owned(),
            Some(config::IsolatedPath::Boolean(true)) => "SCOOP_PATH".to_owned(),
            _ => "PATH".to_owned(),
        };
        let app_path = session
            .app_dir(package.name());

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageEnvPathRemoveStart);
        }

        let env_add_path = env_add_path
            .into_iter()
            .map(|p| internal::path::normalize_path(app_path.join(version).join(p)))
            .collect::<Vec<_>>();

        // Lowercase forms for the case-insensitive match below.
        let add_path_lower = env_add_path
            .iter()
            .map(|p| p.to_string_lossy().to_lowercase())
            .collect::<Vec<_>>();

        // Mirrors Scoop's `env_rm_path`: always clean the default `PATH`,
        // and also the isolated env var when it differs (USE_ISOLATED_PATH).
        let mut targets = vec!["PATH".to_owned()];
        if env_path_name != "PATH" {
            targets.push(env_path_name);
        }

        for target in targets {
            let mut paths = internal::env::get_path_like_env(&target, scope)?;

            // Scoop matches case-insensitively (`-like`); normalize both sides
            // so separator and `.`/`..` differences don't leak entries.
            paths.retain(|p| {
                let norm = internal::path::normalize_path(p).to_string_lossy().to_lowercase();
                !add_path_lower.iter().any(|t| *t == norm)
            });

            if paths.is_empty() {
                // Scoop's Set-EnvVar deletes the variable when the value is empty.
                internal::env::set(&target, None, scope)?;
            } else {
                let updated =
                    std::env::join_paths(paths).map_err(|e| Error::Custom(e.to_string()))?;
                internal::env::set(&target, Some(&updated), scope)?;
            }
        }

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageEnvPathRemoveDone);
        }
    }

    // Unset environment variables
    if let Some(env_set) = manifest.env_set() {
        require_env_privileges(session, package)?;
        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageEnvVarRemoveStart);
        }

        let keys = env_set.keys();
        for key in keys {
            internal::env::set(key, None, scope)?;
        }

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageEnvVarRemoveDone);
        }
    }

    Ok(())
}

/// Global env writes require administrator privileges; this gate only runs
/// when the manifest actually has env entries to apply.
fn require_env_privileges(session: &Session, package: &Package) -> Fallible<()> {
    if session.is_global() && !session.is_admin() {
        return Err(Error::Custom(format!(
            "cannot modify environment variables for global install of '{}': \
             administrator privileges required",
            package.name()
        )));
    }
    Ok(())
}
