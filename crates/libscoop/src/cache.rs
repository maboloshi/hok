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
        let text = path.file_name().unwrap().to_str().unwrap();
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
                        if is_file {
                            if let Ok(item) = CacheFile::from(entry.path()) {
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
                    }
                    None
                })
                .collect::<Vec<_>>();
        }
    }

    Ok(files)
}

/// Remove cache files by query.
///
/// # Errors
///
/// I/O errors will be returned if the cache directory is not readable or failed
/// to remove the cache files.
pub fn remove(session: &crate::Session, query: &str) -> Fallible<()> {
    match query {
        "*" => {
            let config = session.config();
            Ok(crate::internal::fs::empty_dir(config.cache_path())?)
        }
        query => {
            let files = list(session, query)?;
            for f in files.into_iter() {
                std::fs::remove_file(f.path())?;
            }
            Ok(())
        }
    }
}
