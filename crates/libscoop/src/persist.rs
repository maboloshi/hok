use crate::{error::Fallible, internal, package::Package, Session};

/// Link persistent data for a package.
///
/// For each entry in `manifest.persist`:
/// - If persist target exists → link (user data preserved)
/// - If source exists → move to persist, then link
/// - If neither → create empty target (file if name has extension, else dir)
///
/// Directories → junction; files → hard link (Scoop-compatible).
pub fn link(session: &Session, package: &Package) -> Fallible<()> {
    let persists = match package.manifest().persist() {
        Some(p) => p,
        None => return Ok(()),
    };

    let config = session.config();
    let root = config.root_path();
    let app_dir = root.join("apps").join(package.name()).join("current");
    let persist_root = root.join("persist").join(package.name());

    for entry in &persists {
        let source = entry[0];
        let target = entry.get(1).unwrap_or(&entry[0]);

        let src_path = internal::path::normalize_path(app_dir.join(source));
        let tgt_path = internal::path::normalize_path(persist_root.join(target));

        // Ensure persist parent directory exists
        if let Some(parent) = tgt_path.parent() {
            internal::fs::ensure_dir(parent)?;
        }

        // Remove any pre-existing link at source
        let _ = internal::fs::remove_symlink(&src_path);
        let _ = std::fs::remove_file(&src_path);
        let _ = std::fs::remove_dir(&src_path);

        if tgt_path.exists() {
            // Persist data already exists
        } else if src_path.exists() {
            // First install — move default data to persist dir
            if let Some(parent) = tgt_path.parent() {
                internal::fs::ensure_dir(parent)?;
            }
            if src_path.is_dir() {
                std::fs::rename(&src_path, &tgt_path)?;
            } else {
                std::fs::copy(&src_path, &tgt_path)?;
                std::fs::remove_file(&src_path)?;
            }
        } else if has_extension(target) {
            // Neither exists but name looks like a file — create empty file
            if let Some(parent) = tgt_path.parent() {
                internal::fs::ensure_dir(parent)?;
            }
            std::fs::write(&tgt_path, &[])?;
        } else {
            // Neither exists — create empty directory (Scoop default)
            internal::fs::ensure_dir(&tgt_path)?;
        }

        // Create link: junction for dirs, hard link for files
        if tgt_path.is_dir() {
            internal::fs::symlink_dir(&tgt_path, &src_path)?;
        } else {
            std::fs::hard_link(&tgt_path, &src_path)?;
        }
    }

    Ok(())
}

/// Check if a persist entry name looks like a file (has extension).
fn has_extension(name: &str) -> bool {
    std::path::Path::new(name).extension().is_some()
}

/// Remove persistent data symlinks for a package (does NOT remove persist data).
pub fn unlink(session: &Session, package: &Package) -> Fallible<()> {
    assert!(package.is_installed());

    if let Some(persists) = package.manifest().persist() {
        let config = session.config();
        let root = config.root_path();
        let app_dir = root.join("apps").join(package.name()).join("current");

        for entry in &persists {
            let source = entry[0];
            let src_path = internal::path::normalize_path(app_dir.join(source));
            let _ = internal::fs::remove_symlink(&src_path);
        }
    }
    Ok(())
}
