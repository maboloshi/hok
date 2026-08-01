//! Shared manifest discovery for bucket-oriented package tools.
//!
//! Recursively walks a directory tree and returns Scoop manifest JSON files,
//! including categorized bucket layouts.

use std::path::{Path, PathBuf};

/// Discover manifest JSON files under `dir`.
///
/// Returns paths for all `.json` files except `package.json`.
pub(crate) fn discover(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];

    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current)?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }

            if path.extension().map_or(false, |e| e == "json")
                && path.file_name().map_or(true, |n| n != "package.json")
            {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}
