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
//!   [`leaf_base()`] strips the extension; `without_leaf()` returns
//!   the parent path.

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

// ─── Scoop layout helpers ────────────────────────────────────────────────
// Root-parameterised layout resolvers. Kept only for call sites that cannot
// use `Session` methods — i.e. rayon parallel scans in `package::query`,
// where `&Session` is not `Sync` and cannot cross thread boundaries. All
// other layout accessors live on `Session` (`session.apps_dir()` etc.).
//
// # User-level vs effective root
//
// `buckets` are pinned to the *user-level* root even for global installs
// (upstream `$scoopdir\buckets`, buckets.ps1:1) — callers MUST pass
// `config.root_path()` here, never the effective root.

/// `<root>/buckets/<name>`. NOTE: user-level root, see module docs.
#[inline]
pub fn bucket_dir(root: &Path, name: &str) -> PathBuf {
    root.join("buckets").join(name)
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

/// Compute `target` relative to `base` with Windows path semantics, or
/// `None` when the two paths are on different drives (no relative form
/// exists — the caller falls back to the absolute path).
///
/// Mirrors upstream Scoop's `Resolve-Path -Relative` (lib/core.ps1
/// `shim()`), which resolves the target from the shims directory: on the
/// same drive it yields `..\apps\...`; across drives it returns a
/// drive-qualified path, which upstream detects (`^(\.\\)?\w:.*$`) and
/// treats as absolute. Component comparison is case-insensitive, matching
/// Windows path semantics.
pub fn relative_to(base: &Path, target: &Path) -> Option<PathBuf> {
    use Component::*;

    let base_comp: Vec<Component> = base.components().collect();
    let target_comp: Vec<Component> = target.components().collect();

    // Windows drive prefixes must match (case-insensitively); anything
    // else (non-prefixed base, different drives) has no relative form.
    let same_drive = match (base_comp.first(), target_comp.first()) {
        (Some(Prefix(a)), Some(Prefix(b))) => {
            let a = a.as_os_str().to_string_lossy().to_ascii_lowercase();
            let b = b.as_os_str().to_string_lossy().to_ascii_lowercase();
            a == b
        }
        _ => false,
    };
    if !same_drive {
        return None;
    }

    let mut common = 0usize;
    while common < base_comp.len()
        && common < target_comp.len()
        && same_component(base_comp[common], target_comp[common])
    {
        common += 1;
    }

    let mut rel = PathBuf::new();
    for _ in common..base_comp.len() {
        rel.push("..");
    }
    for comp in &target_comp[common..] {
        rel.push(comp.as_os_str());
    }
    Some(rel)
}

/// Case-insensitive component equality for path comparison (Windows
/// semantics); non-`Normal` components compare structurally.
fn same_component(a: Component, b: Component) -> bool {
    match (a, b) {
        (Component::Normal(x), Component::Normal(y)) => {
            let x = x.to_string_lossy().to_ascii_lowercase();
            let y = y.to_string_lossy().to_ascii_lowercase();
            x == y
        }
        (Component::Prefix(x), Component::Prefix(y)) => {
            let x = x.as_os_str().to_string_lossy().to_ascii_lowercase();
            let y = y.as_os_str().to_string_lossy().to_ascii_lowercase();
            x == y
        }
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_to_same_drive() {
        let base = Path::new(r"C:\scoop\shims");
        let target = Path::new(r"C:\scoop\apps\git\current\git.ps1");
        assert_eq!(
            relative_to(base, target).unwrap().to_string_lossy(),
            r"..\apps\git\current\git.ps1"
        );
    }

    #[test]
    fn relative_to_case_insensitive() {
        let base = Path::new(r"c:\scoop\shims");
        let target = Path::new(r"C:\Scoop\apps\git\current\git.ps1");
        assert_eq!(
            relative_to(base, target).unwrap().to_string_lossy(),
            r"..\apps\git\current\git.ps1"
        );
    }

    #[test]
    fn relative_to_cross_drive_is_none() {
        let base = Path::new(r"C:\scoop\shims");
        let target = Path::new(r"D:\scoop\apps\git\current\git.ps1");
        assert!(relative_to(base, target).is_none());
    }

    #[test]
    fn relative_to_relative_base_is_none() {
        let base = Path::new(r"shims");
        let target = Path::new(r"C:\scoop\apps\git\current\git.ps1");
        assert!(relative_to(base, target).is_none());
    }
}
