//! PowerShell module management for package lifecycle.
//!
//! Installs and removes PowerShell modules declared by a package's
//! `psmodule` manifest field.
//!
//! # Design
//!
//! - **Junction link**: [`add()`] creates a junction `modules/<name>` that
//!   points at the package's runtime directory (the `current` junction, or
//!   the version dir under `NO_JUNCTION`), mirroring upstream Scoop's
//!   `install_psmodule` (lib/psmodules.ps1) which uses
//!   `New-DirectoryJunction`.
//! - **PSModulePath**: the modules directory is prepended to `PSModulePath`
//!   when missing, mirroring `ensure_in_psmodulepath`.
//! - **Event emission**: fires `PackagePsModuleAddStart/Done` on install and
//!   `PackagePsModuleRemoveStart/Done` on removal.

use std::ffi::OsString;
use tracing::debug;

use crate::internal;
use crate::{error::Fallible, package::Package, Event, Session};

/// Install the PowerShell module declared by a package into the modules
/// directory, mirroring upstream Scoop's `install_psmodule`
/// (lib/psmodules.ps1): a junction `modules/<name>` points at the package's
/// runtime directory and the modules dir is ensured to be on `PSModulePath`.
///
/// Must run *after* `link_current` so the module target resolves to the
/// `current` junction (or the version dir under `NO_JUNCTION`).
pub fn add(session: &Session, package: &Package) -> Fallible<()> {
    let Some(psmodule) = package.manifest().psmodule() else {
        return Ok(());
    };

    let module_name = psmodule.name();

    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackagePsModuleAddStart(module_name.to_owned()));
    }

    link_module(session, package)?;
    ensure_in_psmodulepath(session)?;

    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackagePsModuleAddDone);
    }
    Ok(())
}

/// Create the `modules/<name>` junction pointing at the package's runtime
/// directory (the `current` junction, or the version dir under
/// `NO_JUNCTION`). Split out of [`add`] so tests can verify the link
/// without touching the registry-backed `PSModulePath`.
fn link_module(session: &Session, package: &Package) -> Fallible<()> {
    let psmodule = package.manifest().psmodule().expect("psmodule present");

    let target = session
        .app_dir(package.name())
        .join(session.current_dir_name(package.version()));
    let link = session.modules_dir().join(psmodule.name());

    debug!(
        "psmodule: linking {} -> {}",
        link.display(),
        target.display()
    );
    internal::fs::symlink_dir(&target, &link)?;
    Ok(())
}

/// Ensure the modules directory is on `PSModulePath` for the current scope,
/// prepending it when missing (upstream `ensure_in_psmodulepath`).
fn ensure_in_psmodulepath(session: &Session) -> Fallible<()> {
    let scope = if session.is_global() {
        internal::env::EnvScope::Global
    } else {
        internal::env::EnvScope::User
    };
    let modules = session.modules_dir();
    let modules_str = modules.to_string_lossy().to_string();

    let current = internal::env::get("PSModulePath", scope)?;
    let current = if current.is_empty() && !session.is_global() {
        // Upstream defaults to the per-user PowerShell modules folder when
        // PSModulePath is unset for the user scope.
        dirs::home_dir()
            .map(|h| {
                h.join("Documents")
                    .join("WindowsPowerShell")
                    .join("Modules")
                    .to_string_lossy()
                    .to_string()
            })
            .unwrap_or_default()
    } else {
        current.to_string_lossy().to_string()
    };

    let already_present = current
        .split(';')
        .any(|p| !p.is_empty() && p.eq_ignore_ascii_case(&modules_str));
    if already_present {
        return Ok(());
    }

    let joined = if current.is_empty() {
        modules_str
    } else {
        format!("{modules_str};{current}")
    };
    internal::env::set("PSModulePath", Some(&OsString::from(joined)), scope)?;
    Ok(())
}

/// Remove PowerShell module imported by a given package.
pub fn remove(session: &Session, package: &Package) -> Fallible<()> {
    assert!(package.is_installed());

    if let Some(psmodule) = package.manifest().psmodule() {
        let mut psmodule_path = session.modules_dir();

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackagePsModuleRemoveStart(
                psmodule.name().to_owned(),
            ));
        }

        psmodule_path.push(psmodule.name());
        let _ = std::fs::remove_dir(psmodule_path);

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackagePsModuleRemoveDone);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::manifest::Manifest;
    use crate::package::{operations, Package};
    use crate::test_utils;

    fn make_package(name: &str, psmodule: &str) -> Package {
        let json = format!(
            r#"{{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT", "psmodule": {}}}"#,
            psmodule
        );
        Package::from(name, "test", Manifest::from_json(name, &json).unwrap())
    }

    /// `link_module` creates a junction `modules/<name>` pointing at the
    /// package's runtime directory (through `current`).
    #[test]
    fn link_module_creates_junction_to_current() {
        let root = test_utils::tmpdir("psmodule_link");
        let session = test_utils::test_session(&root);
        let pkg = make_package("foo", r#"{"name": "foo-mod"}"#);

        // Simulate the install layout: version dir exists, `current` linked.
        let app_dir = root.join("apps").join("foo");
        let version_dir = app_dir.join("1.0.0");
        std::fs::create_dir_all(&version_dir).unwrap();
        operations::link_current(&app_dir, &version_dir, false).unwrap();

        link_module(&session, &pkg).unwrap();

        let link = root.join("modules").join("foo-mod");
        assert!(link.exists(), "module junction missing: {}", link.display());
        let resolved = std::fs::canonicalize(&link).unwrap();
        let target = std::fs::canonicalize(&version_dir).unwrap();
        assert_eq!(resolved, target, "module must point at the app dir");
    }

    /// Under `NO_JUNCTION` the module junction points at the version dir
    /// (the same path `current` would have resolved to).
    #[test]
    fn link_module_no_junction_points_at_version_dir() {
        let root = test_utils::tmpdir("psmodule_link_no_junction");
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
        let session = crate::Session::new_with(&config_path).unwrap();
        let pkg = make_package("foo", r#"{"name": "foo-mod"}"#);

        let app_dir = root.join("apps").join("foo");
        let version_dir = app_dir.join("1.0.0");
        std::fs::create_dir_all(&version_dir).unwrap();
        operations::link_current(&app_dir, &version_dir, true).unwrap();

        link_module(&session, &pkg).unwrap();

        let link = root.join("modules").join("foo-mod");
        assert!(link.exists(), "module junction missing: {}", link.display());
        let resolved = std::fs::canonicalize(&link).unwrap();
        let target = std::fs::canonicalize(&version_dir).unwrap();
        assert_eq!(resolved, target, "module must point at the version dir");
    }
}
