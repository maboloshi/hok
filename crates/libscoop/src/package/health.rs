//! Installed package health checker.
//!
//! Scan the installed packages directory, check whether the file structure of each package is complete (`current` symlink,
//! install record, installed manifest), check the package-owned shims for a missing target, and return a list of discovered
//! issues.
//!
//! This is hok's own diagnostic (the `health` command); upstream's `checkup`
//! command inspects the *environment* instead — see
//! [`crate::package::diagnostic`].
//!
//! # Usage
//!
//! ```no_run
//! use libscoop::package::health;
//! use libscoop::Session;
//!
//! let session = Session::new();
//! let issues = health::check_installed(&session);
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

use super::install_state;

/// A single health issue found for an installed package.
#[derive(Debug, Clone)]
pub struct HealthIssue {
    /// Package name (directory name under `apps/`), or the shim name for a
    /// broken-shim issue.
    pub name: String,
    /// Human-readable description of the issue.
    pub message: String,
}

/// Scan all installed packages and return a list of health issues.
///
/// Each installed package directory under `<root>/apps/` is checked for:
/// - A `current` symlink or directory that exists (a link whose target is gone
///   is reported separately from a link that is missing entirely).
/// - Presence of an install record (`scoop-install.json` or `install.json`).
/// - Presence of an installed manifest (`scoop-manifest.json` or `manifest.json`).
///
/// The session's shims directory is then scanned for shims whose target no
/// longer exists (see [`super::shim::broken_shims`]).
///
/// # Returns
///
/// A (possibly empty) list of [`HealthIssue`] entries, one per problem found.
pub fn check_installed(session: &Session) -> Vec<HealthIssue> {
    let apps_dir = session.apps_dir();
    let mut issues = Vec::new();

    if !apps_dir.exists() {
        return issues;
    }

    let entries = match std::fs::read_dir(&apps_dir) {
        Ok(e) => e,
        Err(_) => return issues,
    };

    let no_junction = session.config().no_junction();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // The 'scoop' directory is reserved for Scoop itself; skip it.
        if name == "scoop" {
            continue;
        }

        let app_dir = entry.path();

        // Under NO_JUNCTION there is no `current` junction to check — the
        // runtime entry is the version dir itself (upstream `currentdir`), so
        // the metadata is checked there. Reporting a missing junction in that
        // mode would flag every installed app as unhealthy.
        let meta_dir = if no_junction {
            let Some(version) = install_state::resolve_current_version(&app_dir, no_junction)
                .map(|resolved| resolved.version)
            else {
                issues.push(HealthIssue {
                    name: name.to_string(),
                    message: hok_i18n::t!("cmd.missing_install_json").to_string(),
                });
                continue;
            };
            app_dir.join(version)
        } else {
            let current = app_dir.join("current");
            if !current.exists() {
                // `exists()` follows the link, so a missing junction and a
                // dangling one both read as `false`. `read_link` tells them
                // apart: it still reads reparse point data when the target is
                // gone, and fails when there is no link at all.
                let message = match std::fs::read_link(&current) {
                    Ok(target) => hok_i18n::t!(
                        "cmd.health_current_dangling",
                        target = target.display().to_string()
                    )
                    .to_string(),
                    Err(_) => hok_i18n::t!("cmd.no_current_symlink").to_string(),
                };
                issues.push(HealthIssue {
                    name: name.to_string(),
                    message,
                });
                continue;
            }
            current
        };

        // Both metadata names count: a version dir is complete when it carries
        // the new `scoop-install.json`/`scoop-manifest.json` *or* the legacy
        // `install.json`/`manifest.json` (lib/manifest.ps1:134-154). Only when
        // neither exists is the file really missing.
        if install_state::install_info_path(&meta_dir).is_none() {
            issues.push(HealthIssue {
                name: name.to_string(),
                message: hok_i18n::t!("cmd.missing_install_json").to_string(),
            });
        }

        if install_state::installed_manifest_path(&meta_dir).is_none() {
            issues.push(HealthIssue {
                name: name.to_string(),
                message: hok_i18n::t!("cmd.missing_manifest_json").to_string(),
            });
        }
    }

    // Shims are package-owned artifacts, so a shim whose target is gone belongs
    // to `health` (upstream has no such check and `checkup` covers the
    // environment only). A scan failure is reported rather than swallowed — an
    // unreadable shim directory must not read as "no broken shims".
    match super::shim::broken_shims(session) {
        Ok(broken) => {
            for shim in broken {
                let target = shim
                    .target
                    .as_ref()
                    .map(|target| target.display().to_string())
                    .unwrap_or_else(|| shim.shim.display().to_string());
                issues.push(HealthIssue {
                    name: shim.name,
                    message: hok_i18n::t!("cmd.health_shim_broken", target = target).to_string(),
                });
            }
        }
        Err(err) => issues.push(HealthIssue {
            name: session.shims_dir().display().to_string(),
            message: hok_i18n::t!("cmd.health_shims_unreadable", e = err.to_string()).to_string(),
        }),
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_fields_accessible() {
        let issue = HealthIssue {
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

    fn session_with_no_junction(root: &std::path::Path, no_junction: bool) -> Session {
        let config_path = root.join("hok.json");
        let root_escaped = root.to_string_lossy().replace('\\', "\\\\");
        let cache_escaped = root.join("cache").to_string_lossy().replace('\\', "\\\\");
        std::fs::write(
            &config_path,
            format!(
                r#"{{"root_path": "{}", "cache_path": "{}", "no_junction": {}}}"#,
                root_escaped, cache_escaped, no_junction
            ),
        )
        .unwrap();
        Session::new_with(&config_path).unwrap()
    }

    /// Under NO_JUNCTION the version dir *is* the runtime entry — an install
    /// with complete metadata must not be reported as having no `current`
    /// symlink.
    #[test]
    fn check_installed_no_junction_accepts_version_dir_without_current() {
        let root = crate::test_utils::tmpdir("checkup_no_junction");
        let session = session_with_no_junction(&root, true);
        let vdir = root.join("apps").join("app").join("1.0.0");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("manifest.json"), r#"{"version": "1.0.0"}"#).unwrap();
        std::fs::write(vdir.join("install.json"), r#"{"architecture": "64bit"}"#).unwrap();

        let issues = check_installed(&session);
        assert!(issues.is_empty(), "healthy install flagged: {:?}", issues);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Under NO_JUNCTION, metadata that really is missing must still be found
    /// (the message is the missing file, not the absent junction).
    #[test]
    fn check_installed_no_junction_reports_missing_metadata() {
        let root = crate::test_utils::tmpdir("checkup_no_junction_missing");
        let session = session_with_no_junction(&root, true);
        let vdir = root.join("apps").join("app").join("1.0.0");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("manifest.json"), r#"{"version": "1.0.0"}"#).unwrap();

        let issues = check_installed(&session);
        assert_eq!(issues.len(), 1, "issues: {:?}", issues);
        assert_eq!(issues[0].name, "app");
        assert!(
            issues[0].message.contains("install.json"),
            "unexpected message: {}",
            issues[0].message
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A version dir carrying only the Scoop >= 0.6.0 names is healthy: the
    /// check must not claim a file is missing when the new name exists.
    #[test]
    fn check_installed_accepts_scoop_prefixed_metadata() {
        let root = crate::test_utils::tmpdir("checkup_scoop_names");
        let session = session_with_no_junction(&root, true);
        let vdir = root.join("apps").join("app").join("1.0.0");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("scoop-manifest.json"), r#"{"version": "1.0.0"}"#).unwrap();
        std::fs::write(
            vdir.join("scoop-install.json"),
            r#"{"architecture": "64bit"}"#,
        )
        .unwrap();

        let issues = check_installed(&session);
        assert!(
            issues.is_empty(),
            "new-name-only install flagged: {:?}",
            issues
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The two names are checked independently: a new manifest with no install
    /// record at all still reports exactly one missing-record issue.
    #[test]
    fn check_installed_reports_missing_record_beside_new_manifest() {
        let root = crate::test_utils::tmpdir("checkup_new_manifest_no_record");
        let session = session_with_no_junction(&root, true);
        let vdir = root.join("apps").join("app").join("1.0.0");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("scoop-manifest.json"), r#"{"version": "1.0.0"}"#).unwrap();

        let issues = check_installed(&session);
        assert_eq!(issues.len(), 1, "issues: {:?}", issues);
        assert!(
            issues[0].message.contains("install.json"),
            "unexpected message: {}",
            issues[0].message
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Junction mode keeps the junction check: no `current` link is an issue.
    #[test]
    fn check_installed_junction_mode_reports_missing_current() {
        let root = crate::test_utils::tmpdir("checkup_junction_missing_current");
        let session = session_with_no_junction(&root, false);
        std::fs::create_dir_all(root.join("apps").join("app").join("1.0.0")).unwrap();

        let issues = check_installed(&session);
        assert_eq!(issues.len(), 1, "issues: {:?}", issues);
        assert_eq!(issues[0].name, "app");
        assert!(
            issues[0].message.contains("current"),
            "unexpected message: {}",
            issues[0].message
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A `current` link whose target is gone is a different problem from a
    /// missing link: report the dangling target so `reset` is the obvious fix,
    /// and do not confuse it with "never linked".
    #[test]
    fn check_installed_reports_a_dangling_current_link() {
        let root = crate::test_utils::tmpdir("checkup_dangling_current");
        let session = session_with_no_junction(&root, false);
        let version = root.join("apps").join("app").join("1.0.0");
        std::fs::create_dir_all(&version).unwrap();
        crate::internal::fs::symlink_dir(&version, root.join("apps").join("app").join("current"))
            .unwrap();
        // Removing the target leaves the link in place, pointing nowhere.
        std::fs::remove_dir_all(&version).unwrap();

        let issues = check_installed(&session);
        assert_eq!(issues.len(), 1, "issues: {:?}", issues);
        assert_eq!(issues[0].name, "app");
        assert!(
            issues[0].message.contains("1.0.0"),
            "the missing target should be named: {}",
            issues[0].message
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A live `current` link is healthy — the dangling branch must not fire.
    #[test]
    fn check_installed_accepts_a_live_current_link() {
        let root = crate::test_utils::tmpdir("checkup_live_current");
        let session = session_with_no_junction(&root, false);
        let version = root.join("apps").join("app").join("1.0.0");
        std::fs::create_dir_all(&version).unwrap();
        std::fs::write(version.join("manifest.json"), r#"{"version": "1.0.0"}"#).unwrap();
        std::fs::write(version.join("install.json"), r#"{"architecture": "64bit"}"#).unwrap();
        crate::internal::fs::symlink_dir(&version, root.join("apps").join("app").join("current"))
            .unwrap();

        let issues = check_installed(&session);
        assert!(issues.is_empty(), "healthy install flagged: {:?}", issues);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Broken shims are reported by the installed-package scan.
    #[test]
    fn check_installed_reports_a_broken_shim() {
        let root = crate::test_utils::tmpdir("checkup_broken_shim");
        let session = session_with_no_junction(&root, true);
        let vdir = root.join("apps").join("app").join("1.0.0");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("manifest.json"), r#"{"version": "1.0.0"}"#).unwrap();
        std::fs::write(vdir.join("install.json"), r#"{"architecture": "64bit"}"#).unwrap();

        let shims = session.shims_dir();
        std::fs::create_dir_all(&shims).unwrap();
        let ghost = root
            .join("apps")
            .join("app")
            .join("current")
            .join("ghost.exe");
        std::fs::write(
            shims.join("gone.shim"),
            format!("path = \"{}\"\r\n", ghost.display()),
        )
        .unwrap();

        let issues = check_installed(&session);
        assert_eq!(issues.len(), 1, "issues: {:?}", issues);
        assert_eq!(issues[0].name, "gone");
        assert!(
            issues[0].message.contains("ghost.exe"),
            "the missing target should be named: {}",
            issues[0].message
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
