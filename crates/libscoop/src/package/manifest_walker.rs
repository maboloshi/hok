//! Shared manifest discovery for bucket-oriented package tools.
//!
//! Recursively walks a directory tree and returns Scoop manifest JSON files,
//! including categorized bucket layouts.
//!
//! # Overview
//!
//! [`discover`] performs an iterative depth-first scan of a directory,
//! collecting every `.json` file except `package.json`. It supports both
//! flat buckets (all manifests in one directory) and categorised layouts
//! (manifests nested inside subdirectories per category).
//!
//! # Errors
//!
//! Returns an `io::Error` if any directory entry cannot be read.

use std::path::{Path, PathBuf};

/// Discover manifest JSON files under `dir`.
///
/// Performs an iterative depth-first walk starting at `dir`. Returns a
/// sorted list of paths for every `.json` file found (recursively), excluding
/// `package.json`.
///
/// # Arguments
///
/// * `dir` — Root directory to scan. May contain subdirectories (categories).
///
/// # Errors
///
/// Returns `Err` if any [`std::fs::read_dir`] call fails (e.g. permission denied).
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

            if path.extension().is_some_and(|e| e == "json")
                && path.file_name().is_none_or(|n| n != "package.json")
            {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

/// Discover manifests and filter them by app patterns, returning
/// `(path, stem)` pairs.
///
/// Combines [`discover`] with Scoop's app-pattern filtering
/// ([`crate::internal::string::matches_any_glob`]): a first pattern of
/// `"*"` matches everything; otherwise each stem must match at least one
/// pattern (glob `*`/`?` supported, plain patterns match exactly).
///
/// Shared by the bucket-scanning commands `checkhashes` / `checkver` /
/// `checkurls`.
pub(crate) fn discover_matching(
    dir: &Path,
    app: &[String],
) -> std::io::Result<Vec<(PathBuf, String)>> {
    let mut out = Vec::new();
    for path in discover(dir)? {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let stem = stem.to_string();
        if crate::internal::string::matches_any_glob(&stem, app) {
            out.push((path, stem));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hok_mw_test_{}", label));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An empty directory returns an empty list.
    #[test]
    fn empty_dir_returns_empty() {
        let dir = tmpdir("empty");
        let result = discover(&dir).unwrap();
        assert!(
            result.is_empty(),
            "An empty directory should return an empty list."
        );
    }

    /// Top-level .json files were found (excluding package.json)。
    #[test]
    fn discovers_top_level_json_files() {
        let dir = tmpdir("top_level");
        fs::write(dir.join("app1.json"), "{}").unwrap();
        fs::write(dir.join("app2.json"), "{}").unwrap();
        fs::write(dir.join("package.json"), "{}").unwrap(); // should be excluded

        let result = discover(&dir).unwrap();
        assert_eq!(
            result.len(),
            2,
            "2 manifests should be found, package.json should be excluded"
        );
        let names: Vec<_> = result
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
            .collect();
        assert!(names.contains(&"app1.json"));
        assert!(names.contains(&"app2.json"));
        assert!(!names.contains(&"package.json"));
    }

    /// Manifest files in categorized subdirectories are also discovered recursively.
    #[test]
    fn discovers_manifests_in_subdirectories() {
        let dir = tmpdir("subdirs");
        let sub = dir.join("games");
        fs::create_dir_all(&sub).unwrap();
        fs::write(dir.join("tool.json"), "{}").unwrap();
        fs::write(sub.join("game1.json"), "{}").unwrap();

        let result = discover(&dir).unwrap();
        assert_eq!(result.len(), 2);
    }

    /// Non-.json files are ignored.
    #[test]
    fn ignores_non_json_files() {
        let dir = tmpdir("non_json");
        fs::write(dir.join("readme.md"), "").unwrap();
        fs::write(dir.join("script.ps1"), "").unwrap();
        fs::write(dir.join("app.json"), "{}").unwrap();

        let result = discover(&dir).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].file_name().unwrap() == "app.json");
    }

    /// Results are sorted by path.
    #[test]
    fn results_are_sorted() {
        let dir = tmpdir("sorted");
        fs::write(dir.join("zoo.json"), "{}").unwrap();
        fs::write(dir.join("alpha.json"), "{}").unwrap();
        fs::write(dir.join("mango.json"), "{}").unwrap();

        let result = discover(&dir).unwrap();
        assert_eq!(result.len(), 3);
        assert!(result[0] < result[1]);
        assert!(result[1] < result[2]);
    }

    /// Non-existent directory returns Err.
    #[test]
    fn nonexistent_dir_returns_error() {
        let dir = std::path::PathBuf::from("/tmp/hok_mw_test_nonexistent_dir_xyz_abc");
        let result = discover(&dir);
        assert!(
            result.is_err(),
            "A non-existent directory should return Err."
        );
    }

    // ── discover_matching ───────────────────────────────────────────────────

    #[test]
    fn matching_wildcard_returns_all_with_stems() {
        let dir = tmpdir("match_wildcard");
        fs::write(dir.join("app1.json"), "{}").unwrap();
        fs::write(dir.join("app2.json"), "{}").unwrap();

        let result = discover_matching(&dir, &["*".to_string()]).unwrap();
        assert_eq!(result.len(), 2);
        let stems: Vec<&str> = result.iter().map(|(_, s)| s.as_str()).collect();
        assert!(stems.contains(&"app1"));
        assert!(stems.contains(&"app2"));
    }

    #[test]
    fn matching_exact_pattern_filters() {
        let dir = tmpdir("match_exact");
        fs::write(dir.join("curl.json"), "{}").unwrap();
        fs::write(dir.join("git-lfs.json"), "{}").unwrap();

        // Plain patterns match exactly (no substring match).
        let result = discover_matching(&dir, &["git".to_string()]).unwrap();
        assert!(
            result.is_empty(),
            "exact pattern must not substring-match git-lfs"
        );

        let result = discover_matching(&dir, &["git*".to_string()]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, "git-lfs");
    }

    #[test]
    fn matching_empty_app_list_matches_nothing_without_panic() {
        let dir = tmpdir("match_empty");
        fs::write(dir.join("app1.json"), "{}").unwrap();

        // Empty app list must not panic (checkver used to index [0]).
        let result = discover_matching(&dir, &[]).unwrap();
        assert!(result.is_empty());
    }
}
