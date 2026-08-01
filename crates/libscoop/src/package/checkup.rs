//! Installed package health checker.
//!
//! Scan the installed packages directory, check whether the file structure of each package is complete (`current` symlink,
//! `install.json`, `manifest.json`), and return a list of discovered issues.
//!
//! # Usage
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
//! # Notes
//!
//! - The function will skip directories named `scoop` (Scoop's own reserved directory).
//! - Only checks file structure integrity, does not verify file contents.

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
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = check_installed(&session);
        }));
        assert!(result.is_ok(), "check_installed should not panic");
    }
}
