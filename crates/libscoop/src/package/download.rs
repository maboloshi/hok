//! Concurrent package download via the async `Network` trait (reqwest).
//!
//! Replaced the blocking `ureq` client (phase 4) so downloads are真 async I/O.
//! Fragmented downloads run concurrent chunk tasks with `buffer_unordered`.
//!
//! The module is split into focused files: the download core lives here,
//! fragmented (range-parallel) machinery in [`download_fragmented`],
//! verification in [`download_verify`], and the standalone `download`
//! command in [`download_apps`].

use std::cell::OnceCell;
use std::{
    collections::HashMap,
    fs::File,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tracing::debug;

use futures::StreamExt;

use crate::network::{HeaderProfile, ScoopRequest};
use crate::{Event, Session, error::Fallible, internal};

use super::{HashString, Package};

pub use super::download_apps::{DownloadOptions, download_apps};
use super::download_fragmented::download_fragmented;
pub(crate) use super::download_verify::download_and_verify;

/// Download size information.
#[derive(Clone, Copy)]
pub struct DownloadSize {
    /// Total size to download.
    pub total: u64,
    /// Whether the total size is estimated.
    pub estimated: bool,
}

/// A set of packages to download.
pub struct PackageSet<'a> {
    /// Backing session (exposed to `download_verify`/`download_apps`).
    pub(crate) session: &'a Session,
    pub packages: &'a [&'a Package],
    caches: OnceCell<HashMap<String, PackageCache<'a>>>,
    reuse_cache: bool,
    /// Skip packages whose download fails instead of aborting the operation.
    ignore_failures: bool,
    /// Skip the inline hash check (`-s`/`skip_hash_check`).
    skip_hash_check: bool,
}

struct FileDownloadInfo<'a> {
    url: &'a str,
    local_size: u64,
    remote_size: u64,
    estimated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CacheMaybeValid {
    Full,
    Partial,
    None,
}

struct PackageCache<'a> {
    package: &'a Package,
    valid: CacheMaybeValid,
    inner: HashMap<String, FileDownloadInfo<'a>>,
}

impl PackageCache<'_> {
    fn update_valid_state(&mut self) {
        let mut cnt = 0;
        for cache in self.inner.values() {
            // Only a file that actually exists locally (local_size > 0) and
            // matches the server size counts as a valid cache entry. A missing
            // local file must not compare equal to an unknown (0) remote size:
            // the equality 0 == 0 previously marked the package as fully
            // cached and skipped its download, then hash verification failed
            // with "file not found".
            if cache.local_size > 0 && cache.local_size == cache.remote_size {
                cnt += 1;
            }
        }
        self.valid = if cnt == self.inner.len() {
            CacheMaybeValid::Full
        } else if cnt > 0 {
            CacheMaybeValid::Partial
        } else {
            CacheMaybeValid::None
        };
    }
}

/// The final step that turns a file whose bytes have all landed into its
/// cache entry.
///
/// Applied by [`download_one_package`] as soon as that *file* is complete —
/// not after the whole batch — so a package that finishes early is already
/// usable while sibling packages are still downloading (upstream moves every
/// file to its cache name right after its own transfer, `lib/download.ps1:58-59`).
enum FileFinalize {
    /// Promote a fully downloaded temp file to its cache name.
    Rename { tmp: PathBuf, path: PathBuf },
    /// Reassemble a fragmented file's parts into its cache name.
    Reassemble { path: PathBuf, parts: Vec<PathBuf> },
}

impl FileFinalize {
    /// The upstream-equivalent failure message key for this step.
    fn error_key(&self) -> &'static str {
        match self {
            FileFinalize::Rename { .. } => "cmd.failed_move_download",
            FileFinalize::Reassemble { .. } => "cmd.failed_reassemble",
        }
    }

    /// Perform the step.
    fn apply(self) -> Fallible<()> {
        match self {
            FileFinalize::Rename { tmp, path } => Ok(std::fs::rename(tmp, path)?),
            FileFinalize::Reassemble { path, parts } => reassemble_fragments(&path, &parts),
        }
    }
}

/// Reassemble a fragmented download's part files into `final_path` and drop
/// the `.parts` directory.
fn reassemble_fragments(final_path: &Path, part_paths: &[PathBuf]) -> Fallible<()> {
    let _ = std::fs::remove_file(final_path);
    let mut dest = File::create(final_path)?;
    for part in part_paths {
        let mut src = File::open(part)?;
        std::io::copy(&mut src, &mut dest)?;
        drop(src);
        let _ = std::fs::remove_file(part);
    }
    if let Some(parent) = final_path.parent() {
        let part_dir = parent.join(format!(
            "{}.parts",
            final_path.file_name().unwrap().to_string_lossy()
        ));
        let _ = std::fs::remove_dir(&part_dir);
    }
    Ok(())
}

// ─── Inline verification ────────────────────────────────────────────────────

/// State of one package's inline verification, as seen by the frontend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifyState {
    /// Its files are being hashed right now (nothing failed so far).
    Checking,
    /// Every hashable file of the package matched its manifest hash.
    Passed,
    /// A file failed verification (the corrupt cache file was removed).
    Failed,
}

/// Progress context for the inline verification of one package.
#[derive(Clone, Debug)]
pub struct PackageVerifyProgressContext {
    /// `bucket/name` whose verification state changed.
    pub ident: String,
    /// The package's new state.
    pub state: VerifyState,
    /// Files hashed so far across the batch (passed + failed).
    pub done: u32,
    /// Total files this batch hashes inline (0 when verification is off).
    pub total: u32,
}

/// What a package task must do with the files it puts in the cache.
///
/// Shared by every concurrent package task: `total` is the batch-wide file
/// count the single download line shows as `校验 n/m`, and `done` is the
/// batch-wide counter those tasks advance together.
struct VerifyPlan {
    /// `false` when the caller skipped hash checks (`-s`/`skip_hash_check`).
    enabled: bool,
    /// Batch-wide number of files that will be hashed inline.
    total: u32,
    /// Files hashed so far across the batch.
    done: Arc<AtomicU32>,
}

impl VerifyPlan {
    /// Report a package's state to the frontend (no-op without an event bus).
    fn report(&self, session: &Session, package: &Package, state: VerifyState) {
        let Some(tx) = session.emitter() else {
            return;
        };
        let _ = tx.try_send(Event::PackageVerifyProgress(PackageVerifyProgressContext {
            ident: package.ident(),
            state,
            done: self.done.load(Ordering::Relaxed),
            total: self.total,
        }));
    }
}

/// Mutable state of one package's inline verification.
#[derive(Default)]
struct PkgVerifyState {
    /// The ident once any file failed (IgnoreFailures) — the download failure
    /// path writes the same field, so a package is reported at most once.
    failed: Option<String>,
    /// `cmd.failed_verify` messages, deferred until the download bar closes
    /// (printing them while it is live would corrupt its row).
    errors: Vec<String>,
    /// Files of this package hashed so far.
    done: usize,
    /// Files of this package that will be hashed (`0` for nightly packages).
    verifiable: usize,
}

/// One package's contribution to the batch outcome.
struct PkgOutcome {
    /// The ident when the package failed under IgnoreFailures.
    failed_ident: Option<String>,
    /// `cmd.failed_verify` messages collected inline, to be printed only once
    /// the download bar has closed.
    verify_errors: Vec<String>,
}

/// Result of [`PackageSet::download`].
#[derive(Debug, Default)]
pub struct DownloadOutcome {
    /// Idents (`bucket/name`) whose download or inline verification failed.
    /// Only populated when IgnoreFailures is enabled.
    pub failed: Vec<String>,
    /// Idents whose files were verified while downloading — the batched tail
    /// verification must not hash them a second time.
    pub verified: Vec<String>,
    /// Deferred `cmd.failed_verify` messages, printed after the bar closes.
    pub verify_errors: Vec<String>,
}

/// Number of files of `pkg` that will actually be hashed.
///
/// Skips the empty hashes upstream also skips (a warning is raised per file)
/// and nightly packages, which upstream never verifies.
fn verifiable_files(pkg: &Package) -> usize {
    if pkg.version() == "nightly" {
        return 0;
    }
    pkg.download_hashes()
        .iter()
        .filter(|h| !matches!(h, HashString::Empty))
        .count()
}

/// Hash one of a package's cache files and fold the result into the batch and
/// package counters, emitting [`Event::PackageVerifyProgress`].
///
/// Under IgnoreFailures a mismatch is recorded (message + failed ident) and the
/// caller keeps going with the package's remaining files, mirroring upstream's
/// `continue` after a failed hash check; otherwise the mismatch propagates as a
/// hard error. Nightly packages, empty hashes and a disabled check are handled
/// by [`super::download_verify::verify_one_file`] / the guards here.
#[allow(clippy::too_many_arguments)]
fn verify_inline_file(
    session: &Session,
    package: &Package,
    filename: &str,
    path: &Path,
    plan: &VerifyPlan,
    state: &mut PkgVerifyState,
    ignore_failures: bool,
) -> Fallible<()> {
    if !plan.enabled {
        return Ok(());
    }
    // Upstream never hashes a nightly package — neither does the tail pass —
    // so no warning either.
    if package.version() == "nightly" {
        return Ok(());
    }
    let files = package.download_filenames();
    let hashes = package.download_hashes();
    // Align by manifest order: `cache.inner` is a HashMap, so the file's index
    // (for the `(i/n)` label) and hash must come from the manifest lists.
    let Some(idx) = files.iter().position(|f| f == filename) else {
        return Ok(());
    };
    let Some(hash) = hashes.get(idx).copied() else {
        return Ok(());
    };
    let urls = package.download_urls();
    let url = urls.get(idx).copied().unwrap_or_default();
    let label = format!("{} ({}/{})", package.name(), idx + 1, files.len());

    match super::download_verify::verify_one_file(session, path, hash, package.name(), url, &label)
    {
        Ok(()) => {}
        Err(e) if ignore_failures => {
            state.errors.push(hok_i18n::t!(
                "cmd.failed_verify",
                name = package.name(),
                error = super::download_verify::verify_error_message(&e)
            ));
            state.failed = Some(package.ident());
        }
        Err(e) => return Err(e),
    }

    state.done += 1;
    plan.done.fetch_add(1, Ordering::Relaxed);
    let pkg_state = if state.failed.is_some() {
        VerifyState::Failed
    } else if state.done >= state.verifiable {
        VerifyState::Passed
    } else {
        VerifyState::Checking
    };
    plan.report(session, package, pkg_state);
    Ok(())
}

/// Download the files of a single package. Runs on a concurrent task so
/// multiple packages download in parallel (cross-package concurrency).
///
/// Per file: fragmented or single-stream download, then finalization into the
/// cache name (rename / reassemble) as soon as that file's bytes have landed,
/// then an inline hash check — upstream verifies every file right after its own
/// transfer (`lib/download.ps1:19-45`, `libexec/scoop-download.ps1:105-135`).
/// Progress, reuse-cache and the cancellation token are honoured throughout;
/// each package writes only its own cache files, so concurrent tasks never
/// collide.
///
/// Returns the package's failure ident (IgnoreFailures) together with the
/// verification messages that must wait for the bar to close.
///
#[allow(clippy::too_many_arguments)]
async fn download_one_package<'a>(
    session: &'a Session,
    cache: &PackageCache<'a>,
    cache_root: &Path,
    net: &Arc<dyn internal::network::Network + Send + Sync>,
    reuse_cache: bool,
    ignore_failures: bool,
    fragmentation_enabled: bool,
    chunk_count: u64,
    min_split_size: u64,
    plan: &VerifyPlan,
) -> Fallible<PkgOutcome> {
    let cookie = cache.package.cookie().unwrap_or_default();
    let ident = cache.package.ident();
    let cookie_map: Option<HashMap<String, String>> = if cookie.is_empty() {
        None
    } else {
        Some(
            cookie
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    };

    // A failure on any file marks the whole package as failed so the rest of
    // the transaction can still be committed (IgnoreFailures).
    let mut pkg_error: Option<crate::Error> = None;
    // Inline verification bookkeeping: failed ident, deferred error messages
    // and the counters that feed `校验 n/m` on the single download line.
    let mut verify_state = PkgVerifyState {
        verifiable: verifiable_files(cache.package),
        ..Default::default()
    };
    let emitter = session.emitter();

    // Total bytes the package must fetch (all files that are not already
    // fully cached). Mirrors the skip condition in the loop below, so the
    // reported pkg_total equals the bytes actually downloaded.
    let pkg_total: u64 = cache
        .inner
        .iter()
        .filter(|(_, d)| !(reuse_cache && d.local_size > 0 && d.local_size == d.remote_size))
        .map(|(_, d)| d.remote_size)
        .sum();

    // Running total of bytes this package has fully downloaded (across its
    // files). Shared across the per-file progress closures so each progress
    // event reports the package-wide cumulative done.
    let pkg_done = Arc::new(AtomicU64::new(0));

    for (filename, dlinfo) in cache.inner.iter() {
        if reuse_cache && dlinfo.local_size > 0 && dlinfo.local_size == dlinfo.remote_size {
            // A partially cached package still hashes the file that was already
            // in the cache: every file of a dispatched package is verified
            // inline, so the batched tail pass can skip the package entirely.
            verify_inline_file(
                session,
                cache.package,
                filename,
                &cache_root.join(filename),
                plan,
                &mut verify_state,
                ignore_failures,
            )?;
            continue;
        }

        let use_fragments = fragmentation_enabled
            && !dlinfo.estimated
            && dlinfo.remote_size >= min_split_size
            && dlinfo.remote_size > 0
            && chunk_count > 1
            // Avoid `(chunk_idx + 1) * chunk_size - 1` underflow when the
            // file is smaller than the configured chunk count.
            && chunk_count <= dlinfo.remote_size;

        // hok's download header contract for this file: User-Agent,
        // PRIVATE_HOSTS extra headers and GitHub API auth — and deliberately no
        // Referer (`HeaderProfile::referer`). Resolved per URL because the
        // PRIVATE_HOSTS match depends on it.
        let request = ScoopRequest::for_url(session, dlinfo.url, HeaderProfile::Download);
        let file_opts = internal::network::RequestOptions {
            cookies: cookie_map.as_ref(),
            ..request.options(120)
        };

        let result: Fallible<FileFinalize> = async {
            if use_fragments {
                let path = cache_root.join(filename);
                let part_dir = cache_root.join(format!("{}.parts", filename));
                internal::fs::ensure_dir(&part_dir)?;

                // Chunks whose retries are exhausted are re-downloaded serially
                // (see `download_fragmented`), so a weak link degrades to fewer
                // connections instead of failing the whole file.
                let url_str = dlinfo.url.to_owned();
                // Aggregate per-chunk deltas into a file-level count so the
                // progress event matches the single-stream path.
                let progress_total = Arc::new(AtomicU64::new(0));
                let progress_total_h = progress_total.clone();
                let throttle = Arc::new(ProgressThrottle::new());
                let throttle_h = throttle.clone();
                let emitter_h = emitter.clone();
                let ident_h = ident.clone();
                let filename_h = filename.to_owned();
                let url_h = url_str.clone();
                let pkg_done_h = pkg_done.clone();
                let pkg_total_h = pkg_total;
                let on_progress = move |delta: u64| {
                    let dlnow = progress_total_h.fetch_add(delta, Ordering::Relaxed) + delta;
                    // Throttle to one event per 100 ms so a slow event loop
                    // cannot back up the bounded channel (aligned with rake's
                    // THROTTLE_DURATION); force the final event so the bar
                    // reaches 100%.
                    let force = dlnow >= dlinfo.remote_size;
                    if throttle_h.should_report(force)
                        && let Some(tx) = &emitter_h
                    {
                        let _ = tx.try_send(Event::PackageDownloadProgress(
                            PackageDownloadProgressContext {
                                ident: ident_h.clone(),
                                url: url_h.clone(),
                                filename: filename_h.clone(),
                                dltotal: dlinfo.remote_size,
                                dlnow,
                                pkg_total: pkg_total_h,
                                pkg_done: pkg_done_h.load(Ordering::Relaxed) + dlnow,
                            },
                        ));
                    }
                };
                let part_paths = download_fragmented(
                    net.as_ref(),
                    &url_str,
                    &file_opts,
                    dlinfo.remote_size,
                    chunk_count,
                    &part_dir,
                    &on_progress,
                    &session.cancellation_token(),
                )
                .await
                .map_err(crate::error::Error::Custom)?;

                pkg_done.fetch_add(dlinfo.remote_size, Ordering::Relaxed);
                Ok(FileFinalize::Reassemble {
                    path,
                    parts: part_paths,
                })
            } else {
                // Single download (with resume for known-size files).
                let path = cache_root.join(filename);
                let tmp = cache_root.join(format!("{}.download", filename));
                let _ = std::fs::remove_file(&path);

                // Resume support: when the total size is known (not estimated)
                // and a partial `.download` already exists, resume from its
                // current size via a Range request instead of re-downloading.
                let known_total = dlinfo.remote_size > 0 && !dlinfo.estimated;
                let existing_len = tmp.metadata().map(|m| m.len()).unwrap_or(0);
                let resume_from = if known_total {
                    existing_len.min(dlinfo.remote_size)
                } else {
                    0
                };

                // A `.download` already at the full size needs no network
                // transfer — promote it straight to the cache.
                if known_total && existing_len == dlinfo.remote_size {
                    Ok(FileFinalize::Rename { tmp, path })
                } else {
                    // Fresh start: drop any stale partial.
                    if resume_from == 0 {
                        let _ = std::fs::remove_file(&tmp);
                    }

                    let fname = filename.to_owned();
                    let url_str = dlinfo.url.to_owned();
                    let dlinfo_total = dlinfo.remote_size;
                    let pkg_total_here = pkg_total;
                    let pkg_done_h = pkg_done.clone();

                    // Stream the body (from `resume_from` when resuming)
                    // through the network capability (reports cumulative bytes
                    // via on_progress).
                    let throttle = Arc::new(ProgressThrottle::new());
                    let throttle_h = throttle.clone();
                    let emitter_h = emitter.clone();
                    let ident_h = ident.clone();
                    let url_h = url_str.clone();
                    let fname_h = fname.clone();
                    let mut on_progress =
                        move |dlnow: u64| -> Pin<Box<dyn Future<Output = ()> + Send>> {
                            // Clone per call: the future is `move` so it must own
                            // its captures.
                            let throttle = throttle_h.clone();
                            let tx = emitter_h.clone();
                            let ident = ident_h.clone();
                            let url = url_h.clone();
                            let filename = fname_h.clone();
                            let total = dlinfo_total;
                            let pkg_done_here = pkg_done_h.clone();
                            Box::pin(async move {
                                // Throttle to one event per 100 ms (aligned with
                                // rake's THROTTLE_DURATION); force the final event
                                // so the bar reaches 100%.
                                let force = dlnow >= total;
                                if throttle.should_report(force)
                                    && let Some(tx) = &tx
                                {
                                    let ctx = PackageDownloadProgressContext {
                                        ident,
                                        url,
                                        filename,
                                        dltotal: total,
                                        dlnow,
                                        pkg_total: pkg_total_here,
                                        pkg_done: pkg_done_here.load(Ordering::Relaxed) + dlnow,
                                    };
                                    // `send().await` gives backpressure: a slow
                                    // event loop cannot silently drop progress.
                                    let _ = tx.send(Event::PackageDownloadProgress(ctx)).await;
                                }
                            })
                        };

                    let resumable_opts = internal::network::RequestOptions {
                        range_start: resume_from,
                        ..file_opts.clone()
                    };
                    // Race the stream against cooperative Ctrl-C cancellation;
                    // on cancel the partial `.download` file is kept so the
                    // next run resumes from here.
                    let cancel = session.cancellation_token();
                    tokio::select! {
                        _ = cancel.cancelled() => {
                            return Err(crate::error::Error::Custom(
                                "download cancelled".to_owned(),
                            ));
                        }
                        res = net.download_stream(
                            &url_str,
                            &resumable_opts,
                            &tmp,
                            &mut on_progress,
                        ) => {
                            // A timeout carries its deadline so the frontend can
                            // render "timed out after Ns": reqwest's own text
                            // ("error sending request for url (...)") hides it
                            // in the `source()` chain.
                            res.map_err(|e| match e {
                                internal::network::DownloadError::Timeout { secs } => {
                                    crate::error::Error::DownloadTimedOut {
                                        url: url_str.clone(),
                                        timeout_secs: secs,
                                    }
                                }
                                internal::network::DownloadError::Other(reason) => {
                                    crate::error::Error::Custom(reason)
                                }
                            })?;
                            // A resumed download must reach the full size — a
                            // short 206 would leave a corrupt part that must
                            // not be promoted to cache.
                            if resume_from > 0
                                && tmp.metadata().map(|m| m.len()).unwrap_or(0)
                                    != dlinfo.remote_size
                            {
                                return Err(crate::error::Error::Custom(format!(
                                    "short resumed download: {} != {} bytes",
                                    tmp.display(),
                                    dlinfo.remote_size,
                                )));
                            }
                        }
                    }

                    pkg_done.fetch_add(dlinfo.remote_size, Ordering::Relaxed);
                    Ok(FileFinalize::Rename { tmp, path })
                }
            }
        }
        .await;

        let finalize = match result {
            Ok(finalize) => finalize,
            Err(e) => {
                pkg_error = Some(e);
                break;
            }
        };

        // Finalize this file now that its own bytes have all landed, then hash
        // it — an early package is both complete and verified while its
        // siblings still download (upstream verifies per file too,
        // `libexec/scoop-download.ps1:105-135`).
        let error_key = finalize.error_key();
        if let Err(e) = finalize.apply() {
            if ignore_failures {
                session
                    .output()
                    .error(hok_i18n::t!(error_key, name = ident.clone(), error = e));
                verify_state.failed = Some(ident.clone());
                // Tell the line too, or the package would sit at its last
                // percentage until the row closes.
                plan.report(session, cache.package, VerifyState::Failed);
            } else {
                return Err(e);
            }
        } else {
            verify_inline_file(
                session,
                cache.package,
                filename,
                &cache_root.join(filename),
                plan,
                &mut verify_state,
                ignore_failures,
            )?;
        }
    }

    if let Some(err) = pkg_error {
        if ignore_failures {
            session.output().error(hok_i18n::t!(
                "cmd.failed_download",
                name = ident.clone(),
                error = err
            ));
            verify_state.failed = Some(ident.clone());
            // Same as a finalization failure: a dead download never reaches its
            // byte total, so the line has to be told it failed.
            plan.report(session, cache.package, VerifyState::Failed);
        } else {
            return Err(err);
        }
    }

    // A package with nothing to hash (nightly, or every hash empty) never goes
    // through `verify_inline_file`, so report its terminal state here —
    // otherwise the line would keep it as "verifying" until the row closes.
    if plan.enabled && verify_state.verifiable == 0 && verify_state.failed.is_none() {
        plan.report(session, cache.package, VerifyState::Passed);
    }

    Ok(PkgOutcome {
        failed_ident: verify_state.failed,
        verify_errors: verify_state.errors,
    })
}

impl<'a> PackageSet<'a> {
    pub fn new(
        session: &'a Session,
        packages: &'a [&Package],
        reuse_cache: bool,
    ) -> Fallible<PackageSet<'a>> {
        Ok(PackageSet {
            session,
            packages,
            caches: OnceCell::new(),
            reuse_cache,
            ignore_failures: false,
            skip_hash_check: false,
        })
    }

    /// Ignore per-package download failures and keep going with the rest of
    /// the transaction.
    ///
    /// When enabled, [`download`][1] skips packages whose download fails (and
    /// reports them in the returned list of failed idents) instead of aborting
    /// the whole operation, so the remaining packages can still be committed.
    ///
    /// [1]: PackageSet::download
    pub fn set_ignore_failures(&mut self, ignore: bool) {
        self.ignore_failures = ignore;
    }

    /// Skip the inline hash check (`-s/--skip-hash-check`).
    ///
    /// Without it every file is verified as it lands; with it neither the
    /// download phase nor the batched tail pass hashes anything.
    pub fn set_skip_hash_check(&mut self, skip: bool) {
        self.skip_hash_check = skip;
    }

    fn load_cache(&self) {
        if self.caches.get().is_some() {
            return;
        }

        let config = self.session.config();
        let cache_root = config.cache_path();
        let mut caches = HashMap::new();

        for &pkg in self.packages.iter() {
            let pkg = pkg.upgradable().unwrap_or(pkg);
            let urls = pkg.download_urls();
            let filenames = pkg.download_filenames();

            let mut package_cache = PackageCache {
                package: pkg,
                valid: CacheMaybeValid::None,
                inner: HashMap::new(),
            };

            let mut file_cached_count = 0;
            for (url, filename) in urls.iter().zip(filenames.iter()) {
                let remote_size = 0u64;
                let mut local_size = 0u64;

                if self.reuse_cache
                    && let Ok(file) = File::open(cache_root.join(filename))
                    && let Ok(metadata) = file.metadata()
                {
                    local_size = metadata.len();
                    file_cached_count += 1;
                }

                package_cache.inner.insert(
                    filename.to_owned(),
                    FileDownloadInfo {
                        url,
                        local_size,
                        remote_size,
                        estimated: false,
                    },
                );
            }

            if self.reuse_cache {
                if file_cached_count == urls.len() {
                    package_cache.valid = CacheMaybeValid::Full;
                } else if file_cached_count > 0 {
                    package_cache.valid = CacheMaybeValid::Partial;
                }
            }

            caches.insert(pkg.ident(), package_cache);
        }

        let _ = self.caches.set(caches);
    }

    // ─── Download ─────────────────────────────────────────────────────────────

    /// Download packages, verifying each file as soon as it lands.
    ///
    /// # Returns
    ///
    /// A [`DownloadOutcome`]: the idents of the packages that failed (only
    /// populated with [`set_ignore_failures`][1]; without it the first failure
    /// aborts), the idents already verified inline, and the verification
    /// messages that must wait for the frontend's download bar to close.
    ///
    /// [1]: PackageSet::set_ignore_failures
    pub async fn download(&mut self) -> Fallible<DownloadOutcome> {
        if self.caches.get().is_none() {
            self.load_cache();
        }

        // Extract config data into owned values inside a scope so the config
        // read guard is dropped before the awaits below (a guard held across an
        // await would block a concurrent `config_mut()`).
        let (cache_root, fragmentation_enabled, chunk_count, min_split_size, jobs) = {
            let config = self.session.config();
            (
                config.cache_path().to_path_buf(),
                config.aria2_enabled(),
                config
                    .aria2_split()
                    .min(config.aria2_max_connection_per_server()) as u64,
                config.aria2_min_split_size(),
                config.jobs() as usize,
            )
        };

        // Extract session-owned copies up front so the concurrent futures
        // below never capture `self` (which would conflict with the caches
        // borrow). Session is Sync + Send, so all of these share cleanly.
        let session = self.session;
        let reuse_cache = self.reuse_cache;
        let ignore_failures = self.ignore_failures;
        let skip_hash_check = self.skip_hash_check;
        let net = self.session.network();
        let emitter = self.session.emitter();

        // Read caches immutably: the download phase never mutates them.
        let package_caches = self.caches.get().unwrap();

        internal::fs::ensure_dir(&cache_root)?;

        // Concurrent cross-package download: each package's files are fetched
        // on its own task, bounded by `jobs`, and each file is finalized to its
        // cache name as soon as its own bytes have landed — then hashed. Every
        // package writes only its own cache files (names embed `name#version#`),
        // so concurrent tasks never collide. Fully-cached packages are skipped
        // (their cache-hit events are still emitted) and left to the batched
        // tail verification; packages whose download fails are reported
        // per-package, matching the sequential semantics.
        let jobs = jobs.max(1);

        // Tell the frontend which packages are about to be downloaded and their
        // total bytes, so it can render one aggregate progress bar (count and
        // grand total) before the per-package progress events start arriving.
        // The same event carries how many of their files will be hashed inline:
        // that denominator drives the line's `校验 n/m` counter.
        let dispatched: Vec<&PackageCache<'_>> = package_caches
            .values()
            .filter(|c| !(reuse_cache && c.valid == CacheMaybeValid::Full))
            .collect();
        let verify_total: u32 = if skip_hash_check {
            0
        } else {
            dispatched
                .iter()
                .map(|c| verifiable_files(c.package) as u32)
                .sum()
        };
        let verified: Vec<String> = dispatched.iter().map(|c| c.package.ident()).collect();

        if let Some(tx) = &emitter {
            let packages: Vec<(String, u64)> = dispatched
                .iter()
                .map(|c| {
                    let total: u64 = c
                        .inner
                        .iter()
                        .filter(|(_, d)| {
                            !(reuse_cache && d.local_size > 0 && d.local_size == d.remote_size)
                        })
                        .map(|(_, d)| d.remote_size)
                        .sum();
                    (c.package.ident(), total)
                })
                .collect();
            let _ = tx.try_send(Event::PackageDownloadInfo {
                packages,
                verify_files: verify_total,
            });
            // The integrity block belongs to this phase: on a terminal the
            // event loop folds it into the download line, off a terminal it
            // still prints the status lines it always printed.
            if verify_total > 0 {
                let _ = tx.try_send(Event::PackageIntegrityCheckStart);
            }
        }

        let plan = VerifyPlan {
            enabled: !skip_hash_check,
            total: verify_total,
            done: Arc::new(AtomicU32::new(0)),
        };

        // Capacity invariant: `buffer_unordered(0)` never polls its source
        // stream, so the stream is never drained and the `collect` below parks
        // forever (a silent hang, not an error). `jobs` was normalized to >= 1
        // above; this pins that normalization here, where the cap is used.
        debug_assert!(jobs >= 1, "buffer_unordered capacity must be >= 1");
        let outcomes: Vec<Fallible<PkgOutcome>> =
            futures::stream::iter(package_caches.values().filter(|c| {
                if reuse_cache && c.valid == CacheMaybeValid::Full {
                    if let Some(tx) = &emitter {
                        // One event per package (not per cached file): the
                        // frontend renders the readable identity, so a
                        // multi-file package would otherwise repeat the same
                        // line once per file.
                        let package = c.package;
                        let _ = tx.try_send(Event::PackageCacheHit {
                            ident: package.ident(),
                            name: package.name().to_owned(),
                            version: package.version().to_owned(),
                        });
                    }
                    false
                } else {
                    true
                }
            }))
            .map(|cache| {
                download_one_package(
                    session,
                    cache,
                    &cache_root,
                    &net,
                    reuse_cache,
                    ignore_failures,
                    fragmentation_enabled,
                    chunk_count,
                    min_split_size,
                    &plan,
                )
            })
            .buffer_unordered(jobs)
            .collect()
            .await;

        // Every task has finished (even the one that failed), so close the
        // inline integrity block before the merge can abort on a hard error.
        if verify_total > 0
            && let Some(tx) = &emitter
        {
            let _ = tx.try_send(Event::PackageIntegrityCheckDone);
        }

        // Merge the per-package outcomes: everything is already finalized to
        // its cache name inside its own task, so only the failure idents and
        // the deferred verification messages are left to collect. A hard
        // (non-ignore) failure aborts here — the packages that finished
        // earlier keep their finalized cache files.
        let mut failed: Vec<String> = vec![];
        let mut verify_errors: Vec<String> = vec![];
        for outcome in outcomes {
            let outcome = outcome?;
            if let Some(ident) = outcome.failed_ident {
                failed.push(ident);
            }
            verify_errors.extend(outcome.verify_errors);
        }

        Ok(DownloadOutcome {
            failed,
            verified,
            verify_errors,
        })
    }

    // ─── Calculate download size ──────────────────────────────────────────────

    pub async fn calculate_download_size(&mut self) -> Fallible<DownloadSize> {
        if self.caches.get().is_none() {
            self.load_cache();
        }

        let jobs = self.session.config().jobs() as usize;

        // Fire all HEAD requests concurrently (one per URL), then apply the
        // results back to the cache entries serially. The HEAD is pure network
        // I/O and only writes `remote_size`/`estimated` into the caches, so the
        // concurrent phase captures owned data and the apply phase mutates the
        // caches under a single `&mut`.
        let net = self.session.network();
        let session = self.session;

        // Collect every (url, filename, ident, cookie) as an owned HEAD task.
        type HeadTask = (String, String, String, Option<HashMap<String, String>>);
        let tasks: Vec<HeadTask> = self
            .packages
            .iter()
            .map(|&pkg| pkg.upgradable().unwrap_or(pkg))
            .flat_map(|pkg| {
                let ident = pkg.ident().to_owned();
                let cookie = pkg.cookie().unwrap_or_default();
                let cookie_map: Option<HashMap<String, String>> = if cookie.is_empty() {
                    None
                } else {
                    Some(
                        cookie
                            .iter()
                            .map(|(k, v)| (k.to_string(), v.to_string()))
                            .collect(),
                    )
                };
                pkg.download_urls()
                    .iter()
                    .zip(pkg.download_filenames().iter())
                    .map(move |(url, filename)| {
                        (
                            url.to_string(),
                            filename.clone(),
                            ident.clone(),
                            cookie_map.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        let results: Vec<(String, String, u16, Option<u64>)> = futures::stream::iter(tasks)
            .map(|(url, filename, ident, cookie_map)| {
                let net = net.clone();
                async move {
                    // Same header contract as the download itself, so the HEAD
                    // probe sees the same headers as the GET.
                    let request = ScoopRequest::for_url(session, &url, HeaderProfile::Download);
                    let opts = internal::network::RequestOptions {
                        cookies: cookie_map.as_ref(),
                        ..request.options(30)
                    };
                    let result = net.head(&url, &opts).await;
                    (ident, filename, result.status_code, result.content_length)
                }
            })
            .buffer_unordered(jobs.max(1))
            .collect()
            .await;

        // Apply HEAD results back into the caches (serial, per file).
        let package_caches = self.caches.get_mut().unwrap();
        let mut total = 0u64;
        let mut estimated = false;

        for (ident, filename, code, content_length) in results {
            let package_cache = package_caches.get_mut(&ident).unwrap();
            let info = package_cache
                .inner
                .get_mut(&filename)
                .expect("failed to get cache info");

            if code == 200 {
                info.remote_size = content_length.unwrap_or(0);
                if info.remote_size != info.local_size {
                    total += info.remote_size;
                }
            } else {
                debug!("code: {}, ident: {}, url: {}", code, ident, filename)
            }

            if info.remote_size == 0 {
                info.estimated = true;
                estimated = true;
            }

            package_cache.update_valid_state();
        }

        Ok(DownloadSize { total, estimated })
    }
}

// ─── Progress throttle ─────────────────────────────────────────────────────

/// Minimum interval between two download progress events.
///
/// Aligned with rake's `THROTTLE_DURATION` (100 ms): download progress can
/// otherwise fire far faster than the event loop consumes, and the bounded
/// `tokio::sync::mpsc` bus drops `try_send` events when it fills, making the
/// progress bar jump. Throttling to one event per interval bounds the event
/// rate so the bar updates smoothly and `send().await` backpressure is cheap.
const PROGRESS_THROTTLE: Duration = Duration::from_millis(100);

/// Throttle download-progress events to at most one per [`PROGRESS_THROTTLE`].
///
/// `Sync` (via `Mutex`) so a single shared throttle can be handed to the
/// concurrent fragmented-download chunk tasks.
#[derive(Debug, Default)]
pub(crate) struct ProgressThrottle {
    last: Mutex<Option<Instant>>,
}

impl ProgressThrottle {
    /// Create a fresh throttle (the first report always passes).
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Whether a progress event should be emitted now: true when at least
    /// [`PROGRESS_THROTTLE`] has elapsed since the previous report (or on the
    /// first call). `force` bypasses the throttle so the final event (bytes
    /// reaching the total) is always delivered — otherwise the bar could sit
    /// below 100% until the download-done event. Updates the timestamp when
    /// it reports (forced or not).
    pub(crate) fn should_report(&self, force: bool) -> bool {
        let now = Instant::now();
        let mut last = self.last.lock().unwrap();
        match *last {
            Some(prev) if !force && now.duration_since(prev) < PROGRESS_THROTTLE => false,
            _ => {
                *last = Some(now);
                true
            }
        }
    }
}

// ─── Progress context ───────────────────────────────────────────────────────

/// Progress context for package download.
#[derive(Clone, Debug)]
pub struct PackageDownloadProgressContext {
    pub ident: String,
    pub url: String,
    pub filename: String,
    /// Total bytes of the *current file*.
    pub dltotal: u64,
    /// Bytes downloaded so far of the *current file*.
    pub dlnow: u64,
    /// Total bytes of the whole package (all files to download). Lets the UI
    /// aggregate per-file progress into a per-package bar / single total bar.
    pub pkg_total: u64,
    /// Bytes of the package downloaded so far (sum of completed files plus the
    /// current file's progress). Monotonic within a package.
    pub pkg_done: u64,
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_chunk_boundaries() {
        let size = 100u64;
        let chunks = 4u64;
        let chunk_size = size / chunks;
        assert_eq!(chunk_size, 25);
        assert_eq!(chunk_size - 1, 24);
        assert_eq!(chunk_size, 25);
        assert_eq!((1 + 1) * chunk_size - 1, 49);
        assert_eq!(3 * chunk_size, 75);
        assert_eq!(size - 1, 99);
    }

    #[test]
    fn test_chunk_cover_all_bytes() {
        check_chunks_cover_all(100, 5);
    }

    #[test]
    fn test_chunk_remainder() {
        check_chunks_cover_all(10, 3);
    }

    /// Verify that `chunks` evenly cover all `size` bytes without gaps.
    fn check_chunks_cover_all(size: u64, chunks: u64) {
        let chunk_size = size / chunks;
        let mut covered = vec![false; size as usize];
        for i in 0..chunks {
            let start = i * chunk_size;
            let end = if i == chunks - 1 {
                size - 1
            } else {
                (i + 1) * chunk_size - 1
            };
            for b in start..=end {
                covered[b as usize] = true;
            }
        }
        assert!(covered.iter().all(|&c| c));
    }

    #[test]
    fn test_chunk_single_byte() {
        let size = 1u64;
        let chunks = 1u64;
        let chunk_size = size / chunks;
        assert_eq!(chunk_size, 1);
        assert_eq!(size - 1, 0);
    }

    #[test]
    fn test_progress_throttle_first_always_reports() {
        let t = super::ProgressThrottle::new();
        assert!(t.should_report(false), "first report must pass");
    }

    #[test]
    fn test_progress_throttle_blocks_rapid_reports() {
        let t = super::ProgressThrottle::new();
        assert!(t.should_report(false));
        // Immediate second report within the 100 ms window is throttled.
        assert!(!t.should_report(false));
    }

    #[test]
    fn test_progress_throttle_force_bypasses_window() {
        let t = super::ProgressThrottle::new();
        assert!(t.should_report(false));
        // `force` bypasses the throttle so the final event always lands.
        assert!(t.should_report(true));
    }

    #[test]
    fn test_progress_throttle_reports_after_interval() {
        let t = super::ProgressThrottle::new();
        assert!(t.should_report(false));
        assert!(!t.should_report(false));
        // Past the throttle interval the throttle re-opens.
        std::thread::sleep(std::time::Duration::from_millis(110));
        assert!(t.should_report(false));
    }

    #[test]
    fn test_update_valid_state_missing_file_with_unknown_size_is_not_cached() {
        // Regression: a package whose cache file is missing (local_size 0) and
        // whose remote size is unknown (0, e.g. a failed HEAD request) must NOT
        // be treated as fully cached. Previously local_size == remote_size
        // evaluated 0 == 0, marking the package Full so download skipped it
        // (emitting a cache-hit event), and hash verification then failed with
        // "file not found".
        use super::{CacheMaybeValid, FileDownloadInfo, PackageCache};
        use std::collections::HashMap;

        let pkg = crate::package::Package::from(
            "app",
            "test",
            crate::package::manifest::Manifest::from_json(
                "app",
                r#"{"version": "2.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
            )
            .unwrap(),
        );
        let mut pc = PackageCache {
            package: &pkg,
            valid: CacheMaybeValid::None,
            inner: HashMap::new(),
        };
        pc.inner.insert(
            "app#2.0.0#abc123.7z".to_owned(),
            FileDownloadInfo {
                url: "https://example.com/app.7z",
                local_size: 0,
                remote_size: 0,
                estimated: true,
            },
        );
        pc.update_valid_state();
        assert_eq!(pc.valid, CacheMaybeValid::None);
    }

    #[test]
    fn test_update_valid_state_full_only_for_present_matching_file() {
        use super::{CacheMaybeValid, FileDownloadInfo, PackageCache};
        use std::collections::HashMap;

        let pkg = crate::package::Package::from(
            "app",
            "test",
            crate::package::manifest::Manifest::from_json(
                "app",
                r#"{"version": "2.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
            )
            .unwrap(),
        );
        let mut pc = PackageCache {
            package: &pkg,
            valid: CacheMaybeValid::None,
            inner: HashMap::new(),
        };
        // A present local file matching the remote size is a valid cache entry.
        pc.inner.insert(
            "app#2.0.0#abc123.7z".to_owned(),
            FileDownloadInfo {
                url: "https://example.com/app.7z",
                local_size: 100,
                remote_size: 100,
                estimated: false,
            },
        );
        pc.update_valid_state();
        assert_eq!(pc.valid, CacheMaybeValid::Full);

        // A present local file whose size differs from the remote is invalid.
        pc.inner.insert(
            "app#2.0.0#abc123.7z".to_owned(),
            FileDownloadInfo {
                url: "https://example.com/app.7z",
                local_size: 50,
                remote_size: 100,
                estimated: false,
            },
        );
        pc.update_valid_state();
        assert_eq!(pc.valid, CacheMaybeValid::None);
    }
}
