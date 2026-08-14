//! Standalone `download` command implementation.
//!
//! Split from `package/download.rs`: resolves each query independently
//! (bare name, `bucket/name`, `name@version`, manifest URL, local path),
//! downloads into the cache without installing, and reports failures
//! per-package without aborting the rest.

use crate::constant::ISOLATED_PACKAGE_BUCKET;
use crate::{error::Fallible, Error, Event, Session};

use super::download::PackageSet;
use super::download_verify::download_and_verify;
use super::{identity, manifest_source, Package};

/// Options for [`download_apps`].
pub struct DownloadOptions {
    /// Ignore the cache and download again (`-f/--force`).
    pub force: bool,
    /// Verify downloaded files against the manifest hashes.
    pub check_hash: bool,
}
/// Download the files of the given packages into the cache directory
/// without installing them, mirroring upstream `scoop download`
/// (libexec/scoop-download.ps1).
///
/// Each query is resolved independently — bare name, `bucket/name`,
/// `name@version`, a manifest URL, or a local manifest path — and a
/// failure on one app does not abort the rest. No dependencies are
/// downloaded and no installation state is touched.
pub fn download_apps(session: &Session, queries: &[&str], opts: &DownloadOptions) -> Fallible<()> {
    let mut packages: Vec<Package> = Vec::new();

    for &query in queries {
        match resolve_download_query(session, query) {
            Ok(pkg) => {
                let version = pkg.version();
                if version.is_empty() {
                    session.output().error(format!(
                        "Manifest for '{}' doesn't specify a version.",
                        query
                    ));
                    continue;
                }
                if !version
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
                {
                    session.output().error(format!(
                        "Manifest version for '{}' has an unsupported character.",
                        query
                    ));
                    continue;
                }
                // Architecture support: no download URLs for the effective
                // architecture (upstream `Get-SupportedArchitecture`).
                if pkg.download_urls().is_empty() {
                    session.output().error(format!(
                        "'{}' doesn't support the current architecture!",
                        pkg.name()
                    ));
                    continue;
                }
                packages.push(pkg);
            }
            Err(e) => {
                session
                    .output()
                    .error(format!("failed to resolve '{}': {}", query, e));
            }
        }
    }

    if packages.is_empty() {
        // Nothing resolved — still end the event loop, or the CLI's
        // handle.join() would hang forever waiting for DownloadDone.
        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::DownloadDone);
        }
        return Ok(());
    }

    let refs: Vec<&Package> = packages.iter().collect();

    // HEAD requests fill in remote_size: fragmented download and the
    // progress bar both depend on it (same as the install pipeline).
    let result = (|| -> Fallible<()> {
        let mut set = PackageSet::new(session, &refs, !opts.force)?;
        set.calculate_download_size()?;
        let failed = download_and_verify(&mut set, !opts.check_hash, true, false)?;

        for pkg in &refs {
            if !failed.contains(&pkg.ident()) {
                session.output().info(format!(
                    "'{}' ({}) was downloaded successfully!",
                    pkg.name(),
                    pkg.version()
                ));
            }
        }

        if !failed.is_empty() {
            // Individual failures already printed per-package; surface an
            // aggregate error so the CLI exits non-zero (scripts/CI can tell).
            let mut failed: Vec<_> = failed.into_iter().collect();
            failed.sort();
            return Err(Error::Custom(format!(
                "download failed for: {}",
                failed.join(", ")
            )));
        }

        Ok(())
    })();

    // Always end the event loop — including error paths (set creation,
    // sizing, verify) — or the CLI's handle.join() hangs forever.
    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::DownloadDone);
    }

    result?;

    Ok(())
}

/// Resolve one download query to a [`Package`], mirroring the install
/// pipeline's dispatch: `name@version` → `generate_user_manifest`, anything
/// else → `resolve_manifest` (URL / local path / bucket lookup).
fn resolve_download_query(session: &Session, query: &str) -> Fallible<Package> {
    if let Some(pkg) = manifest_source::resolve_isolated_query(session, query)? {
        return Ok(pkg);
    }
    // Plain bucket reference: resolve directly — download has no
    // installed-package semantics, so no bucket scanning is needed.
    let aq = identity::parse_app(query).ok_or_else(|| Error::PackageNotFound(query.to_owned()))?;
    let resolved = manifest_source::resolve_manifest(session, &aq.app, aq.bucket.as_deref())?;
    Ok(Package::from(
        &resolved.name,
        ISOLATED_PACKAGE_BUCKET,
        resolved.manifest,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::download_fragmented::test_server::*;

    /// SHA-256 hex of `data`, as used in manifest `hash` fields.
    fn sha256_hex(data: &[u8]) -> String {
        let mut hasher = crate::internal::hash::ChecksumBuilder::new()
            .sha256()
            .build();
        hasher.consume(data);
        hasher.finalize()
    }

    /// Write a standalone manifest file for `name` pointing at `url`.
    fn write_manifest(
        dir: &std::path::Path,
        name: &str,
        version: &str,
        url: &str,
        hash: &str,
    ) -> std::path::PathBuf {
        let path = dir.join(format!("{}.json", name));
        let json = format!(
            r#"{{"version": "{}", "url": "{}", "hash": "{}"}}"#,
            version, url, hash
        );
        std::fs::write(&path, json).unwrap();
        path
    }

    /// Files in the session cache dir, sorted by name.
    fn cache_files(session: &Session) -> Vec<std::path::PathBuf> {
        let config = session.config();
        let dir = config.cache_path();
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.is_file())
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        files
    }
    #[test]
    fn test_download_apps_local_manifest() {
        let data = b"download-apps-test-data".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_basic");
        let session = crate::test_utils::test_session(&root);
        let url = format!("http://{}/app.bin", server.addr);
        let manifest = write_manifest(&root, "testapp", "1.0", &url, &sha256_hex(&data));

        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        download_apps(&session, &[manifest.to_str().unwrap()], &opts).unwrap();
        server.shutdown();

        let files = cache_files(&session);
        assert_eq!(files.len(), 1, "cache files: {:?}", files);
        assert!(files[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("testapp#1.0#"));
        assert_eq!(std::fs::read(&files[0]).unwrap(), data);
    }

    #[test]
    fn test_download_apps_hash_mismatch_removes_file() {
        let data = b"download-apps-mismatch".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_mismatch");
        let session = crate::test_utils::test_session(&root);
        let url = format!("http://{}/app.bin", server.addr);
        // Wrong hash: the downloaded file must be removed after verification,
        // and download_apps reports the failure (non-zero exit for callers).
        let manifest = write_manifest(&root, "badapp", "1.0", &url, &"0".repeat(64));

        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        let err = download_apps(&session, &[manifest.to_str().unwrap()], &opts)
            .expect_err("hash mismatch must be reported");
        assert!(
            err.to_string().contains("download failed for"),
            "unexpected error: {}",
            err
        );
        server.shutdown();

        assert!(
            cache_files(&session).is_empty(),
            "corrupt cache file must be removed"
        );
    }

    #[test]
    fn test_download_apps_no_hash_check_keeps_file() {
        let data = b"download-apps-no-hash".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_no_hash");
        let session = crate::test_utils::test_session(&root);
        let url = format!("http://{}/app.bin", server.addr);
        let manifest = write_manifest(&root, "nohashapp", "1.0", &url, &"0".repeat(64));

        let opts = DownloadOptions {
            force: false,
            check_hash: false,
        };
        download_apps(&session, &[manifest.to_str().unwrap()], &opts).unwrap();
        server.shutdown();

        assert_eq!(cache_files(&session).len(), 1);
    }

    #[test]
    fn test_download_apps_rejects_bad_version() {
        let root = crate::test_utils::tmpdir("download_apps_bad_version");
        let session = crate::test_utils::test_session(&root);

        // Missing version.
        let no_version = root.join("noversion.json");
        std::fs::write(&no_version, r#"{"url": "http://127.0.0.1:1/x.bin"}"#).unwrap();
        // Unsupported character in version.
        let bad_char = root.join("badchar.json");
        std::fs::write(
            &bad_char,
            r#"{"version": "1.0!", "url": "http://127.0.0.1:1/x.bin"}"#,
        )
        .unwrap();

        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        download_apps(
            &session,
            &[no_version.to_str().unwrap(), bad_char.to_str().unwrap()],
            &opts,
        )
        .unwrap();

        assert!(cache_files(&session).is_empty());
    }

    #[test]
    fn test_download_apps_one_bad_query_does_not_abort_others() {
        let data = b"download-apps-mixed".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_mixed");
        let session = crate::test_utils::test_session(&root);
        let url = format!("http://{}/app.bin", server.addr);
        let manifest = write_manifest(&root, "goodapp", "1.0", &url, &sha256_hex(&data));

        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        download_apps(
            &session,
            &[manifest.to_str().unwrap(), r"D:\nonexistent\app.json"],
            &opts,
        )
        .unwrap();
        server.shutdown();

        assert_eq!(cache_files(&session).len(), 1);
    }

    #[test]
    fn test_download_apps_force_redownloads() {
        let data = b"download-apps-force".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_force");
        let session = crate::test_utils::test_session(&root);
        let url = format!("http://{}/app.bin", server.addr);
        let manifest = write_manifest(&root, "forceapp", "1.0", &url, &sha256_hex(&data));
        let manifest_str = manifest.to_str().unwrap().to_owned();

        let cache_hit = DownloadOptions {
            force: false,
            check_hash: true,
        };
        download_apps(&session, &[&manifest_str], &cache_hit).unwrap();
        let requests_after_first = server.ranges.lock().unwrap().len();

        // Cache hit: no new request is sent.
        download_apps(&session, &[&manifest_str], &cache_hit).unwrap();
        assert_eq!(
            server.ranges.lock().unwrap().len(),
            requests_after_first,
            "cache hit must not re-download"
        );

        // Force: the file is downloaded again.
        let force = DownloadOptions {
            force: true,
            check_hash: true,
        };
        download_apps(&session, &[&manifest_str], &force).unwrap();
        server.shutdown();

        assert!(
            server.ranges.lock().unwrap().len() > requests_after_first,
            "force must re-download"
        );
        assert_eq!(cache_files(&session).len(), 1);
    }
}
