//! Inspect Scoop shims.
//!
//! Lists the shims installed under `<root>/shims/` and resolves the concrete
//! shim files (`.cmd` / `.ps1` / `.exe` / no extension) for a given command —
//! the business logic behind the `hok shim` and `hok which` commands.

use std::path::PathBuf;

use crate::{error::Fallible, Session};

/// List the names of all shims (each `.ps1` shim is reported once, without
/// extension). The list is sorted for stable output.
pub fn list_shims(session: &Session) -> Fallible<Vec<String>> {
    let shims_dir = session.effective_root_path().join("shims");
    if !shims_dir.exists() {
        return Ok(Vec::new());
    }

    let mut names = Vec::new();
    for entry in std::fs::read_dir(&shims_dir)?.flatten() {
        let name = entry.file_name();
        if let Some(name) = name.to_str() {
            // Skip .cmd files (show only .ps1 or no extension)
            if let Some(stem) = name.strip_suffix(".ps1") {
                names.push(stem.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

/// Resolve the concrete shim files for a command name.
///
/// Returns `(extension, path)` pairs for every shim variant that exists
/// (extension is `""` for an extension-less shim). The order is stable:
/// no extension, then `.cmd`, `.ps1`, `.exe`.
pub fn shim_paths(session: &Session, name: &str) -> Fallible<Vec<(String, PathBuf)>> {
    let shims_dir = session.effective_root_path().join("shims");
    let mut paths = Vec::new();
    for ext in ["", ".cmd", ".ps1", ".exe"] {
        let path = shims_dir.join(format!("{name}{ext}"));
        if path.exists() {
            paths.push((ext.to_string(), path));
        }
    }
    Ok(paths)
}
