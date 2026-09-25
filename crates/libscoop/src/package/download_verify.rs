//! Download verification shared by the install pipeline and the standalone
//! `download` command.
//!
//! Split from `package/download.rs`: hash verification of downloaded files
//! and the `download() → verify` wrapper, so the semantics cannot drift
//! between the two consumers.

use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

use tracing::info;

use crate::{Error, Event, Session, error::Fallible, internal};

use super::Package;
use super::download::PackageSet;
use super::manifest::HashString;

// ─── Single-file verification ───────────────────────────────────────────────

/// Hash one cache file against its manifest hash — the single verification
/// entry point, shared by the inline path (each file as it lands) and the
/// batched tail pass (cache-only batches).
///
/// An empty hash warns and counts as skipped (upstream `check_hash` in
/// `lib/download.ps1` warns and continues instead of failing hard); a mismatch
/// removes the corrupt cache file (`lib/download.ps1:122-125`) and errors. The
/// per-file progress event is emitted from here so both paths announce the
/// same item at the same point — while the download bar owns the terminal the
/// event loop folds these into the line instead of printing them.
///
/// `progress` is the `"<name> (i/n)"` label both paths use.
pub(crate) fn verify_one_file(
    session: &Session,
    path: &Path,
    hash: &HashString,
    pkg_name: &str,
    url: &str,
    progress: &str,
) -> Fallible<()> {
    if matches!(hash, HashString::Empty) {
        session
            .output()
            .warn(hok_i18n::t!("cmd.no_hash_skip", name = pkg_name));
        return Ok(());
    }

    if let Some(tx) = session.emitter() {
        let _ = tx.try_send(Event::PackageIntegrityCheckProgress(progress.to_owned()));
    }

    let mut hasher = internal::hash::ChecksumBuilder::new()
        .algo(hash.algorithm())?
        .build();

    let mut file = std::fs::File::open(path)?;
    let mut buf = [0; 1024 * 64];
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
        // Upstream removes the corrupt cache file on a hash mismatch
        // (lib/download.ps1:122-125) so the next attempt re-downloads instead
        // of failing forever.
        let _ = std::fs::remove_file(path);
        let ctx = super::HashMismatchContext::new(
            pkg_name.to_owned(),
            url.to_owned(),
            expected.to_owned(),
            actual,
        );
        return Err(Error::HashMismatch(ctx));
    }
    Ok(())
}

// ─── Download verification & standalone download API ───────────────────────

/// User-facing text for a failed verification.
///
/// A hash mismatch carries structured context, so it is rendered through the
/// localized template (`error.hash_mismatch_detail`); any other failure — a
/// hash-algorithm or file-open error — keeps its own text. Both the inline and
/// the batched tail path call this, so the two report identically.
pub(crate) fn verify_error_message(e: &Error) -> String {
    match e {
        Error::HashMismatch(ctx) => ctx.localized(),
        other => other.to_string(),
    }
}

/// Verify downloaded files against the manifest hashes (upstream
/// `check_hash` in lib/download.ps1).
///
/// Only the packages that were *not* verified inline belong here (see
/// [`super::download::DownloadOutcome::verified`]); with an empty list
/// nothing is announced at all.
///
/// Nightly packages skip verification; a missing or empty hash in the
/// manifest warns and continues; a mismatch removes the corrupt cache
/// file so the next attempt re-downloads, and either fails hard or — in
/// IgnoreFailures mode — records the package ident in the returned set.
///
/// Shared by the install pipeline and the standalone `download` command so
/// the verification semantics cannot drift between them.
pub(crate) fn verify_downloads(
    session: &Session,
    packages: &[&Package],
    skip_hash_check: bool,
    ignore_failures: bool,
) -> Fallible<HashSet<String>> {
    let mut failed = HashSet::new();
    if skip_hash_check {
        return Ok(failed);
    }
    // Nothing to verify (the caller already hashed everything inline): an
    // empty block would only print a bare "checking hash" pair of lines.
    if packages.is_empty() {
        return Ok(failed);
    }

    if let Some(tx) = session.emitter() {
        let _ = tx.try_send(Event::PackageIntegrityCheckStart);
    }

    let config = session.config();
    let cache_root = config.cache_path();

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
        let urls = pkg.download_urls();
        let files_cnt = files.len();

        let result = (|| -> Fallible<()> {
            for (idx, (filename, hash)) in files.into_iter().zip(hashes).enumerate() {
                let path = cache_root.join(filename);
                let url = urls.get(idx).copied().unwrap_or_default();
                let label = format!("{} ({}/{})", pkg.name(), idx + 1, files_cnt);
                verify_one_file(session, &path, hash, pkg.name(), url, &label)?;
            }
            Ok(())
        })();

        if let Err(e) = result {
            if ignore_failures {
                failed.insert(pkg.ident());
                session.output().error(hok_i18n::t!(
                    "cmd.failed_verify",
                    name = pkg.name(),
                    error = verify_error_message(&e)
                ));
            } else {
                return Err(e);
            }
        }
    }

    if let Some(tx) = session.emitter() {
        let _ = tx.try_send(Event::PackageIntegrityCheckDone);
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
/// Files are hashed inline while downloading, so the tail pass only covers the
/// packages that never reached a download task (fully cached / offline); the
/// inline path's messages are printed here, once the download line is gone.
///
/// The caller must have run [`PackageSet::calculate_download_size`] on the
/// set first: the HEAD requests it makes fill in `remote_size`, which both
/// fragmented download (splitting) and the progress bar's total depend on.
pub(crate) async fn download_and_verify(
    set: &mut PackageSet<'_>,
    skip_hash_check: bool,
    ignore_failures: bool,
    offline: bool,
) -> Fallible<HashSet<String>> {
    let session = set.session;
    let mut failed = HashSet::new();
    // Packages whose files were hashed while downloading — hashing them again
    // in the tail pass would double every read.
    let mut verified: HashSet<String> = HashSet::new();

    if !offline {
        if let Some(tx) = session.emitter() {
            let _ = tx.try_send(Event::PackageDownloadStart);
        }

        set.set_ignore_failures(ignore_failures);
        set.set_skip_hash_check(skip_hash_check);
        let outcome = set.download().await?;

        if let Some(tx) = session.emitter() {
            let _ = tx.try_send(Event::PackageDownloadDone);
        }

        failed = outcome.failed.into_iter().collect();
        verified = outcome.verified.into_iter().collect();
        // The verification messages were deferred to here: the download line
        // owns the terminal until `PackageDownloadDone` above reaches the
        // event loop, and a raw line printed earlier would corrupt it.
        for msg in outcome.verify_errors {
            session.output().error(msg);
        }
    }

    let ok_pkgs: Vec<&Package> = set
        .packages
        .iter()
        .copied()
        .filter(|p| !failed.contains(&p.ident()))
        .filter(|p| !verified.contains(&p.ident()))
        .collect();

    failed.extend(verify_downloads(
        session,
        &ok_pkgs,
        skip_hash_check,
        ignore_failures,
    )?);
    Ok(failed)
}
