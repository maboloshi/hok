//! Path manipulation utilities.
//!
//! Provides Scoop-compatible path normalisation and segment-level
//! operations used across the codebase.
//!
//! # Design
//!
//! - **Normalisation**: [`normalize_path()`] resolves `.` and `..`
//!   segments, converts backslashes to forward slashes, and produces
//!   a canonical representation for comparison and display.
//! - **Segment helpers**: [`leaf()`] returns the final path component;
//!   [`leaf_base()`] strips the extension; [`without_leaf()`] returns
//!   the parent path.

#![allow(dead_code)]
use std::path::{Component, Path, PathBuf};

/// A directory that looks like a package version dir: `current`, or
/// containing a digit (e.g. `2.44.0`, `nightly-20240801`).
pub(crate) fn is_version_dir(s: &str) -> bool {
    s == "current" || s.chars().any(|c| c.is_ascii_digit())
}

/// Return the Leaf, i.e. file name (with extension), or directory name
/// of given path.
#[inline(always)]
pub fn leaf<P: AsRef<Path> + ?Sized>(path: &P) -> Option<&str> {
    path.as_ref().file_name().and_then(|s| s.to_str())
}

/// Return the LeafBase, i.e. file name without extension, for given file path.
///
/// If the given path is a directory, it returns the `Leaf` of the path instead.
#[inline(always)]
pub fn leaf_base<P: AsRef<Path> + ?Sized>(path: &P) -> Option<&str> {
    path.as_ref().file_stem().and_then(|s| s.to_str())
}

/// Normalize a path, removing things like `.` and `..`.
///
/// CAUTION: This does not resolve symlinks (unlike
/// [`std::fs::canonicalize`]). This may cause incorrect or surprising
/// behavior at times. This should be used carefully. Unfortunately,
/// [`std::fs::canonicalize`] can be hard to use correctly, since it can often
/// fail, or on Windows returns annoying device paths.
///
/// This function is copied from Cargo.
pub fn normalize_path<P: AsRef<Path>>(path: P) -> PathBuf {
    let mut components = path.as_ref().components().peekable();
    let mut ret = if let Some(c @ Component::Prefix(..)) = components.peek().cloned() {
        components.next();
        PathBuf::from(c.as_os_str())
    } else {
        PathBuf::new()
    };

    for component in components {
        match component {
            Component::Prefix(..) => unreachable!(),
            Component::RootDir => {
                ret.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                ret.pop();
            }
            Component::Normal(c) => {
                ret.push(c);
            }
        }
    }
    ret
}
