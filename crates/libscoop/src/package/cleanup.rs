//! Clean up old versions of installed packages.
//!
//! Removes all version directories except the current one for each package,
//! mirroring Scoop's `cleanup` command.

use crate::{error::Fallible, internal, Error, Session};

/// Cleanup old versions of packages.
///
/// Removes all version directories except the current one for each package.
/// If `names` is empty, cleans up all installed packages.
/// Returns a list of (package_name, removed_count, failed_count).
///
/// # Errors
///
/// A [`PackageNotFound`][1] error will be returned if a named package does not
/// exist and `ignore_failure` is false.
///
/// [1]: crate::Error::PackageNotFound
pub fn cleanup(
    session: &Session,
    names: &[String],
    ignore_failure: bool,
) -> Fallible<Vec<(String, usize, usize)>> {
    let apps_dir = session.effective_root_path().join("apps");
    let mut results = Vec::new();

    // If no names given, scan all installed packages
    let scan_names: Vec<String> = if names.is_empty() {
        let mut all = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&apps_dir) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    if let Some(name) = entry.file_name().to_str() {
                        all.push(name.to_owned());
                    }
                }
            }
        }
        all
    } else {
        names.to_vec()
    };

    for name in &scan_names {
        let pkg_dir = apps_dir.join(name);
        if !pkg_dir.exists() {
            if !ignore_failure {
                return Err(Error::PackageNotFound(name.to_owned()));
            }
            continue;
        }

        // Determine current version by reading the "current" symlink target
        let current_version = (|| -> Option<String> {
            let current_link = pkg_dir.join("current");
            std::fs::read_link(&current_link)
                .ok()?
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
        })();

        let Some(ref current_ver) = current_version else {
            // No install info — skip broken package
            continue;
        };

        // Collect old version directories
        let mut old_dirs = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&pkg_dir) {
            for entry in entries.flatten() {
                let fname = entry.file_name();
                let name_str = match fname.to_str() {
                    Some(s) => s,
                    None => continue,
                };
                // Skip "current" symlink and the current version
                if name_str == "current" || name_str == current_ver {
                    continue;
                }
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    old_dirs.push(name_str.to_owned());
                }
            }
        }

        if old_dirs.is_empty() {
            continue;
        }

        let mut removed = 0u32;
        let mut failed = 0u32;
        for ver in &old_dirs {
            let ver_dir = pkg_dir.join(ver);
            if let Err(e) = internal::fs::remove_dir(&ver_dir) {
                let msg = format!("failed to remove {} v{}: {}", name, ver, e);
                if ignore_failure {
                    eprintln!("{}", msg);
                    failed += 1;
                } else {
                    return Err(Error::Custom(msg));
                }
            } else {
                removed += 1;
            }
        }

        if removed > 0 || failed > 0 {
            results.push((name.clone(), removed as usize, failed as usize));
        }
    }

    Ok(results)
}
