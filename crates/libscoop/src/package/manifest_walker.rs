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

    /// 空目录返回空列表。
    #[test]
    fn empty_dir_returns_empty() {
        let dir = tmpdir("empty");
        let result = discover(&dir).unwrap();
        assert!(result.is_empty(), "空目录应返回空列表");
    }

    /// 顶级 .json 文件被发现（package.json 除外）。
    #[test]
    fn discovers_top_level_json_files() {
        let dir = tmpdir("top_level");
        fs::write(dir.join("app1.json"), "{}").unwrap();
        fs::write(dir.join("app2.json"), "{}").unwrap();
        fs::write(dir.join("package.json"), "{}").unwrap(); // should be excluded

        let result = discover(&dir).unwrap();
        assert_eq!(result.len(), 2, "应发现 2 个 manifest，package.json 应被排除");
        let names: Vec<_> = result
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
            .collect();
        assert!(names.contains(&"app1.json"));
        assert!(names.contains(&"app2.json"));
        assert!(!names.contains(&"package.json"));
    }

    /// 分类子目录中的 manifest 也被递归发现。
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

    /// 非 .json 文件被忽略。
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

    /// 结果按路径排序。
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

    /// 不存在的目录返回 Err。
    #[test]
    fn nonexistent_dir_returns_error() {
        let dir = std::path::PathBuf::from("/tmp/hok_mw_test_nonexistent_dir_xyz_abc");
        let result = discover(&dir);
        assert!(result.is_err(), "不存在的目录应返回 Err");
    }
}
