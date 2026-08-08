//! Symlink (de)registration primitives.
//!
//! Recreate / remove the `current` symlink of a package directory.
//!
//! Both functions are `no_junction`-aware, mirroring upstream Scoop's
//! `link_current` / `unlink_current` (lib/install.ps1): under the
//! `NO_JUNCTION` config the junction is never created (and never needs
//! removal) — runtime entries resolve to the version dir directly.

use std::path::Path;

use crate::{error::Fallible, internal};

/// Recreate the `current` symlink of `pkg_dir` pointing at `target_dir`.
///
/// Any pre-existing `current` entry (symlink or leftover real directory) is
/// removed first, so the link always points at `target_dir` afterwards.
/// Under `no_junction` no link is created (upstream returns the version dir
/// as-is instead).
pub fn link_current(pkg_dir: &Path, target_dir: &Path, no_junction: bool) -> Fallible<()> {
    if no_junction {
        return Ok(());
    }
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
/// which guards with `Test-Path` — e.g. when `NO_JUNCTION` is set). Under
/// `no_junction` there is nothing to remove.
pub fn unlink_current(pkg_dir: &Path, no_junction: bool) -> Fallible<()> {
    if no_junction {
        return Ok(());
    }
    let current_lnk = pkg_dir.join("current");
    match internal::fs::remove_symlink(&current_lnk) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_pkg_dir(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("hok_test_{name}"));
        let _ = std::fs::remove_dir_all(&root);
        let pkg_dir = root.join("apps/foo");
        let target = root.join("apps/foo/1.0.0");
        std::fs::create_dir_all(&target).unwrap();
        (pkg_dir, target)
    }

    #[test]
    fn link_current_no_junction_skips_junction() {
        let (pkg_dir, target) = tmp_pkg_dir("link_no_junction");
        link_current(&pkg_dir, &target, true).unwrap();
        assert!(
            !pkg_dir.join("current").exists(),
            "no junction should be created under no_junction"
        );

        link_current(&pkg_dir, &target, false).unwrap();
        assert!(
            pkg_dir.join("current").exists(),
            "junction should be created with junctions enabled"
        );

        unlink_current(&pkg_dir, true).unwrap();
        assert!(
            pkg_dir.join("current").exists(),
            "no_junction unlink must not touch the link"
        );

        unlink_current(&pkg_dir, false).unwrap();
        assert!(
            !pkg_dir.join("current").exists(),
            "unlink removes the junction"
        );
    }
}
