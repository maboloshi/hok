//! File-system utilities.
//!
//! Thin wrappers around common `std::fs` operations used throughout
//! the codebase: directory creation, recursive removal, file copying,
//! and JSON serialisation with pretty-printing.
//!
//! # Design
//!
//! - **Error-returning helpers**: [`ensure_dir()`], [`remove_dir()`],
//!   and [`write_json_with_pretty_formatter()`] return `Fallible` for
//!   ergonomic use with `?`.
//! - **Atomic write**: [`write_json_with_pretty_formatter()`] writes
//!   to a temporary file and then renames it for atomicity.

use serde::Serialize;
use serde_json::ser::PrettyFormatter;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::Fallible;

/// Ensure given `path` exist.
#[inline]
pub fn ensure_dir<P: AsRef<Path> + ?Sized>(path: &P) -> io::Result<()> {
    std::fs::create_dir_all(path.as_ref())
}

/// Read the entire contents of a file into a string.
#[inline]
pub fn read_to_string<P: AsRef<Path>>(path: P) -> io::Result<String> {
    std::fs::read_to_string(path.as_ref())
}

/// Write the given contents to a file at given path.
#[inline]
pub fn write<P: AsRef<Path>, C: AsRef<[u8]>>(path: P, contents: C) -> io::Result<()> {
    std::fs::write(path.as_ref(), contents.as_ref())
}

/// Recursively collect all `.json` files under a directory.
///
/// Uses an explicit stack to avoid deep recursion on deeply nested trees.
pub fn walkdir_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&current) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "json") {
                    files.push(path);
                }
            }
        }
    }
    files
}

/// Remove given `path` recursively.
#[inline]
pub fn remove_dir<P: AsRef<Path>>(path: P) -> io::Result<()> {
    std::fs::remove_dir_all(path.as_ref())
}

/// Remove all files and subdirectories in given `path`.
///
/// This function will not remove the given `path` itself. No-op if the given
/// `path` does not exist.
#[inline(always)]
pub fn empty_dir<P: AsRef<Path> + ?Sized>(path: &P) -> io::Result<()> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(());
    }
    for entry in path.read_dir()? {
        let entry = entry?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            std::fs::remove_dir_all(&entry_path)?;
        } else {
            std::fs::remove_file(&entry_path)?;
        }
    }
    Ok(())
}

/// Write given serializable data to a JSON file at given path.
///
/// This function will create the file if it does not exist, and truncate it.
pub fn write_json<P, D>(path: P, data: D) -> Fallible<()>
where
    P: AsRef<Path>,
    D: Serialize,
{
    let path = path.as_ref();
    ensure_dir(path.parent().unwrap())?;

    // Use 4 spaces for indentation
    let formatter = PrettyFormatter::with_indent(b"    ");
    let mut buf = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    data.serialize(&mut ser)?;

    std::fs::write(path, &buf)?;
    Ok(())
}

/// Remove a symlink at `lnk`.
pub fn remove_symlink<P: AsRef<Path>>(lnk: P) -> io::Result<()> {
    let lnk = lnk.as_ref();
    let metadata = lnk.symlink_metadata()?;
    let mut permissions = metadata.permissions();

    // Remove possible readonly flag on the symlink added by `attrib +R` command
    if permissions.readonly() {
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(lnk, permissions)?;
    }

    if let Ok(target_metadata) = lnk.metadata() {
        if target_metadata.file_type().is_dir() {
            std::fs::remove_dir(lnk)
        } else {
            std::fs::remove_file(lnk)
        }
    } else {
        std::fs::remove_file(lnk).or_else(|_| std::fs::remove_dir(lnk))
    }
}

/// Create a directory symlink at `lnk` pointing to `src`.
pub fn symlink_dir<P: AsRef<Path>, Q: AsRef<Path>>(src: P, lnk: Q) -> io::Result<()> {
    let src = src.as_ref();
    let lnk = lnk.as_ref();
    let _ = remove_symlink(lnk);

    ensure_dir(lnk.parent().unwrap())?;
    if std::os::windows::fs::symlink_dir(src, lnk).is_err() {
        junction::create(src, lnk)
    } else {
        Ok(())
    }
}
