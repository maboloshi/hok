//! Symlink (de)registration primitives.
//!
//! Recreate / remove the `current` symlink of a package directory.

use std::path::Path;

use crate::{error::Fallible, internal};

/// Recreate the `current` symlink of `pkg_dir` pointing at `target_dir`.
///
/// Any pre-existing `current` entry (symlink or leftover real directory) is
/// removed first, so the link always points at `target_dir` afterwards.
pub fn link_current(pkg_dir: &Path, target_dir: &Path) -> Fallible<()> {
    let current_lnk = pkg_dir.join("current");
    let _ = internal::fs::remove_symlink(&current_lnk);
    if current_lnk.exists() {
        let _ = std::fs::remove_dir_all(&current_lnk);
    }
    internal::fs::symlink_dir(target_dir, &current_lnk)?;
    Ok(())
}

/// Remove the `current` symlink of `pkg_dir`, if present.
///
/// Missing `current` is not an error (mirrors Scoop's `unlink_current`,
/// which guards with `Test-Path` — e.g. when `NO_JUNCTION` is set).
pub fn unlink_current(pkg_dir: &Path) -> Fallible<()> {
    let current_lnk = pkg_dir.join("current");
    match internal::fs::remove_symlink(&current_lnk) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
