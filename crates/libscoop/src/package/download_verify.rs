//! Download verification shared by the install pipeline and the standalone
//! `download` command.
//!
//! Split from `package/download.rs`: hash verification of downloaded files
//! and the `download() → verify` wrapper, so the semantics cannot drift
//! between the two consumers.

use std::collections::HashSet;
use std::io::Read;

use tracing::info;

use crate::{error::Fallible, internal, Error, Event, Session};

use super::download::PackageSet;
use super::Package;

// ─── Download verification & standalone download API ───────────────────────

/// Verify downloaded files against the manifest hashes (upstream
/// `check_hash` in lib/download.ps1).
///
/// Nightly packages skip verification; a missing or empty hash in the
/// manifest warns and continues; a mismatch removes the corrupt cache
/// file so the next attempt re-downloads, and either fails hard or — in
/// IgnoreFailure mode — records the package ident in the returned set.
///
/// Shared by the install pipeline and the standalone `download` command so
/// the verification semantics cannot drift between them.
pub(crate) fn verify_downloads(
    session: &Session,
    packages: &[&Package],
    no_hash_check: bool,
    ignore_failure: bool,
) -> Fallible<HashSet<String>> {
    let mut failed = HashSet::new();
    if no_hash_check {
        return Ok(failed);
    }

    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackageIntegrityCheckStart);
    }

    let config = session.config();
    let cache_root = config.cache_path();

    let mut buf = [0; 1024 * 64];

    for &pkg in packages.iter() {
        if pkg.version() == "nightly" {
            info!("skip hash check for nightly package '{}'", pkg.name());
            continue;
        }

        // Resolve the upgradable reference exactly like `load_cache` does
        // (`pkg.upgradable().unwrap_or(pkg)`), so verification checks the
        // same cache filename the downloader produced for the *new* version.
        // In force/reinstall mode `set.packages` holds the installed package
        // (old version) whose `upgradable` points at the bucket manifest;
        // using the raw package would look for the old version's cache file
        // and fail with "file not found" even though the new one is cached.
        let pkg = pkg.upgradable().unwrap_or(pkg);

        let files = pkg.download_filenames();
        let hashes = pkg.download_hashes();
        let files_cnt = files.len();

        let result = (|| -> Fallible<()> {
            for (idx, (filename, hash)) in files.into_iter().zip(hashes).enumerate() {
                let path = cache_root.join(filename);

                // No hash in the manifest (missing or `""`): upstream
                // `check_hash` (lib/download.ps1) warns and continues
                // without verification instead of failing hard.
                if matches!(hash, crate::package::manifest::HashString::Empty) {
                    session.output().warn(format!(
                        "no hash in manifest for '{}', skipping verification",
                        pkg.name()
                    ));
                    continue;
                }

                let mut hasher = internal::hash::ChecksumBuilder::new()
                    .algo(hash.algorithm())?
                    .build();

                if let Some(tx) = session.emitter() {
                    let progress = format!("{} ({}/{})", pkg.name(), idx + 1, files_cnt);
                    let _ = tx.send(Event::PackageIntegrityCheckProgress(progress));
                }

                let mut file = std::fs::File::open(&path)?;
                loop {
                    let len = file.read(&mut buf)?;
                    if len == 0 {
                        break;
                    }
                    hasher.consume(&buf[..len]);
                }

                let actual = hasher.finalize();
                let expected = hash.value();
                if actual != expected {
                    // Upstream removes the corrupt cache file on a hash
                    // mismatch (lib/download.ps1:122-125) so the next
                    // attempt re-downloads instead of failing forever.
                    let _ = std::fs::remove_file(&path);
                    let name = pkg.name().to_owned();
                    let url = pkg.download_urls()[idx].to_owned();
                    let ctx =
                        super::HashMismatchContext::new(name, url, expected.to_owned(), actual);
                    return Err(Error::HashMismatch(ctx));
                }
            }
            Ok(())
        })();

        if let Err(e) = result {
            if ignore_failure {
                failed.insert(pkg.ident());
                session
                    .output()
                    .error(format!("failed to verify '{}': {}", pkg.name(), e));
            } else {
                return Err(e);
            }
        }
    }

    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackageIntegrityCheckDone);
    }

    Ok(failed)
}

/// Download the given packages and verify their hashes, returning the
/// idents that failed either step.
///
/// Wraps the shared sequence `download() → failed filter → verify_downloads`
/// used by both the install pipeline and the standalone download command,
/// emitting the download events around it. With `offline` no request is made
/// and only the cache is verified.
///
/// The caller must have run [`PackageSet::calculate_download_size`] on the
/// set first: the HEAD requests it makes fill in `remote_size`, which both
/// fragmented download (splitting) and the progress bar's total depend on.
pub(crate) fn download_and_verify(
    set: &mut PackageSet<'_>,
    no_hash_check: bool,
    ignore_failure: bool,
    offline: bool,
) -> Fallible<HashSet<String>> {
    let session = set.session;
    let mut failed = HashSet::new();

    if !offline {
        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageDownloadStart);
        }

        set.set_ignore_failure(ignore_failure);
        failed = set.download()?.into_iter().collect();

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageDownloadDone);
        }
    }

    let ok_pkgs: Vec<&Package> = set
        .packages
        .iter()
        .copied()
        .filter(|p| !failed.contains(&p.ident()))
        .collect();

    failed.extend(verify_downloads(
        session,
        &ok_pkgs,
        no_hash_check,
        ignore_failure,
    )?);
    Ok(failed)
}
