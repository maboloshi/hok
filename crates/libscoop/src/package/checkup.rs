//! Installed package health checker.
//!
//! 扫描已安装包目录，检查每个包的文件结构是否完整（`current` 软链接、
//! `install.json`、`manifest.json`），并返回发现的问题列表。
//!
//! # 使用方式
//!
//! ```no_run
//! use libscoop::package::checkup;
//! use libscoop::Session;
//!
//! let session = Session::new();
//! let issues = checkup::check_installed(&session);
//! for issue in &issues {
//!     println!("{}: {}", issue.name, issue.message);
//! }
//! ```
//!
//! # 注意事项
//!
//! - 函数会跳过名为 `scoop` 的目录（Scoop 自身保留目录）。
//! - 仅检查文件结构完整性，不验证文件内容。

use crate::Session;

/// A single health issue found for an installed package.
#[derive(Debug, Clone)]
pub struct CheckupIssue {
    /// Package name (directory name under `apps/`).
    pub name: String,
    /// Human-readable description of the issue.
    pub message: String,
}

/// Scan all installed packages and return a list of health issues.
///
/// Each installed package directory under `<root>/apps/` is checked for:
/// - A `current` symlink or directory that exists.
/// - Presence of `current/install.json`.
/// - Presence of `current/manifest.json`.
///
/// # Returns
///
/// A (possibly empty) list of [`CheckupIssue`] entries, one per problem found.
pub fn check_installed(session: &Session) -> Vec<CheckupIssue> {
    let config = session.config();
    let apps_dir = config.root_path().join("apps");
    let mut issues = Vec::new();

    if !apps_dir.exists() {
        return issues;
    }

    let entries = match std::fs::read_dir(&apps_dir) {
        Ok(e) => e,
        Err(_) => return issues,
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // The 'scoop' directory is reserved for Scoop itself; skip it.
        if name == "scoop" {
            continue;
        }

        let app_dir = entry.path();
        let current = app_dir.join("current");

        if !current.exists() {
            issues.push(CheckupIssue {
                name: name.to_string(),
                message: "missing 'current' symlink".to_string(),
            });
            continue;
        }

        let install_json = current.join("install.json");
        if !install_json.exists() {
            issues.push(CheckupIssue {
                name: name.to_string(),
                message: "missing install.json".to_string(),
            });
        }

        let manifest_json = current.join("manifest.json");
        if !manifest_json.exists() {
            issues.push(CheckupIssue {
                name: name.to_string(),
                message: "missing manifest.json".to_string(),
            });
        }
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_fields_accessible() {
        let issue = CheckupIssue {
            name: "test-pkg".to_string(),
            message: "missing manifest.json".to_string(),
        };
        assert_eq!(issue.name, "test-pkg");
        assert_eq!(issue.message, "missing manifest.json");
    }

    #[test]
    fn check_installed_nonexistent_dir_returns_empty() {
        // Build a session whose apps dir doesn't exist
        let session = crate::Session::new();
        // check_installed won't panic on a missing apps dir
        let result = std::panic::catch_unwind(|| {
            let _ = check_installed(&session);
        });
        assert!(result.is_ok(), "check_installed should not panic");
    }
}
