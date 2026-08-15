//! Inspect Scoop shims.
//!
//! Lists the shims installed under `<root>/shims/` and resolves the concrete
//! shim files (`.cmd` / `.ps1` / `.exe` / no extension) for a given command —
//! the business logic behind the `hok shim` and `hok which` commands.

use std::path::PathBuf;

use crate::{error::Fallible, Session};

/// List the names of all shims (each shim is reported once, without
/// extension). Both `.shim` metadata files and `.ps1` shims are collected
/// (matching Scoop's `shim list`, which globs `*.shim`, `*.ps1`); a name
/// with both variants is listed once. The list is sorted for stable output.
pub fn list_shims(session: &Session) -> Fallible<Vec<String>> {
    let shims_dir = session.shims_dir();
    if !shims_dir.exists() {
        return Ok(Vec::new());
    }

    let mut names = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(&shims_dir)?.flatten() {
        let name = entry.file_name();
        if let Some(name) = name.to_str() {
            // Collect both `.shim` (metadata; Exe/custom shims) and `.ps1`
            // shims; `.cmd` wrappers are skipped as they pair with those.
            for suffix in [".shim", ".ps1"] {
                if let Some(stem) = name.strip_suffix(suffix) {
                    names.insert(stem.to_string());
                    break;
                }
            }
        }
    }
    Ok(names.into_iter().collect())
}

/// Resolve the concrete shim files for a command name.
///
/// Returns `(extension, path)` pairs for every shim variant that exists
/// (extension is `""` for an extension-less shim). The order is stable:
/// no extension, then `.cmd`, `.ps1`, `.exe`.
pub fn shim_paths(session: &Session, name: &str) -> Fallible<Vec<(String, PathBuf)>> {
    let shims_dir = session.shims_dir();
    let mut paths = Vec::new();
    for ext in ["", ".cmd", ".ps1", ".exe"] {
        let path = shims_dir.join(format!("{name}{ext}"));
        if path.exists() {
            paths.push((ext.to_string(), path));
        }
    }
    Ok(paths)
}
