//! Manifest generation from a download URL.
//!
//! Downloads a file from a URL, computes its hash, and generates a manifest
//! skeleton — the business logic behind the `hok create` command.
//!
//! # Usage
//!
//! ```no_run
//! use libscoop::package::create;
//! use libscoop::Session;
//!
//! let session = Session::new();
//! let manifest = create::create_manifest(&session, "https://example.com/app.zip");
//! println!("{}", serde_json::to_string_pretty(&manifest.unwrap()).unwrap());
//! ```
//!
//! # Design
//!
//! The module owns all non-trivial logic: URL file-name extraction, archive
//! type detection, downloading, hash computation, and manifest skeleton
//! construction. The CLI layer only renders the resulting JSON.

use serde_json::Value;

use crate::{error::Fallible, network, Session};

/// Extract the file name from a download URL, stripping any query string.
fn extract_filename(url: &str) -> &str {
    url.rsplit('/')
        .next()
        .and_then(|s| s.split('?').next())
        .filter(|s| !s.is_empty())
        .unwrap_or("download")
}

/// Derive an app name from the file name by stripping the extension.
fn derive_name(filename: &str) -> &str {
    filename
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(filename)
}

/// Detect whether the file name denotes a (supported) archive.
fn is_archive(filename: &str) -> bool {
    filename.ends_with(".zip")
        || filename.ends_with(".7z")
        || filename.ends_with(".tar.gz")
        || filename.ends_with(".tgz")
        || filename.ends_with(".tar.xz")
        || filename.ends_with(".tar.bz2")
        || filename.ends_with(".tar")
        || filename.ends_with(".gz")
        || filename.ends_with(".bz2")
        || filename.ends_with(".xz")
        || filename.ends_with(".zst")
        || filename.ends_with(".rar")
}

/// Create a manifest skeleton from a download URL.
///
/// Downloads the file to a temporary location, computes its SHA-256 hash, and
/// returns a manifest JSON value. Non-archive downloads get a `bin` entry
/// derived from the file name.
///
/// # Errors
///
/// Returns an error if the download or the hash computation fails.
pub fn create_manifest(session: &Session, url: &str) -> Fallible<Value> {
    let url = url.trim();
    let filename = extract_filename(url);
    let name = derive_name(filename);
    let archive = is_archive(filename);

    // Download to a temp location and compute the hash.
    let tmp_dir = std::env::temp_dir().join("hok-create");
    std::fs::create_dir_all(&tmp_dir)?;
    let dest = tmp_dir.join(filename);

    let result = (|| -> Fallible<Value> {
        network::download_file(session, url, &dest)?;
        let hash = crate::internal::hash::compute_file_hash(&dest, "sha256")?;

        let mut manifest = serde_json::json!({
            "version": "0.0.0",
            "description": format!("{} description", name),
            "homepage": "https://example.com",
            "license": "Unknown",
            "url": url,
            "hash": hash,
        });

        if !archive {
            manifest
                .as_object_mut()
                .expect("json! object")
                .insert("bin".to_string(), serde_json::json!([name]));
        }

        Ok(manifest)
    })();

    let _ = std::fs::remove_dir_all(&tmp_dir);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_filename_basic() {
        assert_eq!(extract_filename("https://example.com/app.zip"), "app.zip");
    }

    #[test]
    fn extract_filename_strips_query() {
        assert_eq!(
            extract_filename("https://example.com/app.zip?download=1"),
            "app.zip"
        );
    }

    #[test]
    fn extract_filename_trailing_slash_falls_back() {
        assert_eq!(extract_filename("https://example.com/dir/"), "download");
    }

    #[test]
    fn derive_name_strips_extension() {
        assert_eq!(derive_name("app.zip"), "app");
    }

    #[test]
    fn derive_name_dotted_name() {
        assert_eq!(derive_name("my.app.v1.0.0.zip"), "my.app.v1.0.0");
    }

    #[test]
    fn derive_name_no_extension() {
        assert_eq!(derive_name("app"), "app");
    }

    #[test]
    fn is_archive_detects_common_types() {
        for name in ["app.zip", "app.7z", "app.tar.gz", "app.tgz", "app.rar"] {
            assert!(is_archive(name), "{name} should be an archive");
        }
    }

    #[test]
    fn is_archive_rejects_binaries() {
        for name in ["app.exe", "app.msi", "app"] {
            assert!(!is_archive(name), "{name} should not be an archive");
        }
    }
}
