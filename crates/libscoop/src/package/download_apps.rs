//! Standalone `download` command implementation.
//!
//! Split from `package/download.rs`: resolves each query independently
//! (bare name, `bucket/name`, `name@version`, manifest URL, local path),
//! downloads into the cache without installing, and reports failures
//! per-package without aborting the rest.

use crate::constant::ISOLATED_PACKAGE_BUCKET;
use crate::{Error, Event, Session, error::Fallible};

use super::download::PackageSet;
use super::download_verify::download_and_verify;
use super::{Package, identity, manifest_source};

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
pub async fn download_apps(
    session: &Session,
    queries: &[&str],
    opts: &DownloadOptions,
) -> Fallible<()> {
    let mut packages: Vec<Package> = Vec::new();

    for &query in queries {
        match resolve_download_query(session, query).await {
            Ok(pkg) => {
                let version = pkg.version();
                if version.is_empty() {
                    session
                        .output()
                        .error(hok_i18n::t!("cmd.manifest_no_version", name = query));
                    continue;
                }
                if !version
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
                {
                    session
                        .output()
                        .error(hok_i18n::t!("cmd.manifest_version_invalid", name = query));
                    continue;
                }
                // Architecture support: no download URLs for the effective
                // architecture (upstream `Get-SupportedArchitecture`).
                if pkg.download_urls().is_empty() {
                    session
                        .output()
                        .error(hok_i18n::t!("cmd.arch_unsupported", name = pkg.name()));
                    continue;
                }
                packages.push(pkg);
            }
            Err(e) => {
                session.output().error(hok_i18n::t!(
                    "cmd.failed_resolve",
                    query = query,
                    error = e
                ));
            }
        }
    }

    if packages.is_empty() {
        // Nothing resolved — still end the event loop, or the CLI's
        // handle.join() would hang forever waiting for DownloadDone.
        if let Some(tx) = session.emitter() {
            let _ = tx.try_send(Event::DownloadDone);
        }
        return Ok(());
    }

    let refs: Vec<&Package> = packages.iter().collect();

    // HEAD requests fill in remote_size: fragmented download and the
    // progress bar both depend on it (same as the install pipeline).
    let result: Fallible<()> = async {
        let mut set = PackageSet::new(session, &refs, !opts.force)?;
        set.calculate_download_size().await?;
        let failed = download_and_verify(&mut set, !opts.check_hash, true, false).await?;

        for pkg in &refs {
            if !failed.contains(&pkg.ident()) {
                session.output().info(hok_i18n::t!(
                    "cmd.downloaded_success",
                    name = pkg.name(),
                    version = pkg.version()
                ));
            }
        }

        if !failed.is_empty() {
            // Individual failures were already printed per-package; the
            // batch summary is informational. Scoop keeps downloading the
            // remaining apps and still exits 0 (`scoop-download.ps1:110-143`),
            // so no aggregate error is raised.
            let mut failed: Vec<_> = failed.into_iter().collect();
            failed.sort();
            session.output().error(hok_i18n::t!(
                "cmd.download_failed_list",
                names = failed.join(", ")
            ));
        }

        Ok(())
    }
    .await;

    // Always end the event loop — including error paths (set creation,
    // sizing, verify) — or the CLI's handle.join() hangs forever.
    if let Some(tx) = session.emitter() {
        let _ = tx.try_send(Event::DownloadDone);
    }

    result?;

    Ok(())
}

/// Resolve one download query to a [`Package`], mirroring the install
/// pipeline's dispatch: `name@version` → `generate_user_manifest`, anything
/// else → `get_manifest` (URL / local path / installed record / bucket lookup).
async fn resolve_download_query(session: &Session, query: &str) -> Fallible<Package> {
    if let Some(pkg) = manifest_source::resolve_isolated_query(session, query).await? {
        return Ok(pkg);
    }
    // Plain bucket reference: resolve through the one `Get-Manifest` entry.
    let aq = identity::parse_app(query).ok_or_else(|| Error::PackageNotFound(query.to_owned()))?;
    let resolved = manifest_source::get_manifest(session, &aq.app, aq.bucket.as_deref()).await?;
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
        cache_files_in(config.cache_path())
    }

    /// Capture every `Output::Error` the session emits.
    fn capture_errors(session: &Session) -> std::sync::Arc<std::sync::Mutex<Vec<String>>> {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = {
            let seen = seen.clone();
            std::sync::Arc::new(move |out: crate::output::Output| {
                if let crate::output::Output::Error(msg) = out {
                    seen.lock().unwrap().push(msg);
                }
            })
        };
        session.set_output(sink).unwrap();
        seen
    }

    /// A download that fails under IgnoreFailures is reported as failed on the
    /// line — its bytes never reach the total, so a percentage would freeze.
    #[tokio::test]
    async fn test_download_apps_reports_a_failed_download() {
        use crate::package::download::VerifyState;

        // Every GET answers 500, so sizing succeeds and the transfer fails.
        let data = b"failed-download".to_vec();
        let mut server = spawn_range_server(data, u32::MAX, 500, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_failed_download");
        let session = crate::test_utils::test_session(&root);
        let url = format!("http://{}/app.bin", server.addr);
        let manifest = write_manifest(&root, "deadapp", "1.0", &url, &"0".repeat(64));
        let manifest = manifest.to_string_lossy().into_owned();

        let mut rx = session
            .event_bus()
            .receiver()
            .expect("event bus must be free");
        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        download_apps(&session, &[&manifest], &opts).await.unwrap();
        server.shutdown();

        let mut states = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let Event::PackageVerifyProgress(ctx) = ev {
                states.push((ctx.ident, ctx.state));
            }
        }
        assert!(
            states
                .iter()
                .any(|(ident, state)| ident.ends_with("/deadapp") && *state == VerifyState::Failed),
            "the failed download must be marked failed: {states:?}"
        );
    }

    /// Number of captured messages that report a hash mismatch for `name`.
    fn mismatch_reports(seen: &std::sync::Arc<std::sync::Mutex<Vec<String>>>, name: &str) -> usize {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|m| m.contains("Hash mismatch for package") && m.contains(name))
            .count()
    }

    /// A corrupt file of a *dispatched* package is hashed exactly once (inline)
    /// and reported once, not once inline plus once in the tail pass.
    #[tokio::test]
    async fn test_download_apps_hashes_a_downloaded_file_once() {
        let data = b"hashed-once".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_hashed_once");
        let session = crate::test_utils::test_session(&root);
        let seen = capture_errors(&session);
        let url = format!("http://{}/app.bin", server.addr);
        let m = write_manifest(&root, "hashedonce", "1.0", &url, &"0".repeat(64));

        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        download_apps(&session, &[m.to_str().unwrap()], &opts)
            .await
            .unwrap();
        server.shutdown();

        assert_eq!(
            mismatch_reports(&seen, "hashedonce"),
            1,
            "one report only: {:?}",
            seen.lock().unwrap()
        );
        assert!(cache_files(&session).is_empty());
    }

    /// A fully cached package never reaches a download task, so its corrupt file
    /// is reported exactly once — by the tail pass.
    #[tokio::test]
    async fn test_download_apps_hashes_a_cache_hit_once() {
        let data = b"cache-hit-once".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_cache_hit_once");
        let session = crate::test_utils::test_session(&root);
        let url = format!("http://{}/app.bin", server.addr);
        let m = write_manifest(&root, "cachehit", "1.0", &url, &sha256_hex(&data));
        let m = m.to_string_lossy().into_owned();

        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        download_apps(&session, &[&m], &opts).await.unwrap();
        let files = cache_files(&session);
        std::fs::write(&files[0], vec![b'X'; data.len()]).unwrap();

        let seen = capture_errors(&session);
        download_apps(&session, &[&m], &opts).await.unwrap();
        server.shutdown();

        assert_eq!(
            mismatch_reports(&seen, "cachehit"),
            1,
            "one report only: {:?}",
            seen.lock().unwrap()
        );
        assert!(cache_files(&session).is_empty());
    }

    /// A fully cached package is announced once per package, with its readable
    /// identity — not once per cached file, and not the cache file name
    /// (`<name>#<version>#<sha7><ext>`), which the frontend cannot interpret.
    #[tokio::test]
    async fn test_download_apps_reports_a_cache_hit_once_per_package() {
        let data = b"cache-hit-identity".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_cache_hit_identity");
        let session = crate::test_utils::test_session(&root);
        let (a, b) = (
            format!("http://{}/a.bin", server.addr),
            format!("http://{}/b.bin", server.addr),
        );
        // Two files: the per-file emission this replaced would have announced
        // the same package twice.
        let json = format!(
            r#"{{"version": "2.5", "url": ["{a}", "{b}"], "hash": ["{}", "{}"]}}"#,
            sha256_hex(&data),
            sha256_hex(&data)
        );
        let path = root.join("identityapp.json");
        std::fs::write(&path, json).unwrap();
        let query = path.to_str().unwrap();

        // The receiver must be taken before the first run: the bounded channel
        // fills up unattended and `try_send` then drops events silently.
        let mut rx = session
            .event_bus()
            .receiver()
            .expect("event bus must be free");
        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        download_apps(&session, &[query], &opts).await.unwrap();
        while rx.try_recv().is_ok() {}

        download_apps(&session, &[query], &opts).await.unwrap();
        server.shutdown();

        let hits: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok())
            .filter_map(|ev| match ev {
                Event::PackageCacheHit {
                    ident,
                    name,
                    version,
                } => Some((ident, name, version)),
                _ => None,
            })
            .collect();
        assert_eq!(hits.len(), 1, "one event per cached package: {hits:?}");
        assert_eq!(hits[0].1, "identityapp");
        assert_eq!(hits[0].2, "2.5");
        assert!(hits[0].0.ends_with("/identityapp"), "{:?}", hits[0]);
    }

    /// The hard-failure path (install's default): a hash mismatch aborts the
    /// operation and still removes the corrupt file.
    #[tokio::test]
    async fn test_download_and_verify_hard_failure_aborts() {
        use crate::package::Package;
        use crate::package::download::{PackageSet, download_and_verify};
        use crate::package::manifest::Manifest;

        let data = b"hard-failure".to_vec();
        let mut server = spawn_range_server(data, 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_and_verify_hard_failure");
        let session = crate::test_utils::test_session(&root);
        let url = format!("http://{}/app.bin", server.addr);
        let manifest = Manifest::from_json(
            "hardapp",
            &format!(
                r#"{{"version": "1.0", "url": "{}", "hash": "{}"}}"#,
                url,
                "0".repeat(64)
            ),
        )
        .unwrap();
        let pkg = Package::from("hardapp", "test", manifest);
        let packages = [&pkg];

        let mut set = PackageSet::new(&session, &packages, false).unwrap();
        set.calculate_download_size().await.unwrap();
        let result = download_and_verify(&mut set, false, false, false).await;
        server.shutdown();

        assert!(result.is_err(), "hard failure must abort: {result:?}");
        assert!(
            cache_files(&session).is_empty(),
            "corrupt file removed even on the hard path"
        );
    }

    /// A partially cached package hashes its per-file cache hit (inline, in the
    /// skip branch) as well as the file it downloads, keeps going after the
    /// failure and reports once.
    #[tokio::test]
    async fn test_download_apps_verifies_a_partially_cached_package() {
        let data = b"partial-cache".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_partial_cache");
        let session = crate::test_utils::test_session(&root);
        let (a, b) = (
            format!("http://{}/a.bin", server.addr),
            format!("http://{}/b.bin", server.addr),
        );
        let json = format!(
            r#"{{"version": "1.0", "url": ["{a}", "{b}"], "hash": ["{}", "{}"]}}"#,
            sha256_hex(&data),
            sha256_hex(&data)
        );
        let path = root.join("partapp.json");
        std::fs::write(&path, json).unwrap();

        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        // First run: both files land, so the next run is a partial cache.
        download_apps(&session, &[path.to_str().unwrap()], &opts)
            .await
            .unwrap();
        let files = cache_files(&session);
        assert_eq!(files.len(), 2, "two cache files: {files:?}");

        // Corrupt one entry and drop the other: the package is then partial, so
        // it is dispatched with one per-file cache hit. The cache names are
        // derived from the URL, so the two files are told apart by position.
        std::fs::write(&files[0], vec![b'X'; data.len()]).unwrap();
        std::fs::remove_file(&files[1]).unwrap();

        let seen = capture_errors(&session);
        download_apps(&session, &[path.to_str().unwrap()], &opts)
            .await
            .unwrap();
        server.shutdown();

        assert_eq!(
            mismatch_reports(&seen, "partapp"),
            1,
            "the corrupt per-file cache hit is reported once: {:?}",
            seen.lock().unwrap()
        );
        let left = cache_files(&session);
        assert_eq!(
            left.len(),
            1,
            "only the re-downloaded file is left: {left:?}"
        );
        assert_eq!(
            std::fs::read(&left[0]).unwrap(),
            data,
            "the re-downloaded file is intact"
        );
    }

    /// Files directly inside `dir`, sorted by name.
    fn cache_files_in(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
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

    /// The *finalized* cache file for `prefix` among `files`, if present.
    ///
    /// A `.download` temp file does not count: it exists from the first byte
    /// and is only renamed once the file itself is complete.
    fn finalized_cache_file<'a>(
        files: &'a [std::path::PathBuf],
        prefix: &str,
    ) -> Option<&'a std::path::PathBuf> {
        files.iter().find(|path| {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            name.starts_with(prefix) && !name.ends_with(".download")
        })
    }
    #[tokio::test]
    async fn test_download_apps_local_manifest() {
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
        download_apps(&session, &[manifest.to_str().unwrap()], &opts)
            .await
            .unwrap();
        server.shutdown();

        let files = cache_files(&session);
        assert_eq!(files.len(), 1, "cache files: {:?}", files);
        assert!(
            files[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("testapp#1.0#")
        );
        assert_eq!(std::fs::read(&files[0]).unwrap(), data);
    }

    /// A package whose download finishes must be finalized (moved to its
    /// cache name) immediately, not once every sibling in the batch finished:
    /// upstream moves each file as soon as its own transfer completes
    /// (`lib/download.ps1:58-59`).
    #[tokio::test]
    async fn test_download_apps_finalizes_completed_package_immediately() {
        let fast_data = b"download-apps-fast".to_vec();
        let slow_data = b"download-apps-slow".to_vec();
        let mut fast = spawn_range_server(fast_data.clone(), 0, 0, false, false, None);
        // The slow server withholds its body long enough that the fast
        // package must become observable in the cache while it is in flight.
        let mut slow = spawn_range_server_delayed(
            slow_data.clone(),
            0,
            0,
            false,
            false,
            None,
            std::time::Duration::from_secs(30),
        );
        let root = crate::test_utils::tmpdir("download_apps_finalize_each");
        let session = crate::test_utils::test_session(&root);
        let cache_dir = session.config().cache_path().to_path_buf();

        let fast_manifest = write_manifest(
            &root,
            "fastapp",
            "1.0",
            &format!("http://{}/app.bin", fast.addr),
            &sha256_hex(&fast_data),
        );
        let slow_manifest = write_manifest(
            &root,
            "slowapp",
            "1.0",
            &format!("http://{}/app.bin", slow.addr),
            &sha256_hex(&slow_data),
        );
        let queries = [
            fast_manifest.to_string_lossy().into_owned(),
            slow_manifest.to_string_lossy().into_owned(),
        ];

        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        let refs: Vec<&str> = queries.iter().map(|s| s.as_str()).collect();
        // Not `tokio::spawn`: `Package` is not `Send` (it holds a `OnceCell`),
        // so the batch future is polled in place and dropped to cancel it.
        let mut download = std::pin::pin!(download_apps(&session, &refs, &opts));

        // The fast package must be finalized while the slow one has not
        // received a single body byte yet (both download concurrently, `jobs`
        // defaults to 4). Only the *finalized* file counts — the `.download`
        // temp file exists from the first byte either way.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut files = cache_files_in(&cache_dir);
        let mut finished_early = false;
        while finalized_cache_file(&files, "fastapp#").is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "fast package was never finalized: {:?}",
                files
            );
            tokio::select! {
                biased;
                _ = &mut download => {
                    // The whole batch finished; the fast package was only
                    // finalized together with (or after) the slow one.
                    finished_early = true;
                    break;
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {
                    files = cache_files_in(&cache_dir);
                }
            }
        }
        assert!(
            !finished_early,
            "the fast package was not finalized before the batch finished"
        );
        assert!(
            finalized_cache_file(&files, "slowapp#").is_none(),
            "the slow package must still be downloading: {:?}",
            files
        );
        let fast_file = finalized_cache_file(&files, "fastapp#").unwrap();
        assert_eq!(std::fs::read(fast_file).unwrap(), fast_data);

        // The batch is left unpolled from here: it is cancelled when the
        // pinned future drops at the end of the test, so the slow download
        // never completes (its server withholds the body for 30 s).
        fast.shutdown();
        slow.shutdown();
    }

    /// Inline verification: a corrupt file is removed as soon as its own
    /// package's check runs — while the sibling package's download is still in
    /// flight. The batched pass could only ever have removed it after every
    /// download finished, which the delayed sibling server rules out.
    #[tokio::test]
    async fn test_download_apps_verifies_while_sibling_downloads() {
        use crate::package::download::VerifyState;

        let fast_data = b"inline-verify-fast".to_vec();
        let slow_data = b"inline-verify-slow".to_vec();
        let mut fast = spawn_range_server(fast_data.clone(), 0, 0, false, false, None);
        let mut slow = spawn_range_server_delayed(
            slow_data.clone(),
            0,
            0,
            false,
            false,
            None,
            std::time::Duration::from_secs(30),
        );
        let root = crate::test_utils::tmpdir("download_apps_inline_verify");
        let session = crate::test_utils::test_session(&root);
        let cache_dir = session.config().cache_path().to_path_buf();

        // Wrong hash on purpose: the inline check must fail and remove it.
        let bad = write_manifest(
            &root,
            "badfast",
            "1.0",
            &format!("http://{}/app.bin", fast.addr),
            &"0".repeat(64),
        );
        let good = write_manifest(
            &root,
            "slowapp",
            "1.0",
            &format!("http://{}/app.bin", slow.addr),
            &sha256_hex(&slow_data),
        );
        let queries = [
            bad.to_string_lossy().into_owned(),
            good.to_string_lossy().into_owned(),
        ];

        let mut rx = session
            .event_bus()
            .receiver()
            .expect("event bus must be free");
        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        let refs: Vec<&str> = queries.iter().map(|s| s.as_str()).collect();
        let mut download = std::pin::pin!(download_apps(&session, &refs, &opts));

        // Wait for the failure event itself: it has to arrive before the
        // sibling's download ends, otherwise verification only ran in a batch.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            tokio::select! {
                biased;
                ev = rx.recv() => match ev {
                    Some(Event::PackageVerifyProgress(ctx))
                        if ctx.ident.ends_with("/badfast")
                            && ctx.state == VerifyState::Failed =>
                    {
                        // The file is removed before the event is emitted.
                        let files = cache_files_in(&cache_dir);
                        assert!(
                            finalized_cache_file(&files, "badfast#").is_none(),
                            "the corrupt file must be gone mid-flight: {files:?}"
                        );
                        assert!(
                            finalized_cache_file(&files, "slowapp#").is_none(),
                            "the sibling package must still be downloading: {files:?}"
                        );
                        break;
                    }
                    Some(_) => {}
                    None => panic!("event bus closed before the verify event"),
                },
                _ = &mut download => {
                    panic!("the batch finished before inline verification")
                }
                _ = tokio::time::sleep_until(deadline) => {
                    panic!("no inline verification event arrived")
                }
            }
        }

        // The batch is left unpolled from here and cancelled when the pinned
        // future drops at the end of the test — the slow server never answers.
        fast.shutdown();
        slow.shutdown();
    }

    /// A fully cached package never reaches a download task, so the batched
    /// tail pass must still catch a corrupt cache file on a cache hit.
    #[tokio::test]
    async fn test_download_apps_verifies_cache_hit_on_the_tail_pass() {
        let data = b"cache-hit-verify".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_cache_hit_verify");
        let session = crate::test_utils::test_session(&root);
        let url = format!("http://{}/app.bin", server.addr);
        let manifest = write_manifest(&root, "hitapp", "1.0", &url, &sha256_hex(&data));
        let manifest_str = manifest.to_string_lossy().into_owned();

        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        // Download it once...
        download_apps(&session, &[&manifest_str], &opts)
            .await
            .unwrap();
        let files = cache_files(&session);
        assert_eq!(files.len(), 1, "cache files: {files:?}");

        // ...then corrupt it in place with the same length, so the next run
        // still sees a complete (and therefore cached) file.
        std::fs::write(&files[0], vec![b'X'; data.len()]).unwrap();
        download_apps(&session, &[&manifest_str], &opts)
            .await
            .unwrap();
        server.shutdown();

        assert!(
            cache_files(&session).is_empty(),
            "a corrupt cache file must be removed on a cache hit"
        );
    }

    /// Nightly packages are never hashed (upstream skips them entirely), so a
    /// wrong manifest hash must not cost a nightly download its file.
    #[tokio::test]
    async fn test_download_apps_skips_hash_check_for_nightly() {
        let data = b"nightly-verify-skip".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_nightly_skip");
        let session = crate::test_utils::test_session(&root);
        let url = format!("http://{}/app.bin", server.addr);
        // Wrong hash on purpose — nightly is never verified anyway.
        let manifest = write_manifest(&root, "nightapp", "nightly", &url, &"0".repeat(64));
        let manifest_str = manifest.to_string_lossy().into_owned();

        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        download_apps(&session, &[&manifest_str], &opts)
            .await
            .unwrap();
        server.shutdown();

        let files = cache_files(&session);
        assert_eq!(files.len(), 1, "nightly file kept: {files:?}");
        assert_eq!(std::fs::read(&files[0]).unwrap(), data);
    }

    /// Fragmented (range-parallel) downloads must reassemble each file into
    /// its cache name as soon as that file's own chunks are complete, leaving
    /// no `.parts` directory behind.
    #[tokio::test]
    async fn test_download_apps_reassembles_fragmented_file() {
        let data: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_fragments");
        let session = crate::test_utils::test_session(&root);
        // Force the fragmented path for this small file.
        crate::config::set(&session, "aria2-min-split-size", "1k").unwrap();
        let url = format!("http://{}/app.bin", server.addr);
        let manifest = write_manifest(&root, "fragapp", "1.0", &url, &sha256_hex(&data));

        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        download_apps(&session, &[manifest.to_str().unwrap()], &opts)
            .await
            .unwrap();
        server.shutdown();

        let files = cache_files(&session);
        assert_eq!(files.len(), 1, "cache files: {:?}", files);
        let final_file = finalized_cache_file(&files, "fragapp#")
            .unwrap_or_else(|| panic!("file was not reassembled: {:?}", files));
        assert_eq!(std::fs::read(final_file).unwrap(), data);
        // The parts were consumed, not left behind.
        let leftovers: Vec<_> = std::fs::read_dir(session.config().cache_path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".parts"))
            .collect();
        assert!(leftovers.is_empty(), "leftover parts: {:?}", leftovers);
        assert!(
            server.ranges.lock().unwrap().len() > 1,
            "the file must have been fetched in parallel chunks: {:?}",
            server.ranges.lock().unwrap()
        );
    }

    #[tokio::test]
    async fn test_download_apps_hash_mismatch_removes_file() {
        let data = b"download-apps-mismatch".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_mismatch");
        let session = crate::test_utils::test_session(&root);
        let url = format!("http://{}/app.bin", server.addr);
        // Wrong hash: the downloaded file must be removed after verification.
        // The failure is reported per package, but the batch still succeeds —
        // Scoop continues on failure and exits 0 (`scoop-download.ps1:143`).
        let manifest = write_manifest(&root, "badapp", "1.0", &url, &"0".repeat(64));

        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        download_apps(&session, &[manifest.to_str().unwrap()], &opts)
            .await
            .expect("a failed package must not fail the batch");
        server.shutdown();

        assert!(
            cache_files(&session).is_empty(),
            "corrupt cache file must be removed"
        );
    }

    #[tokio::test]
    async fn test_download_apps_skip_hash_check_keeps_file() {
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
        download_apps(&session, &[manifest.to_str().unwrap()], &opts)
            .await
            .unwrap();
        server.shutdown();

        assert_eq!(cache_files(&session).len(), 1);
    }

    #[tokio::test]
    async fn test_download_apps_rejects_bad_version() {
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
        .await
        .unwrap();

        assert!(cache_files(&session).is_empty());
    }

    #[tokio::test]
    async fn test_download_apps_one_bad_query_does_not_abort_others() {
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
        .await
        .unwrap();
        server.shutdown();

        assert_eq!(cache_files(&session).len(), 1);
    }

    #[tokio::test]
    async fn test_download_apps_force_redownloads() {
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
        download_apps(&session, &[&manifest_str], &cache_hit)
            .await
            .unwrap();
        let requests_after_first = server.ranges.lock().unwrap().len();

        // Cache hit: no new request is sent.
        download_apps(&session, &[&manifest_str], &cache_hit)
            .await
            .unwrap();
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
        download_apps(&session, &[&manifest_str], &force)
            .await
            .unwrap();
        server.shutdown();

        assert!(
            server.ranges.lock().unwrap().len() > requests_after_first,
            "force must re-download"
        );
        assert_eq!(cache_files(&session).len(), 1);
    }

    /// Concurrent multi-package download (cross-package `jobs` concurrency):
    /// two packages fetched in parallel must both land in the cache with their
    /// own files, and neither download must be lost to the other.
    #[tokio::test]
    async fn test_download_apps_concurrent_packages() {
        let data = b"concurrent-package-download".to_vec();
        let mut server = spawn_range_server(data.clone(), 0, 0, false, false, None);
        let root = crate::test_utils::tmpdir("download_apps_concurrent");
        let session = crate::test_utils::test_session(&root);
        let url = format!("http://{}/app.bin", server.addr);
        let hash = sha256_hex(&data);

        let m1 = write_manifest(&root, "concapp1", "1.0", &url, &hash);
        let m2 = write_manifest(&root, "concapp2", "1.0", &url, &hash);

        let opts = DownloadOptions {
            force: false,
            check_hash: true,
        };
        download_apps(
            &session,
            &[m1.to_str().unwrap(), m2.to_str().unwrap()],
            &opts,
        )
        .await
        .unwrap();
        server.shutdown();

        let files = cache_files(&session);
        assert_eq!(
            files.len(),
            2,
            "both concurrent packages must be cached: {:?}",
            files
        );
        for f in &files {
            assert_eq!(&std::fs::read(f).unwrap(), &data);
        }
        assert!(
            files.iter().any(|f| f
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("concapp1#")),
            "concapp1 cached: {:?}",
            files
        );
        assert!(
            files.iter().any(|f| f
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("concapp2#")),
            "concapp2 cached: {:?}",
            files
        );
    }
}
