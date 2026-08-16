//! Persistent-data linking for package upgrades.
//!
//! Scoop preserves user data across reinstalls/upgrades by storing it
//! in a `persist` directory and linking it back after extraction. This
//! module implements that logic.
//!
//! # Design
//!
//! - **Three-way logic**: For each `persist` entry:
//!   1. Persist target exists → link (user data preserved as-is).
//!   2. Source exists → move to persist dir, then link.
//!   3. Neither → create empty target (file if name has extension,
//!      directory otherwise), then link.
//! - **Link types**: Directories use junctions; files use hard links.
//!   This matches Scoop's behaviour for transparent data access.

use std::path::Path;

use tracing::debug;

use crate::{error::Fallible, internal, package::Package, permission, Event, Session};

/// Link persistent data for a package.
///
/// For each entry in `manifest.persist`:
/// - If persist target exists → link (user data preserved)
/// - If source exists → move to persist, then link
/// - If neither → create empty target (file if name has extension, else dir)
///
/// Directories → junction; files → hard link (Scoop-compatible).
pub fn link(session: &Session, package: &Package) -> Fallible<()> {
    let persists = match package.manifest().persist() {
        Some(p) => p,
        None => return Ok(()),
    };

    let version_dir = session
        .app_dir(package.name())
        .join(session.current_dir_name(&package.effective_version()));
    let persist_root = session.persist_dir(package.name());

    for entry in &persists {
        let source = entry[0];
        let target = entry.get(1).unwrap_or(&entry[0]);

        let src_path = internal::path::normalize_path(version_dir.join(source));
        let tgt_path = internal::path::normalize_path(persist_root.join(target));

        // Ensure persist parent directory exists
        if let Some(parent) = tgt_path.parent() {
            internal::fs::ensure_dir(parent)?;
        }

        // Remove any pre-existing link at source
        let _ = internal::fs::remove_symlink(&src_path);
        let _ = std::fs::remove_file(&src_path);
        let _ = std::fs::remove_dir(&src_path);

        if tgt_path.exists() {
            // Persist data already exists
        } else if src_path.exists() {
            // First install — move default data to persist dir
            if let Some(parent) = tgt_path.parent() {
                internal::fs::ensure_dir(parent)?;
            }
            if src_path.is_dir() {
                std::fs::rename(&src_path, &tgt_path)?;
            } else {
                std::fs::copy(&src_path, &tgt_path)?;
                std::fs::remove_file(&src_path)?;
            }
        } else if has_extension(target) {
            // Neither exists but name looks like a file — create empty file
            if let Some(parent) = tgt_path.parent() {
                internal::fs::ensure_dir(parent)?;
            }
            std::fs::write(&tgt_path, [])?;
        } else {
            // Neither exists — create empty directory (Scoop default)
            internal::fs::ensure_dir(&tgt_path)?;
        }

        // Create link: junction for dirs, hard link for files
        if tgt_path.is_dir() {
            internal::fs::symlink_dir(&tgt_path, &src_path)?;
        } else {
            std::fs::hard_link(&tgt_path, &src_path)?;
        }
    }

    Ok(())
}

/// Check if a persist entry name looks like a file (has extension).
fn has_extension(name: &str) -> bool {
    std::path::Path::new(name).extension().is_some()
}

/// Scoop's `persist_permission` (lib/install.ps1:522-531): for *global*
/// installs with a `persist` field, grant the built-in Users group
/// (S-1-5-32-545) `Write` + `ObjectInherit` on the global persist root, so
/// non-admin users can write persisted data. Idempotent; skipped unless the
/// current process is elevated — exactly the `$global -and $manifest.persist
/// -and (is_admin)` guard of upstream. Runs right after [`link`] on install
/// and reset (install.ps1:67 / reset.ps1:91).
///
/// The ACL execution lives in [`permission`] as a reusable primitive; this
/// function owns the persist-specific guard and root resolution.
pub fn persist_permission(session: &Session, package: &Package) -> Fallible<()> {
    if !session.is_global() || package.manifest().persist().is_none() || !session.is_admin() {
        return Ok(());
    }
    permission::grant_users_write_inherit(&session.persist_root())
}

/// Remove persistent data symlinks in a specific version directory
/// (does NOT remove persist data).
///
/// Mirrors Scoop's `unlink_persist_data $manifest $dir`, which is called
/// against the version directory right before that directory is removed —
/// both for the current version and for each older version being cleaned up.
pub fn unlink(session: &Session, package: &Package, version_dir: &Path) -> Fallible<()> {
    let _ = session;
    assert!(package.is_installed());

    if let Some(persists) = package.manifest().persist() {
        for entry in &persists {
            let source = entry[0];
            let src_path = internal::path::normalize_path(version_dir.join(source));
            let _ = internal::fs::remove_symlink(&src_path);
        }
    }
    Ok(())
}

/// Purge the persistent data directory of `pkg_name` entirely.
///
/// Mirrors Scoop's `Remove-Item "$persistDir\$app"` on purge (scoop uninstall
/// `-p`). Emits [`Event::PackagePersistPurgeStart`] / [`Event::PackagePersistPurgeDone`]
/// around the removal.
pub fn purge(session: &Session, pkg_name: &str) -> Fallible<()> {
    debug!("persist: purging data for {}", pkg_name);
    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackagePersistPurgeStart);
    }
    let persist_dir = session.persist_dir(pkg_name);
    internal::fs::remove_dir(persist_dir)?;
    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackagePersistPurgeDone);
    }
    Ok(())
}
