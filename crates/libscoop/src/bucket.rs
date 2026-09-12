//! Scoop bucket representation and management.
//!
//! A Scoop bucket is a directory (often a git repository) containing
//! package manifests in JSON format. This module provides [`Bucket`]
//! for inspecting individual buckets and free functions for listing,
//! adding, and removing buckets from a [`Session`].
//!
//! # Design
//!
//! - **Bucket as a folder**: A bucket is fundamentally a local path with
//!   manifest JSON files. The git remote is optional — local-only buckets
//!   (without a remote) are supported and simply skip updates.
//! - **Lazy metadata**: `remote_url` and `updated_at` are [`OnceLock`]
//!   fields, resolved on first access to avoid unnecessary git operations.
//! - **Concurrent listing**: `bucket_list()` uses `rayon` to parallelise
//!   bucket directory scanning for performance.
//! - **Known buckets**: `is_known_bucket()` checks against a built-in
//!   list (see `constant.rs`) to allow shorthand names like `"extras"`.
//!
//! # Important Notes
//!
//! - Buckets are identified by their **local directory name**, not by
//!   their remote URL. Renaming a bucket folder changes its identity.
//! - The `Bucket` struct is `Clone + Debug` and is designed to be cheaply
//!   cloneable (it holds only a path, a name, and two `OnceLock`s).

use rayon::prelude::{IntoParallelIterator, ParallelBridge, ParallelIterator};
use std::fs::DirEntry;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tracing::warn;

use crate::Session;
use crate::error::{Error, Fallible};
use crate::internal;

/// Scoop bucket representation.
///
/// A fact about Scoop bucket is that it is just a folder containing package
/// manifest files in JSON format. It could be simply a local directory or a
/// git repository.
#[derive(Clone, Debug)]
pub struct Bucket {
    /// The local path of the bucket.
    path: PathBuf,

    /// The name of the bucket.
    name: String,

    ///  The remote subscription url of the bucket.
    ///
    /// A Scoop bucket is generally a subscription to a remote git repository,
    /// this field will be the remote url of the git repository unless the git
    /// metadata is broken.
    ///
    /// Non-git bucket is also supported by Scoop, mostly it is a local directory
    /// which does not have a remote url, and bucket update is not supported.
    remote_url: OnceLock<Option<String>>,

    /// The HEAD commit author date of the bucket repo (cached).
    updated_at: OnceLock<Option<String>>,
}

impl Bucket {
    /// Create a bucket representation from a given local path.
    ///
    /// # Returns
    ///
    /// A bucket representation.
    ///
    /// # Errors
    ///
    /// This method will return an error if the given path does not exist. I/O
    /// errors will be returned if the bucket directory is not readable.
    pub fn from(path: &Path) -> Fallible<Bucket> {
        let path = path.to_owned();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap();

        if !path.exists() {
            return Err(Error::BucketNotFound(name));
        }

        let bucket = Bucket {
            path,
            name,
            remote_url: OnceLock::new(),
            updated_at: OnceLock::new(),
        };

        Ok(bucket)
    }

    /// Get the name of the bucket.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the path of the bucket.
    #[inline]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get manifest count of the bucket.
    ///
    /// # Returns
    ///
    /// The count of manifests in the bucket.
    #[inline]
    pub fn manifest_count(&self) -> usize {
        self.manifests().map(|v| v.len()).unwrap_or(0)
    }

    /// Get the remote url of the bucket.
    ///
    /// # Returns
    ///
    /// The git remote url of the bucket, otherwise none if the bucket is not a
    /// git repository, or it's a local git repository not having a remote url
    /// or the git metadata is broken.
    pub fn remote_url(&self) -> Option<&str> {
        self.remote_url
            .get_or_init(|| internal::git::remote_url_of(self.path(), "origin").unwrap_or(None))
            .as_deref()
    }

    /// Get the HEAD commit author date of the bucket.
    ///
    /// # Returns
    ///
    /// A formatted date string like "2026-07-25", or `None` if the bucket
    /// is not a git repository or git metadata is broken.
    pub fn updated_at(&self) -> Option<&str> {
        self.updated_at
            .get_or_init(|| {
                let secs = internal::git::head_commit_time(self.path())?;
                internal::time::format_short_local(secs)
            })
            .as_deref()
    }

    /// Get the source path of the bucket.
    ///
    /// # Returns
    ///
    /// Either the remote url of the bucket or the local path of the bucket.
    #[inline]
    pub fn source(&self) -> &str {
        self.remote_url().unwrap_or(self.path().to_str().unwrap())
    }

    /// Get the manifest path of the given package name.
    ///
    /// # Returns
    ///
    /// The path of the manifest file, none if the package is not in the bucket.
    pub(crate) fn path_of_manifest(&self, name: &str) -> Option<PathBuf> {
        let filename = format!("{}.json", name);

        let mut path = self.path().to_path_buf();
        path.push(&filename);

        if path.exists() {
            return Some(path);
        } else {
            path.pop();
            path.push("bucket");
            path.push(&filename);

            if path.exists() {
                return Some(path);
            } else {
                let first = name.chars().take(1).last().unwrap();
                let category = if first.is_ascii_lowercase() {
                    first.to_string()
                } else {
                    "#".to_owned()
                };

                path.pop();
                path.push(&category);
                path.push(&filename);

                if path.exists() {
                    return Some(path);
                }
            }
        }

        None
    }

    /// Get manifests from the bucket.
    ///
    /// # Returns
    ///
    /// a list of DirEntry of manifest files.
    ///
    /// # Errors
    ///
    /// I/O errors will be returned if the bucket directory is not readable.
    pub(crate) fn manifests(&self) -> Fallible<Vec<DirEntry>> {
        let mut path = self.path().to_owned();
        path.push("bucket");

        let iter = match par_read_dir(&path) {
            Ok(entries) => {
                // Skip entries whose metadata cannot be read (concurrent removal,
                // permissions, network drives) instead of panicking.
                type Classified = (Vec<(DirEntry, bool)>, Vec<(DirEntry, bool)>);
                let classified: Classified = entries
                    .filter_map(|de| de.file_type().ok().map(|ft| (de, ft.is_dir())))
                    .partition(|(_, is_dir)| *is_dir);
                let dirs = classified.0.into_iter().map(|(de, _)| de);
                let files = classified.1.into_iter().map(|(de, _)| de);

                // If the inner `bucket` directory contains subdirectories then it
                // is considered as a bucket with categories and we need to search
                // all subdirectories for manifest files.
                //
                // Category support was introduced in Scoop v0.3.0:
                // https://github.com/ScoopInstaller/Scoop/pull/5119
                let dirs = dirs.collect::<Vec<_>>();
                let files = files.collect::<Vec<_>>();
                if dirs.is_empty() {
                    files.into_par_iter()
                } else {
                    dirs.into_par_iter()
                        .filter_map(|de| par_read_dir(&de.path()).ok())
                        .flatten()
                        .collect::<Vec<_>>()
                        .into_par_iter()
                }
            }
            _ => {
                path.pop();
                par_read_dir(&path)?.collect::<Vec<_>>().into_par_iter()
            }
        };

        let ret = iter.filter(is_manifest).collect::<Vec<_>>();

        Ok(ret)
    }
}

/// Helper function to interate entries in a directory in parallel.
fn par_read_dir(path: &Path) -> std::io::Result<impl ParallelIterator<Item = DirEntry> + use<>> {
    let dir = path.read_dir()?;
    let path = path.to_owned();
    Ok(dir.par_bridge().filter_map(move |de| match de {
        Ok(entry) => Some(entry),
        Err(err) => {
            warn!(
                "failed to read directory entry in {} (err: {})",
                path.display(),
                err
            );
            None
        }
    }))
}

/// Helper function to check if a directory entry is a manifest file.
fn is_manifest(dir_entry: &DirEntry) -> bool {
    // Non-UTF-8 file names and unreadable metadata are not manifests —
    // skip them instead of panicking.
    let filename = dir_entry.file_name();
    let Some(name) = filename.to_str() else {
        return false;
    };
    let Ok(is_file) = dir_entry.file_type().map(|ft| ft.is_file()) else {
        return false;
    };
    // Ignore npm package config file, that said, there will
    // be no package named `package`, it's a reserved name.
    is_file && name.ends_with(".json") && name != "package.json"
}

/// Get a list of added buckets.
///
/// # Note
///
/// The returned list are unsorted.
pub fn bucket_added(session: &Session) -> Fallible<Vec<Bucket>> {
    let mut buckets = vec![];
    let buckets_dir = session.buckets_dir();

    match buckets_dir.read_dir() {
        Err(err) => {
            warn!("failed to read buckets dir (err: {})", err);
        }
        Ok(entries) => {
            buckets = entries
                .par_bridge()
                .filter_map(|entry| {
                    if let Ok(entry) = entry {
                        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                        let path = entry.path();

                        if is_dir {
                            match Bucket::from(&path) {
                                Err(err) => {
                                    warn!(
                                        "failed to parse bucket {} (err: {})",
                                        path.display(),
                                        err
                                    )
                                }
                                Ok(bucket) => return Some(bucket),
                            }
                        }
                    }
                    None
                })
                .collect::<Vec<_>>();
        }
    };

    Ok(buckets)
}

/// Bucket update progress context.
#[derive(Clone)]
pub struct BucketUpdateProgressContext {
    /// The name of the bucket.
    name: String,

    /// The update progress state of the bucket.
    state: BucketUpdateState,
}

impl BucketUpdateProgressContext {
    pub fn new(name: &str) -> BucketUpdateProgressContext {
        BucketUpdateProgressContext {
            name: name.to_owned(),
            state: BucketUpdateState::Started,
        }
    }

    /// Get the name of the bucket associated with this context.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the update progress state of the bucket.
    pub fn state(&self) -> &BucketUpdateState {
        &self.state
    }

    pub(crate) fn set_succeeded(&mut self) {
        self.state = BucketUpdateState::Succeeded;
    }

    pub(crate) fn set_failed(&mut self, msg: &str) {
        self.state = BucketUpdateState::Failed(msg.to_owned());
    }
}

/// Bucket update progress state.
#[derive(Clone, PartialEq)]
pub enum BucketUpdateState {
    /// The bucket is started to update.
    Started,

    /// The bucket is failed to update with the given error message.
    Failed(String),

    /// The bucket is updated successfully.
    Succeeded,
}

impl BucketUpdateState {
    /// Check if the state is started.
    pub fn started(&self) -> bool {
        self == &BucketUpdateState::Started
    }

    /// Check if the state is succeeded.
    pub fn succeeded(&self) -> bool {
        self == &BucketUpdateState::Succeeded
    }

    /// Check if the state is failed.
    pub fn failed(&self) -> Option<&str> {
        match self {
            BucketUpdateState::Failed(msg) => Some(msg),
            _ => None,
        }
    }
}

// ─── Session operations ────────────────────────────────────────────────────

/// Add a bucket to Scoop.
///
/// # Errors
///
/// This method will return an error if the bucket already exists, or the remote
/// url is not specified when adding a non built-in bucket.
///
/// A git error will be returned if failed to clone the bucket.
pub async fn add(session: &Session, name: &str, remote_url: &str) -> Fallible<()> {
    let (mut path, proxy) = {
        let config = session.config();
        let mut path = config.root_path().to_owned();
        path.push("buckets");
        let proxy = config.proxy().map(|s| s.to_owned());
        (path, proxy)
    };

    internal::fs::ensure_dir(&path)?;

    path.push(name);
    if path.exists() {
        return Err(Error::BucketAlreadyExists(name.to_owned()));
    }

    let remote_url = match remote_url.is_empty() {
        false => remote_url,
        true => crate::constant::BUILTIN_BUCKET_LIST
            .iter()
            .find(|&&(n, _)| n == name)
            .map(|&(_, remote)| remote)
            .ok_or_else(|| Error::BucketAddRemoteRequired(name.to_owned()))?,
    };

    session
        .git()
        .clone_repo(remote_url, &path, proxy.as_deref())
        .await
}

/// Get a list of added buckets.
///
/// # Returns
///
/// A list of added buckets sorted by name.Buckets cannot be parsed will be
/// filtered out.
///
/// # Errors
///
/// I/O errors will be returned if the `buckets` directory is not readable.
pub fn list(session: &Session) -> Fallible<Vec<Bucket>> {
    bucket_added(session).map(|mut buckets| {
        buckets.sort_by_key(|b| b.name().to_lowercase());
        buckets
    })
}

/// Get a list of known (built-in) buckets.
///
/// # Returns
///
/// A list of known buckets.
pub fn list_known() -> Vec<(&'static str, &'static str)> {
    crate::constant::BUILTIN_BUCKET_LIST.to_vec()
}

/// Update all added buckets. *
///
/// # Errors
///
/// I/O errors will be returned if the `buckets` directory is not readable or
/// failed to start up the update threads.
///
/// A [`ConfigInUse`][1] error will be returned if the config is borrowed elsewhere.
///
/// [1]: crate::Error::ConfigInUse
/// Manifest changes to apply to the SQLite manifest cache after updating
/// buckets, plus buckets that could not be cleanly diffed.
///
/// Produced by [`update`]. A consumer (the manifest-cache refresh) applies the
/// upserts/deletes incrementally, or falls back to a full re-scan when
/// [`UpdateChanges::full_scan`] is non-empty.
#[derive(Debug, Clone, Default)]
pub struct UpdateChanges {
    /// `(bucket, current manifest path)` for added / modified / renamed
    /// manifests — these should be (re)inserted into the cache.
    pub upsert: Vec<(String, std::path::PathBuf)>,
    /// `(bucket, manifest name)` for deleted manifests — these rows should be
    /// removed from the cache.
    pub delete: Vec<(String, String)>,
    /// Bucket names that were not cleanly diffed (never synced, no git remote,
    /// or the pull / diff failed) — the cache must fall back to re-scanning
    /// these fully.
    pub full_scan: Vec<String>,
    /// `(bucket, HEAD commit)` after the pull. The refresh records these as the
    /// commit each bucket's cached rows reflect, which is the baseline the next
    /// update diffs from.
    pub synced: Vec<(String, String)>,
}

/// Per-bucket outcome of a cache-tracking bucket update.
enum BucketUpdateOutcome {
    /// The bucket was pulled cleanly; re-insert these manifests and drop these
    /// names.
    Diff {
        upsert: Vec<std::path::PathBuf>,
        delete: Vec<String>,
    },
    /// The bucket could not be cleanly diffed — requires a full re-scan.
    Full,
    /// The bucket was pulled but no cache tracking was requested.
    None,
}

/// Turn a repo-relative diff of manifest files into absolute upsert paths and
/// deleted manifest names (basename without the `.json` extension).
fn classify_diff(
    repo: &std::path::Path,
    entries: Vec<internal::git::ManifestDiffEntry>,
) -> (Vec<std::path::PathBuf>, Vec<String>) {
    let mut upsert = Vec::new();
    let mut delete = Vec::new();
    for e in entries {
        if e.deleted {
            let name = std::path::Path::new(&e.path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            delete.push(name);
        } else {
            upsert.push(repo.join(&e.path));
        }
    }
    (upsert, delete)
}

pub async fn update(session: &Session) -> Fallible<UpdateChanges> {
    let buckets = bucket_added(session)?;
    let mut changes = UpdateChanges::default();

    if buckets.is_empty() {
        if let Some(tx) = session.emitter() {
            let _ = tx.send(crate::Event::BucketUpdateDone).await;
        }

        return Ok(changes);
    }

    // Only compute manifest diffs when the SQLite cache is enabled; otherwise
    // the update has no cache consumer and the extra git reads are pure waste.
    let track_cache = session.config().use_sqlite_cache();

    // Extract the proxy before the awaits; the config read guard is dropped
    // so a concurrent `config_mut()` is never blocked by the async pulls.
    let proxy = {
        let config = session.config();
        config.proxy().map(|s| s.to_owned())
    };
    let emitter = session.emitter();
    // The git capability is shareable across update tasks; tests inject an
    // in-memory fake here.
    let git = session.git();

    // Buckets without a remote are never pulled, so they can't be diffed —
    // mark them for a full re-scan so the cache stays complete.
    if track_cache {
        for b in buckets.iter().filter(|b| b.remote_url().is_none()) {
            changes.full_scan.push(b.name().to_owned());
        }
    }

    // Each bucket is diffed from the commit its cached rows were last synced
    // at — *not* from this run's pre-pull HEAD. A pull that is a no-op (the
    // repo already sits at the remote's HEAD) still has to replay everything
    // that changed since the last successful sync; diffing from the pre-pull
    // HEAD would report an empty diff and leave those rows stale for good.
    // A bucket with no marker (never synced, or the marker table is new) can't
    // be diffed and is re-scanned fully instead.
    let baselines = if track_cache {
        match crate::package::manifest_cache::synced_baselines(session) {
            Ok(baselines) => baselines,
            Err(e) => {
                // Without the markers nothing can be diffed and every bucket is
                // re-scanned below. That is the safe direction, but it is worth
                // saying why the work grew.
                tracing::warn!("cannot read the manifest cache sync markers: {e}");
                std::collections::HashMap::new()
            }
        }
    } else {
        std::collections::HashMap::new()
    };

    // Pull each bucket concurrently. git2 pulls are blocking, so each runs as
    // its own task on the runtime's worker pool (真 async orchestration; the
    // blocking body is offloaded by tokio).
    let handles: Vec<_> = buckets
        .iter()
        .filter(|b| b.remote_url().is_some())
        .map(|bucket| {
            let repo = bucket.path().to_owned();
            let name = bucket.name().to_owned();
            let proxy = proxy.clone();
            let git = git.clone();
            let emitter = emitter.clone();
            let baseline = baselines.get(&name).cloned();
            let track = track_cache;

            tokio::task::spawn(async move {
                let mut ctx = BucketUpdateProgressContext::new(name.as_str());

                if let Some(tx) = emitter.clone() {
                    let _ = tx
                        .send(crate::Event::BucketUpdateProgress(ctx.clone()))
                        .await;
                }

                // Diff from the last successfully synced commit (see above). A
                // bucket that was never synced has no baseline, so it cannot be
                // diffed and is re-scanned instead.
                let outcome = match git.pull(&repo, proxy.as_deref()).await {
                    Ok(_) => {
                        if let Some(tx) = emitter {
                            ctx.set_succeeded();
                            let _ = tx.send(crate::Event::BucketUpdateProgress(ctx)).await;
                        }
                        if track {
                            match baseline {
                                Some(from) => match git.diff_manifest_changes(&repo, &from) {
                                    Ok(entries) => {
                                        let (upsert, delete) = classify_diff(&repo, entries);
                                        BucketUpdateOutcome::Diff { upsert, delete }
                                    }
                                    Err(_) => BucketUpdateOutcome::Full,
                                },
                                None => BucketUpdateOutcome::Full,
                            }
                        } else {
                            BucketUpdateOutcome::None
                        }
                    }
                    Err(err) => {
                        if let Some(tx) = emitter {
                            ctx.set_failed(err.to_string().as_str());
                            let _ = tx.send(crate::Event::BucketUpdateProgress(ctx)).await;
                        }
                        BucketUpdateOutcome::Full
                    }
                };

                // The commit the cache reflects once this update's refresh has
                // run, whatever the outcome: a failed pull leaves the working
                // tree where it was, and the cache still matches it.
                let synced_at = if track {
                    git.head_commit_id(&repo)
                } else {
                    None
                };

                (name, outcome, synced_at)
            })
        })
        .collect();

    for handle in handles {
        match handle.await {
            Ok((name, outcome, synced_at)) => {
                if let Some(commit) = synced_at {
                    changes.synced.push((name.clone(), commit));
                }
                match outcome {
                    BucketUpdateOutcome::Diff { upsert, delete } => {
                        changes
                            .upsert
                            .extend(upsert.into_iter().map(|p| (name.clone(), p)));
                        changes
                            .delete
                            .extend(delete.into_iter().map(|n| (name.clone(), n)));
                    }
                    BucketUpdateOutcome::Full => changes.full_scan.push(name),
                    BucketUpdateOutcome::None => {}
                }
            }
            Err(e) => tracing::warn!("bucket update task failed to join: {e}"),
        }
    }

    // Scoop writes LAST_UPDATE unconditionally after the no-arg update
    // branch (scoop-update.ps1:399) — even when every bucket was skipped
    // — so the cooldown/outdated checks see that an update ran.
    let time = internal::time::format_last_update(time::OffsetDateTime::now_utc());
    {
        let mut config = session.config_mut()?;
        config.set("last_update", time.as_str())?;
        crate::config::write(&config)?;
    }

    if let Some(tx) = emitter {
        let _ = tx.send(crate::Event::BucketUpdateDone).await;
    }

    Ok(changes)
}

/// Remove a bucket from Scoop.
///
/// # Errors
///
/// This method will return an error if the bucket does not exist. I/O errors
/// will be returned if the bucket directory is unable to be removed.
pub fn remove(session: &Session, name: &str) -> Fallible<()> {
    let mut path = session.config().root_path().to_owned();
    path.push("buckets");
    path.push(name);

    if !path.exists() {
        return Err(Error::BucketNotFound(name.to_owned()));
    }

    Ok(std::fs::remove_dir_all(path.as_path())?)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn known_buckets_match_official_list() {
        // The compile-time generated list must contain all 10 official
        // ScoopInstaller buckets (assets/buckets.json), including the three
        // hok previously missed.
        let known = crate::constant::BUILTIN_BUCKET_LIST.to_vec();
        assert_eq!(known.len(), 10, "known buckets: {known:?}");
        for name in [
            "main",
            "extras",
            "versions",
            "nirsoft",
            "sysinternals",
            "php",
            "nerd-fonts",
            "nonportable",
            "java",
            "games",
        ] {
            assert!(
                known.iter().any(|(n, _)| *n == name),
                "missing known bucket {name}"
            );
        }
        // every known bucket has a repo url
        for (name, repo) in &known {
            assert!(
                repo.starts_with("https://"),
                "invalid repo for {name}: {repo}"
            );
        }
    }

    #[tokio::test]
    async fn bucket_priority_matches_known_keys() {
        // BUCKET_PRIORITY is generated from the same buckets.json key order,
        // so the two lists must agree on the key set.
        let known: Vec<&str> = crate::constant::BUILTIN_BUCKET_LIST
            .iter()
            .map(|(n, _)| *n)
            .collect();
        let priority = crate::constant::BUCKET_PRIORITY.to_vec();
        assert_eq!(known, priority, "priority must match known key order");
    }

    #[tokio::test]
    async fn bucket_add_uses_injected_git_fake() {
        use crate::test_utils::{FakeGit, test_session, tmpdir};
        use std::sync::Arc;

        let root = tmpdir("bucket_add_fake_git");
        let session = test_session(&root);
        let fake = Arc::new(FakeGit::default());
        session.set_git(fake.clone()).unwrap();

        crate::bucket::add(&session, "extras", "https://example.com/extras.git")
            .await
            .unwrap();

        let clones = fake.clones.lock().unwrap();
        assert_eq!(clones.len(), 1);
        assert_eq!(clones[0].0, "https://example.com/extras.git");
        assert!(
            clones[0].1.to_string_lossy().ends_with("buckets\\extras"),
            "clone path: {:?}",
            clones[0].1
        );
    }

    #[tokio::test]
    async fn bucket_update_uses_injected_git_fake() {
        use crate::test_utils::{FakeGit, test_session, tmpdir};
        use std::sync::Arc;

        let root = tmpdir("bucket_update_fake_git");
        let session = test_session(&root);

        // A real git repo with an origin remote so remote_url() reports it
        // as updatable (the metadata read still uses real git); the update's
        // pull itself goes through the injected fake, so no network is hit.
        let bucket_dir = root.join("buckets").join("extras");
        std::fs::create_dir_all(&bucket_dir).unwrap();
        let repo = git2::Repository::init(&bucket_dir).unwrap();
        repo.remote("origin", "https://example.com/extras.git")
            .unwrap();

        let fake = Arc::new(FakeGit::default());
        session.set_git(fake.clone()).unwrap();

        crate::bucket::update(&session).await.unwrap();

        let pulls = fake.pulls.lock().unwrap();
        assert_eq!(pulls.len(), 1, "pulls: {:?}", *pulls);
        assert_eq!(pulls[0], bucket_dir);
    }

    #[tokio::test]
    async fn bucket_update_tracks_cache_changes_via_git_fake() {
        use crate::internal::git::ManifestDiffEntry;
        use crate::test_utils::{FakeGit, test_session, tmpdir};
        use std::sync::Arc;

        let root = tmpdir("bucket_update_cache_track");
        let session = test_session(&root);

        // A real git repo with an origin remote so it is updatable; the pull
        // and diff go through the injected fake.
        let bucket_dir = root.join("buckets").join("extras");
        std::fs::create_dir_all(&bucket_dir).unwrap();
        let repo = git2::Repository::init(&bucket_dir).unwrap();
        repo.remote("origin", "https://example.com/extras.git")
            .unwrap();

        // Enable the SQLite cache so bucket::update tracks the manifest diff.
        session
            .config_mut()
            .unwrap()
            .set("use_sqlite_cache", "true")
            .unwrap();

        let fake = Arc::new(FakeGit::default());
        fake.head_ids
            .lock()
            .unwrap()
            .insert(bucket_dir.clone(), "abc123".to_owned());
        fake.diffs.lock().unwrap().insert(
            bucket_dir.clone(),
            vec![
                ManifestDiffEntry {
                    path: "bucket/foo.json".to_owned(),
                    deleted: false,
                },
                ManifestDiffEntry {
                    path: "bucket/bar.json".to_owned(),
                    deleted: true,
                },
            ],
        );
        session.set_git(fake.clone()).unwrap();

        // The bucket's cached rows were last synced at `old123`.
        {
            let conn = crate::package::manifest_cache::open(&session).unwrap();
            crate::package::manifest_cache::record_synced(
                &conn,
                &[("extras".to_owned(), "old123".to_owned())],
            )
            .unwrap();
        }

        let changes = crate::bucket::update(&session).await.unwrap();

        // foo.json is an upsert path under the extras bucket; bar.json a delete.
        assert_eq!(changes.upsert.len(), 1, "upserts: {:?}", changes.upsert);
        assert_eq!(changes.upsert[0].0, "extras");
        assert!(
            changes.upsert[0].1.ends_with("foo.json"),
            "upsert path: {:?}",
            changes.upsert[0].1
        );
        assert_eq!(
            changes.delete,
            vec![("extras".to_owned(), "bar".to_owned())]
        );
        assert!(
            changes.full_scan.is_empty(),
            "cleanly-diffed bucket must not be re-scanned fully: {:?}",
            changes.full_scan
        );

        // The diff starts at the last *synced* commit, never at this run's
        // pre-pull HEAD: a no-op pull must still replay everything that
        // changed since the cache was last refreshed.
        assert_eq!(
            *fake.diff_baselines.lock().unwrap(),
            vec![(bucket_dir.clone(), "old123".to_owned())],
            "the diff must start from the last synced commit"
        );
        // ... and the new HEAD becomes the next baseline.
        assert_eq!(
            changes.synced,
            vec![("extras".to_owned(), "abc123".to_owned())]
        );
    }

    /// A bucket the cache never synced carries no baseline, so it is re-scanned
    /// fully — no diff is requested at all.
    #[tokio::test]
    async fn bucket_update_rescans_a_bucket_without_a_sync_marker() {
        use crate::test_utils::{FakeGit, test_session, tmpdir};
        use std::sync::Arc;

        let root = tmpdir("bucket_update_no_marker");
        let session = test_session(&root);

        let bucket_dir = root.join("buckets").join("extras");
        std::fs::create_dir_all(&bucket_dir).unwrap();
        let repo = git2::Repository::init(&bucket_dir).unwrap();
        repo.remote("origin", "https://example.com/extras.git")
            .unwrap();
        session
            .config_mut()
            .unwrap()
            .set("use_sqlite_cache", "true")
            .unwrap();

        let fake = Arc::new(FakeGit::default());
        fake.head_ids
            .lock()
            .unwrap()
            .insert(bucket_dir.clone(), "abc123".to_owned());
        session.set_git(fake.clone()).unwrap();

        let changes = crate::bucket::update(&session).await.unwrap();

        assert_eq!(changes.full_scan, vec!["extras".to_owned()]);
        assert!(changes.upsert.is_empty() && changes.delete.is_empty());
        assert!(
            fake.diff_baselines.lock().unwrap().is_empty(),
            "without a baseline there is nothing to diff from"
        );
        assert_eq!(
            changes.synced,
            vec![("extras".to_owned(), "abc123".to_owned())],
            "the bucket is still recorded as synced at its current HEAD"
        );
    }
}
