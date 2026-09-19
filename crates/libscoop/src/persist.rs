//! Persistent-data linking for package upgrades.
//!
//! Scoop preserves user data across reinstalls/upgrades by storing it
//! in a `persist` directory and linking it back after extraction. This
//! module implements that logic.
//!
//! # Design
//!
//! - **Three-way logic**: For each `persist` entry:
//!   1. Persist target exists → keep it; if the archive also ships a copy at
//!      the source path, move that copy aside to `<source>.original`
//!      (upstream's backup), then link.
//!   2. No target but a source exists → move the source into the persist dir,
//!      then link.
//!   3. Neither → create empty target (file if name has extension,
//!      directory otherwise), then link.
//! - **Link types**: Directories use junctions; files use hard links.
//!   This matches Scoop's behaviour for transparent data access.

use std::path::Path;

use tracing::debug;

use crate::{Event, Session, error::Fallible, internal, package::Package, permission};

/// Link persistent data for a package.
///
/// For each entry in `manifest.persist`:
/// - If persist target exists → keep it and move any archive copy at the
///   source aside to `<source>.original`, then link
/// - If no target but a source exists → move the source to persist, then link
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

        // Drop a leftover link at `source` (a repeated install leaves the
        // junction behind). A real file or directory there is the archive's own
        // default data and belongs to the branches below — `remove_symlink`
        // unlinks plain entries as well, so only a link may be unlinked here.
        if is_link(&src_path) {
            internal::fs::remove_symlink(&src_path)?;
        }

        if tgt_path.exists() {
            // Persist data already exists: keep it as-is and move the archive's
            // own copy out of the way so the link can take its place. Upstream
            // renames it to `<source>.original` (lib/install.ps1:465-469); a
            // non-empty directory cannot be deleted in place, which is what
            // used to surface as `ERROR_ALREADY_EXISTS` (os error 183) from
            // the link creation below.
            if src_path.exists() {
                let backup = with_suffix(&src_path, ".original");
                remove_path(&backup)?;
                std::fs::rename(&src_path, &backup)?;
            }
        } else if src_path.exists() {
            // First install — move default data to persist dir
            if src_path.is_dir() {
                std::fs::rename(&src_path, &tgt_path)?;
            } else {
                std::fs::copy(&src_path, &tgt_path)?;
                std::fs::remove_file(&src_path)?;
            }
        } else if has_extension(target) {
            // Neither exists but name looks like a file — create empty file
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

/// Whether a symlink or junction (as opposed to a real file/directory) sits at
/// `path`. Windows reports a directory junction as a symlink through
/// `symlink_metadata`.
fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
}

/// `<path><suffix>` — the shape of upstream's `"$source.original"` backup
/// (lib/install.ps1:468).
fn with_suffix(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    std::path::PathBuf::from(name)
}

/// Remove whatever sits at `path` (file, directory, or link) if anything does.
///
/// Only used for the `<source>.original` backup: the backup of a repeated
/// install has to go before the new one can be renamed into place.
fn remove_path(path: &Path) -> Fallible<()> {
    if is_link(path) {
        // Unlink, so a leftover junction is never walked into.
        internal::fs::remove_symlink(path)?;
        return Ok(());
    }
    match std::fs::symlink_metadata(path) {
        Err(_) => Ok(()),
        Ok(meta) if meta.is_dir() => {
            std::fs::remove_dir_all(path)?;
            Ok(())
        }
        Ok(_) => {
            std::fs::remove_file(path)?;
            Ok(())
        }
    }
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
        let _ = tx.try_send(Event::PackagePersistPurgeStart);
    }
    let persist_dir = session.persist_dir(pkg_name);
    internal::fs::remove_dir(persist_dir)?;
    if let Some(tx) = session.emitter() {
        let _ = tx.try_send(Event::PackagePersistPurgeDone);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::manifest::Manifest;
    use crate::package::operations;
    use crate::test_utils;

    /// Build a package whose manifest declares a single `persist` entry.
    fn make_package(name: &str, persist: &str) -> Package {
        let json = format!(
            r#"{{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT", "persist": {}}}"#,
            persist
        );
        Package::from(name, "test", Manifest::from_json(name, &json).unwrap())
    }

    /// Lay out `<root>/apps/<name>/1.0.0` plus its `current` junction — the
    /// state `commit_one_install` leaves behind right before `link` runs.
    /// Returns the version dir.
    fn install_layout(root: &Path, name: &str) -> std::path::PathBuf {
        let app_dir = root.join("apps").join(name);
        let version_dir = app_dir.join("1.0.0");
        std::fs::create_dir_all(&version_dir).unwrap();
        operations::link_current(&app_dir, &version_dir, false).unwrap();
        version_dir
    }

    /// Regression (sublime-text 4-4212): the persist target already exists and
    /// the archive ships a *non-empty* copy of the persisted path (`Data/`).
    /// The archive copy must be moved aside to `<source>.original`; it used to
    /// survive the (non-recursive) cleanup and make the junction creation fail
    /// with `ERROR_ALREADY_EXISTS` (os error 183).
    #[test]
    fn link_moves_non_empty_source_aside_when_target_exists() {
        let root = test_utils::tmpdir("persist_existing_target");
        let session = test_utils::test_session(&root);
        let pkg = make_package("sub", r#""Data""#);
        let version_dir = install_layout(&root, "sub");

        // The archive ships `Data/KEEPME`; the persist dir already holds the
        // user's data from an earlier install.
        std::fs::create_dir_all(version_dir.join("Data")).unwrap();
        std::fs::write(version_dir.join("Data").join("KEEPME"), []).unwrap();
        let persist_data = root.join("persist").join("sub").join("Data");
        std::fs::create_dir_all(&persist_data).unwrap();
        std::fs::write(persist_data.join("settings.dat"), b"user").unwrap();

        link(&session, &pkg).unwrap();

        let src = version_dir.join("Data");
        assert_eq!(
            std::fs::canonicalize(&src).unwrap(),
            std::fs::canonicalize(&persist_data).unwrap(),
            "source must link to the persist dir"
        );
        assert_eq!(
            std::fs::read(persist_data.join("settings.dat")).unwrap(),
            b"user",
            "persisted user data must be left untouched"
        );
        assert!(
            !persist_data.join("KEEPME").exists(),
            "the archive copy must not be merged into the persist dir"
        );
        assert!(
            version_dir.join("Data.original").join("KEEPME").exists(),
            "the archive copy must be preserved as Data.original"
        );
    }

    /// A backup left behind by an earlier attempt must not block the rename.
    #[test]
    fn link_replaces_previous_original_backup() {
        let root = test_utils::tmpdir("persist_replace_backup");
        let session = test_utils::test_session(&root);
        let pkg = make_package("sub", r#""Data""#);
        let version_dir = install_layout(&root, "sub");

        std::fs::create_dir_all(version_dir.join("Data")).unwrap();
        std::fs::write(version_dir.join("Data").join("KEEPME"), []).unwrap();
        std::fs::create_dir_all(version_dir.join("Data.original")).unwrap();
        std::fs::write(version_dir.join("Data.original").join("stale"), []).unwrap();
        std::fs::create_dir_all(root.join("persist").join("sub").join("Data")).unwrap();

        link(&session, &pkg).unwrap();

        let backup = version_dir.join("Data.original");
        assert!(backup.join("KEEPME").exists(), "new backup content missing");
        assert!(!backup.join("stale").exists(), "stale backup not replaced");
    }

    /// First install: the archive's default data moves into the persist dir
    /// and the source becomes the link.
    #[test]
    fn link_moves_source_into_persist_when_target_absent() {
        let root = test_utils::tmpdir("persist_first_install");
        let session = test_utils::test_session(&root);
        let pkg = make_package("sub", r#""Data""#);
        let version_dir = install_layout(&root, "sub");

        std::fs::create_dir_all(version_dir.join("Data")).unwrap();
        std::fs::write(version_dir.join("Data").join("KEEPME"), []).unwrap();

        link(&session, &pkg).unwrap();

        let persist_data = root.join("persist").join("sub").join("Data");
        assert!(
            persist_data.join("KEEPME").exists(),
            "default data not moved"
        );
        assert_eq!(
            std::fs::canonicalize(version_dir.join("Data")).unwrap(),
            std::fs::canonicalize(&persist_data).unwrap()
        );
    }

    /// A *file* shipped by the archive at the persist path must be moved into
    /// the persist dir with its content intact — the previous cleanup deleted
    /// it first and then created an empty file in its place, losing the
    /// archive's default content.
    #[test]
    fn link_moves_archive_file_into_persist_without_losing_content() {
        let root = test_utils::tmpdir("persist_archive_file");
        let session = test_utils::test_session(&root);
        let pkg = make_package("sub", r#""config.ini""#);
        let version_dir = install_layout(&root, "sub");
        std::fs::write(version_dir.join("config.ini"), b"defaults").unwrap();

        link(&session, &pkg).unwrap();

        let persist_file = root.join("persist").join("sub").join("config.ini");
        assert_eq!(
            std::fs::read(&persist_file).unwrap(),
            b"defaults",
            "archive default content lost"
        );
        assert_eq!(
            std::fs::read(version_dir.join("config.ini")).unwrap(),
            b"defaults",
            "linked file must expose the same content"
        );
    }

    /// Re-linking an intact install is a no-op: `current` already carries the
    /// link, so there is no source data to move and no backup to write.
    #[test]
    fn link_is_idempotent_for_an_intact_install() {
        let root = test_utils::tmpdir("persist_idempotent");
        let session = test_utils::test_session(&root);
        let pkg = make_package("sub", r#""Data""#);
        let version_dir = install_layout(&root, "sub");

        // Persist data from an earlier install, no archive copy at the source
        // (which is what an intact link looks like on a re-run).
        let persist_data = root.join("persist").join("sub").join("Data");
        std::fs::create_dir_all(&persist_data).unwrap();

        link(&session, &pkg).unwrap();
        link(&session, &pkg).unwrap();

        assert!(
            is_link(&version_dir.join("Data")),
            "the source must be a junction, not a directory copy"
        );
        assert_eq!(
            std::fs::canonicalize(version_dir.join("Data")).unwrap(),
            std::fs::canonicalize(&persist_data).unwrap()
        );
        assert!(
            !version_dir.join("Data.original").exists(),
            "an intact install must not grow a backup"
        );
    }
}
