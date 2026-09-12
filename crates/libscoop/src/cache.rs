//! Scoop download cache file representation.
//!
//! Cache files follow the naming convention `app#version#filenamified_url`.
//! The [`CacheFile`] struct parses and validates this pattern, providing
//! typed accessors for `package_name()`, `version()`, and `path()`.

use std::path::{Path, PathBuf};

use crate::constant::REGEX_CACHE_FILE;
use crate::error::{Error, Fallible};

/// Scoop cache file representation
#[derive(Clone, Debug)]
pub struct CacheFile {
    path: PathBuf,
}

impl CacheFile {
    pub fn from(path: PathBuf) -> Fallible<CacheFile> {
        // Non-UTF-8 names cannot match the `app#version#url` cache naming
        // convention — reject them as invalid cache files.
        let Some(text) = path.file_name().and_then(|n| n.to_str()) else {
            return Err(Error::InvalidCacheFile { path });
        };
        match REGEX_CACHE_FILE.is_match(text) {
            false => Err(Error::InvalidCacheFile { path }),
            true => Ok(CacheFile { path }),
        }
    }

    /// Get path of this cache file
    #[inline]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get file name of this cache file
    #[inline]
    pub fn file_name(&self) -> &str {
        self.path.file_name().unwrap().to_str().unwrap()
    }

    /// Get package name of this cache file
    #[inline]
    pub fn package_name(&self) -> &str {
        self.file_name().split_once('#').map(|s| s.0).unwrap()
    }

    /// Get version of this cache file
    #[inline]
    pub fn version(&self) -> &str {
        self.file_name().splitn(3, '#').collect::<Vec<_>>()[1]
    }
}

// ─── Session operations ────────────────────────────────────────────────────

/// Get a list of downloaded cache files.
///
/// # Returns
///
/// A list of downloaded cache files.
///
/// # Errors
///
/// I/O errors will be returned if the cache directory is not readable.
pub fn list(session: &crate::Session, query: &str) -> Fallible<Vec<CacheFile>> {
    let is_wildcard_query = query.eq("*") || query.is_empty();
    let config = session.config();
    let cache_dir = config.cache_path();
    let mut files = vec![];

    match cache_dir.read_dir() {
        Err(err) => {
            tracing::debug!("failed to read cache dir (err: {})", err);
        }
        Ok(entires) => {
            files = entires
                .filter_map(|de| {
                    if let Ok(entry) = de {
                        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
                        if is_file && let Ok(item) = CacheFile::from(entry.path()) {
                            if !is_wildcard_query {
                                let matched = item
                                    .package_name()
                                    .to_lowercase()
                                    .contains(&query.to_lowercase());
                                if matched {
                                    return Some(item);
                                } else {
                                    return None;
                                }
                            }

                            return Some(item);
                        }
                    }
                    None
                })
                .collect::<Vec<_>>();
        }
    }

    sort_listing(&mut files);

    Ok(files)
}

/// Order a listing by package name (case-insensitively) then version — the
/// order upstream's `Get-ChildItem` presents, and the one that makes a listing
/// reproducible: `read_dir` returns an arbitrary order, so without this the
/// rows (and therefore a rendered table's column widths) would differ between
/// filesystems and runs.
fn sort_listing(files: &mut [CacheFile]) {
    files.sort_by(|a, b| {
        a.package_name()
            .to_lowercase()
            .cmp(&b.package_name().to_lowercase())
            .then_with(|| a.version().cmp(b.version()))
            .then_with(|| a.file_name().cmp(b.file_name()))
    });
}

/// Remove cache files by query.
///
/// Files that are already gone are silently skipped. Files that fail to
/// remove (e.g. locked by a running process on Windows) are reported through
/// the session output channel and skipped, so a single locked file never
/// aborts the batch.
///
/// # Errors
///
/// I/O errors will be returned if the cache directory is not readable.
pub fn remove(session: &crate::Session, query: &str) -> Fallible<()> {
    match query {
        "*" => {
            let config = session.config();
            let cache_dir = config.cache_path();
            if !cache_dir.exists() {
                return Ok(());
            }
            for entry in cache_dir.read_dir()? {
                let entry = entry?;
                let path = entry.path();
                let result = if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
                if let Err(e) = result {
                    // Already gone — treat as success (Scoop's
                    // Remove-Item -Force semantics).
                    if e.kind() != std::io::ErrorKind::NotFound {
                        session.output().error(hok_i18n::t!(
                            "cmd.failed_remove_cache",
                            path = path.display(),
                            error = e
                        ));
                    }
                }
            }
            Ok(())
        }
        query => {
            let files = list(session, query)?;
            for f in files.into_iter() {
                if let Err(e) = std::fs::remove_file(f.path())
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    session.output().error(hok_i18n::t!(
                        "cmd.failed_remove_cache",
                        path = f.path().display(),
                        error = e
                    ));
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file(name: &str) -> CacheFile {
        CacheFile::from(PathBuf::from(name)).expect("valid cache file name")
    }

    fn identity(f: &CacheFile) -> String {
        format!("{}#{}", f.package_name(), f.version())
    }

    /// A listing is ordered by package name (case-insensitively) then version,
    /// regardless of the order the directory returned.
    #[test]
    fn listing_is_ordered_by_name_then_version() {
        let mut files = vec![
            file("zstd#1.5.7#a.zip"),
            file("Brave#1.94.121#b.zip"),
            file("7zip#26.03#c.msi"),
            file("croc#11.5.0#d.zip"),
            file("croc#11.4.0#e.zip"),
        ];

        sort_listing(&mut files);

        let order = files.iter().map(identity).collect::<Vec<_>>();
        assert_eq!(
            order,
            [
                "7zip#26.03",
                "Brave#1.94.121",
                "croc#11.4.0",
                "croc#11.5.0",
                "zstd#1.5.7"
            ]
        );
    }

    /// Two files of the same package and version (different URLs) still get a
    /// deterministic order.
    #[test]
    fn listing_is_stable_for_equal_name_and_version() {
        let mut files = vec![file("croc#1.0#z.zip"), file("croc#1.0#a.zip")];
        sort_listing(&mut files);

        let order = files
            .iter()
            .map(|f| f.file_name().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(order, ["croc#1.0#a.zip", "croc#1.0#z.zip"]);
    }
}
