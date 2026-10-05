//! Manifest history index: seed it once, then keep it current.
//!
//! `hok install app@<old version>` can fall back to the bucket's git history
//! when the manifest at HEAD carries no `autoupdate` section that can rebuild
//! the requested version. This module maintains the index that turns that
//! fallback into a point query — `(bucket, name, version) -> blob OID, path,
//! committed_at` — instead of a `git log --follow -G` pickaxe scan per lookup.
//!
//! Two paths, one invariant:
//!
//! - **seed** — a bucket with no usable baseline is walked once, from HEAD
//!   down its first-parent chain. Every [`SEED_BATCH_COMMITS`] commits are
//!   committed together with the cursor they reached, so an interrupted seed
//!   resumes where it stopped instead of starting over.
//! - **incremental** — a seeded bucket is walked from the new HEAD down to its
//!   recorded anchor, which is normally a handful of commits.
//!
//! Every failure here is a warning, never an error: the index is derived data
//! and `hok update` must not fail because it could not be maintained. A bucket
//! that fails keeps its old baseline, so the next update retries it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use rusqlite::{Connection, params};

use crate::internal::git::{HistoryCommit, HistoryRepo};
use crate::internal::json::parse_str;
use crate::{Session, error::Fallible};

/// Commits per transaction while seeding.
///
/// Seeding is the only long-running part of the index: one transaction per
/// commit would fsync ~90k times, and one transaction for the whole chain
/// would lose everything to an interruption. 2000 keeps both bounded, which is
/// also what makes a resumed seed cheap.
const SEED_BATCH_COMMITS: usize = 2000;

/// Commits per parallel chunk while indexing.
///
/// The chain walk itself is sequential and cheap — it reads no trees — so it is
/// decoupled from the expensive half (one tree diff, blob read and version
/// parse per commit), which is fanned out to the rayon pool in chunks of this
/// size. Small enough that every worker stays busy, large enough that the
/// per-chunk barrier is noise.
const INDEX_CHUNK_COMMITS: usize = 500;

/// Bring every added bucket's history index up to its current HEAD.
///
/// A bucket that cannot be indexed is warned about and skipped, so one broken
/// repository never holds up the others. Only a failure that concerns the
/// database itself (it cannot be opened) is returned as an error.
pub fn refresh(session: &Session) -> Fallible<()> {
    let conn = super::manifest_cache::open(session)?;
    let buckets = crate::bucket::bucket_added(session)?;
    for bucket in &buckets {
        if let Err(e) = refresh_bucket(&conn, bucket.name(), bucket.path()) {
            // Derived data: keep the old baseline so the next update retries,
            // and never surface this as a command failure.
            tracing::warn!(
                "manifest history index skipped bucket '{}': {e}",
                bucket.name()
            );
        }
    }

    // A bucket that is no longer added keeps neither rows nor a baseline: its
    // name can be reused by an unrelated repository, and the old rows would
    // then answer lookups with blob ids from a stranger. This mirrors what the
    // manifest cache does with its own rows (`drop_buckets_not_added`).
    drop_buckets_not_added(&conn, &buckets)?;
    Ok(())
}

/// Drop the index rows and baseline of buckets that are no longer added.
fn drop_buckets_not_added(conn: &Connection, added: &[crate::bucket::Bucket]) -> Fallible<()> {
    if added.is_empty() {
        conn.execute("DELETE FROM hok_manifest_history", [])?;
        conn.execute("DELETE FROM hok_manifest_history_sync", [])?;
        return Ok(());
    }

    let names = added.iter().map(|bucket| bucket.name()).collect::<Vec<_>>();
    let placeholders = (1..=names.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let params = names
        .iter()
        .map(|name| name as &dyn rusqlite::types::ToSql)
        .collect::<Vec<_>>();

    for table in ["hok_manifest_history", "hok_manifest_history_sync"] {
        conn.execute(
            &format!("DELETE FROM {table} WHERE bucket NOT IN ({placeholders})"),
            params.as_slice(),
        )?;
    }
    Ok(())
}

/// Seed or increment one bucket, depending on its recorded baseline.
fn refresh_bucket(conn: &Connection, bucket: &str, path: &Path) -> Fallible<()> {
    let Some(history) = HistoryRepo::open(path) else {
        // A plain directory bucket has no history to index.
        return Ok(());
    };
    let Some(head) = history.head() else {
        // An initialised but empty repository has nothing to walk.
        return Ok(());
    };

    let stored = baseline(conn, bucket)?;

    // A *completed* seed whose anchor this repository still contains: only the
    // commits added since then are new.
    if let Some((commit, true)) = stored.as_ref()
        && history.is_ancestor(&head, commit)
    {
        if commit == &head {
            return Ok(()); // Nothing moved since the last update.
        }
        return incremental(conn, &history, path, bucket, &head, commit);
    }

    // Everything else walks the chain: no baseline at all, a seed that was
    // interrupted, or an anchor this repository no longer contains (a re-clone
    // or a rewritten history). An interrupted seed resumes *at* its cursor —
    // inclusive, which re-processes that one commit and is a no-op because
    // rows are inserted with `INSERT OR IGNORE` — rather than being treated as
    // an anchor, which would mark the unscanned older half as done.
    let resume = match stored.as_ref() {
        Some((commit, false)) => Some(commit.as_str()),
        _ => None,
    };
    seed(conn, &history, path, bucket, &head, resume)
}

/// Scan the commits between the already-indexed `anchor` and `head`.
fn incremental(
    conn: &Connection,
    history: &HistoryRepo,
    path: &Path,
    bucket: &str,
    head: &str,
    anchor: &str,
) -> Fallible<()> {
    let tx = conn.unchecked_transaction()?;
    let mut chain = history.walk_chain(head, None, Some(anchor));
    while let Some(chunk) = next_chunk(&mut chain) {
        index_chunk(&tx, path, bucket, &chunk)?;
    }
    // The anchor commit is re-walked and its rows ignored; the new anchor is
    // HEAD, because everything between the two is now indexed.
    set_baseline(&tx, bucket, head, true)?;
    tx.commit()?;
    Ok(())
}

/// Walk the first-parent chain from `head` down, indexing every manifest
/// version it introduced.
///
/// `resume` is the cursor of an interrupted seed: the walk starts there
/// (inclusive) instead of at HEAD, and everything above it is already indexed.
/// Only a walk that reaches a root commit flips the bucket to seeded and
/// anchors it at HEAD — stopping anywhere else means the safety cap in
/// [`HistoryRepo::walk_chain`] cut the walk short, so the cursor is kept and
/// the next pass continues from it.
fn seed(
    conn: &Connection,
    history: &HistoryRepo,
    path: &Path,
    bucket: &str,
    head: &str,
    resume: Option<&str>,
) -> Fallible<()> {
    let mut tx = conn.unchecked_transaction()?;
    let mut written = 0usize;
    let mut cursor = head.to_owned();
    let mut reached_root = false;

    let mut chain = history.walk_chain(head, resume, None);
    while let Some(chunk) = next_chunk(&mut chain) {
        index_chunk(&tx, path, bucket, &chunk)?;
        written += chunk.len();
        if let Some(last) = chunk.last() {
            cursor.clone_from(&last.commit);
            reached_root = last.parent_tree.is_none();
        }

        if written >= SEED_BATCH_COMMITS {
            set_baseline(&tx, bucket, &cursor, false)?;
            tx.commit()?;
            tx = conn.unchecked_transaction()?;
            written = 0;
        }
    }

    let anchor = if reached_root { head } else { &cursor };
    set_baseline(&tx, bucket, anchor, reached_root)?;
    tx.commit()?;
    Ok(())
}

/// Pull at most [`INDEX_CHUNK_COMMITS`] commits off the chain walk, or `None`
/// once it is exhausted.
///
/// Chunking keeps the walk lazy: the chain of a large bucket is ~100k commits,
/// which is not worth materialising just to hand it to the pool.
fn next_chunk(chain: &mut impl Iterator<Item = HistoryCommit>) -> Option<Vec<HistoryCommit>> {
    let chunk: Vec<_> = chain.take(INDEX_CHUNK_COMMITS).collect();
    (!chunk.is_empty()).then_some(chunk)
}

/// Index every manifest version the commits of one chunk introduced.
///
/// The chunk is diffed in parallel — each rayon worker keeps its own
/// [`HistoryRepo`], because a `git2::Repository` is neither `Sync` nor safe to
/// share — and the rows are then inserted **in chain order**, so
/// `INSERT OR IGNORE` still keeps the newest commit's row for a version that
/// more than one commit introduced.
fn index_chunk(
    tx: &rusqlite::Transaction<'_>,
    repo_path: &Path,
    bucket: &str,
    chunk: &[HistoryCommit],
) -> Fallible<()> {
    for row in delta_rows(repo_path, chunk) {
        // `INSERT OR IGNORE` keeps the first writer, and the walk visits
        // commits newest-first — so a version carried by several commits keeps
        // the newest one's OID, path and date.
        tx.execute(
            "INSERT OR IGNORE INTO hok_manifest_history
                 (bucket, name, version, blob_oid, path, committed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                bucket,
                row.name,
                row.version,
                row.blob_oid,
                row.path,
                row.committed_at
            ],
        )?;
    }
    Ok(())
}

/// One index row, before the bucket name is attached.
struct HistoryRow {
    name: String,
    version: String,
    blob_oid: String,
    path: String,
    committed_at: i64,
}

/// Every version the commits of `chunk` introduced, in chain order.
fn delta_rows(repo_path: &Path, chunk: &[HistoryCommit]) -> Vec<HistoryRow> {
    // `par_iter().map(...).collect()` preserves the input order, which the
    // insert order above depends on; the per-commit `Vec`s are concatenated in
    // chain order afterwards rather than with `flatten` (rayon's `flatten` is
    // not order-preserving).
    let per_commit: Vec<Vec<HistoryRow>> = chunk
        .par_iter()
        .map(|entry| {
            with_worker_repo(repo_path, |history| match history {
                Some(history) => delta_rows_of(history, entry),
                None => Vec::new(),
            })
        })
        .collect();
    per_commit.into_iter().flatten().collect()
}

thread_local! {
    /// One repository handle per worker thread.
    ///
    /// A `git2::Repository` is neither `Sync` nor safe to share, and opening
    /// one is far too expensive to repeat for every chunk — a seed of a large
    /// bucket runs hundreds of chunks, and re-opening per chunk cost more than
    /// the parallelism saved (measured: `versions` 96.9 s serial → 191.6 s
    /// when every chunk re-opened its handles).
    static WORKER_REPOS: RefCell<HashMap<PathBuf, Option<HistoryRepo>>> =
        RefCell::new(HashMap::new());
}

/// Run `f` with this thread's handle to the repository at `path`.
///
/// `None` means the directory is not a repository; that answer is cached too,
/// so a plain directory bucket is not re-probed for every chunk.
fn with_worker_repo<T>(path: &Path, f: impl FnOnce(Option<&HistoryRepo>) -> T) -> T {
    WORKER_REPOS.with(|repos| {
        let mut repos = repos.borrow_mut();
        let history = repos
            .entry(path.to_path_buf())
            .or_insert_with(|| HistoryRepo::open(path));
        f(history.as_ref())
    })
}

/// Every index row one commit introduced.
fn delta_rows_of(history: &HistoryRepo, entry: &HistoryCommit) -> Vec<HistoryRow> {
    let mut rows = Vec::new();
    for delta in history.manifest_deltas(&entry.tree, entry.parent_tree.as_deref()) {
        // A deletion removes nothing: the versions a bucket no longer carries
        // at HEAD are exactly what this table is for.
        if delta.deleted {
            continue;
        }
        let Some(blob_oid) = delta.blob_oid.as_deref() else {
            continue;
        };
        // Unreadable (`gc --prune`) or non-UTF-8 content is skipped, so a
        // manifest the index cannot represent never becomes a row.
        let Some(text) = history.blob_text(blob_oid) else {
            continue;
        };
        let Some(name) = manifest_name(&delta.path) else {
            continue;
        };
        let Ok(probe) = parse_str::<VersionProbe>(&text) else {
            continue;
        };
        // Same cacheable rule as `insert_manifest_row`: a JSON without a
        // version is not a package manifest.
        let Some(version) = probe.version.filter(|version| !version.is_empty()) else {
            continue;
        };
        rows.push(HistoryRow {
            name: name.to_owned(),
            version,
            blob_oid: blob_oid.to_owned(),
            path: delta.path.clone(),
            committed_at: entry.committed_at,
        });
    }
    rows
}

/// The only manifest field the index stores.
///
/// Deliberately narrower than [`crate::package::Manifest`]: the seed reads
/// every historical blob of a bucket, and the full typed parse also builds
/// every URL, hash, architecture and autoupdate structure that the index then
/// throws away. Going through the narrow probe also keeps the custom
/// `License`/`Sourceforge` visitors out of the untrusted-history path, and
/// still shares [`parse_str`], so the JSON5-only manifests Scoop accepts are
/// indexed as well.
#[derive(serde::Deserialize)]
struct VersionProbe {
    #[serde(default)]
    version: Option<String>,
}

/// The package name a manifest path denotes: `bucket/foo.json` -> `foo`.
fn manifest_name(path: &str) -> Option<&str> {
    Path::new(path).file_stem().and_then(|stem| stem.to_str())
}

/// The recorded baseline of a bucket: the commit, and whether the seed that
/// produced it ran to the end of the chain.
fn baseline(conn: &Connection, bucket: &str) -> Fallible<Option<(String, bool)>> {
    let mut stmt = conn.prepare(
        "SELECT indexed_commit, seeded FROM hok_manifest_history_sync WHERE bucket = ?1",
    )?;
    let mut rows = stmt.query_map([bucket], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Record where a bucket's index got to.
fn set_baseline(
    tx: &rusqlite::Transaction<'_>,
    bucket: &str,
    commit: &str,
    seeded: bool,
) -> Fallible<()> {
    tx.execute(
        "INSERT OR REPLACE INTO hok_manifest_history_sync (bucket, indexed_commit, seeded)
         VALUES (?1, ?2, ?3)",
        params![bucket, commit, seeded],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::Manifest;
    use crate::test_utils::{test_session, tmpdir, write_bucket_manifest};
    use git2::{Repository, Signature, Time};

    fn sig() -> Signature<'static> {
        Signature::new("hok tester", "tester@example.com", &Time::new(0, 0)).unwrap()
    }

    /// A minimal but valid manifest body at `version`.
    fn manifest(version: &str) -> String {
        format!(r#"{{"version": "{version}"}}"#)
    }

    /// Open (or create) the bucket repository at `<root>/buckets/<bucket>`.
    fn bucket_repo(root: &Path, bucket: &str) -> Repository {
        let dir = root.join("buckets").join(bucket);
        std::fs::create_dir_all(dir.join("bucket")).unwrap();
        Repository::open(&dir).unwrap_or_else(|_| Repository::init(&dir).unwrap())
    }

    /// Write `rel` and commit it on HEAD, returning the new commit.
    ///
    /// The index is kept between calls, so a repository built with this helper
    /// accumulates files like a real bucket instead of replacing its tree.
    fn commit_file(repo: &Repository, rel: &str, content: &str) -> String {
        let full = repo.workdir().unwrap().join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&full, content).unwrap();

        let mut index = repo.index().unwrap();
        index.add_path(Path::new(rel)).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree_to(repo).unwrap()).unwrap();
        let parent = repo.head().ok().map(|h| h.peel_to_commit().unwrap());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig(), &sig(), rel, &tree, &parents)
            .unwrap()
            .to_string()
    }

    /// Remove `rel` from the worktree and the index, and commit the deletion.
    fn delete_file(repo: &Repository, rel: &str) -> String {
        std::fs::remove_file(repo.workdir().unwrap().join(rel)).unwrap();

        let mut index = repo.index().unwrap();
        index.remove_path(Path::new(rel)).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree_to(repo).unwrap()).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig(), &sig(), rel, &tree, &[&parent])
            .unwrap()
            .to_string()
    }

    /// `(name, version, blob_oid, path)` per indexed row, in version order.
    fn rows(conn: &Connection, bucket: &str) -> Vec<(String, String, String, String)> {
        let mut stmt = conn
            .prepare(
                "SELECT name, version, blob_oid, path FROM hok_manifest_history
                 WHERE bucket = ?1 ORDER BY name, version",
            )
            .unwrap();
        let mapped = stmt
            .query_map([bucket], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap();
        mapped.map(|row| row.unwrap()).collect()
    }

    /// The recorded `(indexed_commit, seeded)` of one bucket.
    fn baseline_of(conn: &Connection, bucket: &str) -> (String, bool) {
        conn.query_row(
            "SELECT indexed_commit, seeded FROM hok_manifest_history_sync WHERE bucket = ?1",
            [bucket],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
    }

    fn versions_of(conn: &Connection, bucket: &str) -> Vec<String> {
        rows(conn, bucket).into_iter().map(|row| row.1).collect()
    }

    /// Seeding walks the whole chain: every version the manifest ever carried
    /// gets a row, each pointing at the blob that really holds it.
    #[test]
    fn seed_indexes_every_version_in_the_chain() {
        let root = tmpdir("history_seed");
        let session = test_session(&root);
        let repo = bucket_repo(&root, "main");
        commit_file(&repo, "bucket/foo.json", &manifest("1.0"));
        commit_file(&repo, "bucket/foo.json", &manifest("2.0"));
        commit_file(&repo, "bucket/foo.json", &manifest("3.0"));
        // A commit that carries no manifest must not disturb the index.
        let head = commit_file(&repo, "README.md", "hello\n");

        refresh(&session).unwrap();

        let conn = crate::package::manifest_cache::open(&session).unwrap();
        let rows = rows(&conn, "main");
        assert_eq!(versions_of(&conn, "main"), ["1.0", "2.0", "3.0"]);
        assert!(
            rows.iter()
                .all(|row| row.0 == "foo" && row.3 == "bucket/foo.json")
        );

        // The read path's whole contract: the OID resolves to a manifest
        // whose version is the one the row promises.
        let history = HistoryRepo::open(root.join("buckets").join("main")).unwrap();
        for (_, version, blob_oid, _) in &rows {
            let text = history.blob_text(blob_oid).expect("blob must be readable");
            let manifest = Manifest::from_json("foo", &text).unwrap();
            assert_eq!(manifest.version(), version);
        }

        assert_eq!(baseline_of(&conn, "main"), (head, true));
    }

    /// Seeding the same bucket twice neither duplicates rows nor moves the
    /// baseline: the second pass has nothing to do.
    #[test]
    fn seed_is_idempotent() {
        let root = tmpdir("history_seed_twice");
        let session = test_session(&root);
        let repo = bucket_repo(&root, "main");
        commit_file(&repo, "bucket/foo.json", &manifest("1.0"));
        commit_file(&repo, "bucket/bar.json", &manifest("1.0"));

        refresh(&session).unwrap();
        let conn = crate::package::manifest_cache::open(&session).unwrap();
        let after_first = rows(&conn, "main");
        let baseline = baseline_of(&conn, "main");
        drop(conn);

        refresh(&session).unwrap();

        let conn = crate::package::manifest_cache::open(&session).unwrap();
        assert_eq!(rows(&conn, "main"), after_first);
        assert_eq!(baseline_of(&conn, "main"), baseline);
    }

    /// Two commits of one chunk may introduce the *same* version (a manifest
    /// is edited without a version bump). The row must keep the newer commit's
    /// blob: the chunk is diffed in parallel but inserted in chain order, so
    /// `INSERT OR IGNORE` still sees the newest commit first.
    #[test]
    fn same_version_in_one_chunk_keeps_the_newer_commit() {
        let root = tmpdir("history_same_version");
        let session = test_session(&root);
        let repo = bucket_repo(&root, "main");
        commit_file(
            &repo,
            "bucket/foo.json",
            r#"{"version": "1.0", "description": "old"}"#,
        );
        let head = commit_file(
            &repo,
            "bucket/foo.json",
            r#"{"version": "1.0", "description": "new"}"#,
        );

        refresh(&session).unwrap();

        let conn = crate::package::manifest_cache::open(&session).unwrap();
        let rows = rows(&conn, "main");
        assert_eq!(rows.len(), 1, "one row per (name, version)");
        let history = HistoryRepo::open(root.join("buckets").join("main")).unwrap();
        let text = history
            .blob_text(&rows[0].2)
            .expect("the recorded blob must be readable");
        assert!(
            text.contains("new"),
            "the newer commit's blob must win, got {text}"
        );
        assert_eq!(baseline_of(&conn, "main"), (head, true));
    }

    /// Incremental maintenance indexes every commit added since the seed —
    /// including intermediate versions, which a single diff between the
    /// baseline and HEAD would collapse into the newest one.
    #[test]
    fn incremental_indexes_every_commit_added_since_the_seed() {
        let root = tmpdir("history_incremental");
        let session = test_session(&root);
        let repo = bucket_repo(&root, "main");
        commit_file(&repo, "bucket/foo.json", &manifest("1.0"));

        refresh(&session).unwrap();
        let conn = crate::package::manifest_cache::open(&session).unwrap();
        assert_eq!(versions_of(&conn, "main"), ["1.0"]);
        drop(conn);

        // Two versions land between updates.
        commit_file(&repo, "bucket/foo.json", &manifest("2.0"));
        let head = commit_file(&repo, "bucket/foo.json", &manifest("3.0"));

        refresh(&session).unwrap();

        let conn = crate::package::manifest_cache::open(&session).unwrap();
        assert_eq!(versions_of(&conn, "main"), ["1.0", "2.0", "3.0"]);
        assert_eq!(baseline_of(&conn, "main"), (head, true));
    }

    /// A manifest that disappears from HEAD keeps its history rows: that is
    /// what separates this table from the current-version cache.
    #[test]
    fn deletion_keeps_the_history_rows() {
        let root = tmpdir("history_delete");
        let session = test_session(&root);
        let repo = bucket_repo(&root, "main");
        commit_file(&repo, "bucket/foo.json", &manifest("1.0"));
        commit_file(&repo, "bucket/foo.json", &manifest("2.0"));

        refresh(&session).unwrap();
        let head = delete_file(&repo, "bucket/foo.json");

        refresh(&session).unwrap();

        let conn = crate::package::manifest_cache::open(&session).unwrap();
        assert_eq!(
            versions_of(&conn, "main"),
            ["1.0", "2.0"],
            "a deletion must not drop history"
        );
        assert_eq!(baseline_of(&conn, "main"), (head, true));
    }

    /// An interrupted seed resumes *below* its cursor. Treating the cursor as
    /// an anchor would re-anchor the bucket at HEAD and leave the unscanned
    /// older half of the history missing for good.
    #[test]
    fn interrupted_seed_resumes_instead_of_re_anchoring() {
        let root = tmpdir("history_resume");
        let session = test_session(&root);
        let repo = bucket_repo(&root, "main");
        commit_file(&repo, "bucket/foo.json", &manifest("1.0"));
        let middle = commit_file(&repo, "bucket/foo.json", &manifest("2.0"));
        let head = commit_file(&repo, "bucket/foo.json", &manifest("3.0"));

        refresh(&session).unwrap();

        // Rewind the marker to "a batch had covered HEAD..middle when the seed
        // died", and drop the row that batch never reached.
        let conn = crate::package::manifest_cache::open(&session).unwrap();
        conn.execute(
            "UPDATE hok_manifest_history_sync SET indexed_commit = ?1, seeded = 0
             WHERE bucket = 'main'",
            [&middle],
        )
        .unwrap();
        conn.execute(
            "DELETE FROM hok_manifest_history WHERE bucket = 'main' AND version = '1.0'",
            [],
        )
        .unwrap();
        drop(conn);

        refresh(&session).unwrap();

        let conn = crate::package::manifest_cache::open(&session).unwrap();
        assert_eq!(
            versions_of(&conn, "main"),
            ["1.0", "2.0", "3.0"],
            "a resumed seed must walk below its cursor, not re-anchor at HEAD"
        );
        assert_eq!(baseline_of(&conn, "main"), (head, true));
    }

    /// An anchor the repository no longer contains (re-clone, rewritten
    /// history) forces a re-seed rather than a walk from a stranger commit.
    #[test]
    fn an_unknown_anchor_reseeds() {
        let root = tmpdir("history_rewritten");
        let session = test_session(&root);
        let repo = bucket_repo(&root, "main");
        commit_file(&repo, "bucket/foo.json", &manifest("1.0"));
        let head = commit_file(&repo, "bucket/foo.json", &manifest("2.0"));

        refresh(&session).unwrap();

        let conn = crate::package::manifest_cache::open(&session).unwrap();
        conn.execute(
            "UPDATE hok_manifest_history_sync
             SET indexed_commit = '9999999999999999999999999999999999999999', seeded = 1
             WHERE bucket = 'main'",
            [],
        )
        .unwrap();
        drop(conn);

        refresh(&session).unwrap();

        let conn = crate::package::manifest_cache::open(&session).unwrap();
        assert_eq!(versions_of(&conn, "main"), ["1.0", "2.0"]);
        assert_eq!(baseline_of(&conn, "main"), (head, true));
    }

    /// Flat and categorised buckets keep their manifests at the repository
    /// root or under a category directory rather than under `bucket/`, so the
    /// index must cover those layouts too — `manifest_walker`'s recursive
    /// `.json` minus `package.json` is the canonical rule.
    #[test]
    fn seed_covers_flat_and_categorised_layouts() {
        let root = tmpdir("history_layouts");
        let session = test_session(&root);
        let repo = bucket_repo(&root, "flat");
        commit_file(&repo, "flat-app.json", &manifest("1.0"));
        commit_file(&repo, "c/classified.json", &manifest("2.0"));
        // Repo-level npm metadata is not a package manifest.
        commit_file(&repo, "package.json", r#"{"name": "not-a-manifest"}"#);

        refresh(&session).unwrap();

        let conn = crate::package::manifest_cache::open(&session).unwrap();
        let indexed = rows(&conn, "flat")
            .into_iter()
            .map(|row| (row.0, row.1, row.3))
            .collect::<Vec<_>>();
        assert_eq!(
            indexed,
            [
                (
                    "classified".to_owned(),
                    "2.0".to_owned(),
                    "c/classified.json".to_owned()
                ),
                (
                    "flat-app".to_owned(),
                    "1.0".to_owned(),
                    "flat-app.json".to_owned()
                ),
            ]
        );
    }

    /// The index reads only the `version` field, through the shared parse
    /// chain: a JSON5-only manifest is indexed, and one whose `license` carries
    /// an unknown key never reaches the full typed parse. That second shape is
    /// what used to abort the seed — see
    /// `manifest::tests::license_unknown_keys_do_not_eat_the_next_member`.
    #[test]
    fn seed_indexes_manifests_through_the_narrow_version_probe() {
        let root = tmpdir("history_version_probe");
        let session = test_session(&root);
        let repo = bucket_repo(&root, "main");
        // JSON5-only: the trailing comma is rejected by strict `serde_json`.
        commit_file(&repo, "bucket/json5.json", "{\"version\": \"1.0\",}\n");
        // Parseable but schema-invalid: an unknown key inside `license`.
        commit_file(
            &repo,
            "bucket/typo.json",
            r#"{"version": "2.0", "license": {"identifier": "Freeware", "url:": "https://example.com/x"}}"#,
        );
        // No version: not a package manifest, so no row.
        commit_file(&repo, "bucket/noversion.json", r#"{"license": "MIT"}"#);

        refresh(&session).unwrap();

        let conn = crate::package::manifest_cache::open(&session).unwrap();
        assert_eq!(versions_of(&conn, "main"), ["1.0", "2.0"]);
    }

    /// Buckets without a repository are skipped, and one of them never stops
    /// the others from being indexed.
    #[test]
    fn buckets_without_a_repository_are_skipped() {
        let root = tmpdir("history_plain_bucket");
        let session = test_session(&root);
        // A plain directory bucket: manifests but no git history.
        write_bucket_manifest(&root, "local", "foo", &manifest("1.0"));
        let repo = bucket_repo(&root, "main");
        commit_file(&repo, "bucket/bar.json", &manifest("9.9"));

        refresh(&session).unwrap();

        let conn = crate::package::manifest_cache::open(&session).unwrap();
        assert!(rows(&conn, "local").is_empty());
        assert_eq!(
            versions_of(&conn, "main"),
            ["9.9"],
            "a bucket without history must not stop the others"
        );
    }

    /// A bucket that is removed keeps neither rows nor a baseline: its name
    /// can be reused by an unrelated repository, and stale rows would then
    /// answer lookups with blob ids from a stranger.
    #[test]
    fn removing_a_bucket_drops_its_rows_and_baseline() {
        let root = tmpdir("history_bucket_removed");
        let session = test_session(&root);
        let repo = bucket_repo(&root, "main");
        commit_file(&repo, "bucket/foo.json", &manifest("1.0"));
        let gone = bucket_repo(&root, "extras");
        commit_file(&gone, "bucket/bar.json", &manifest("2.0"));

        refresh(&session).unwrap();
        let conn = crate::package::manifest_cache::open(&session).unwrap();
        assert_eq!(versions_of(&conn, "extras"), ["2.0"]);
        drop(conn);

        std::fs::remove_dir_all(root.join("buckets").join("extras")).unwrap();
        refresh(&session).unwrap();

        let conn = crate::package::manifest_cache::open(&session).unwrap();
        assert!(
            rows(&conn, "extras").is_empty(),
            "a removed bucket must not keep rows"
        );
        let baselines: i64 = conn
            .query_row(
                "SELECT count(*) FROM hok_manifest_history_sync WHERE bucket = 'extras'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(baselines, 0, "nor a baseline to resume from");
        assert_eq!(
            versions_of(&conn, "main"),
            ["1.0"],
            "the remaining bucket is untouched"
        );
    }

    /// A database that cannot be opened is the one failure `refresh` reports:
    /// the call site logs it and keeps `hok update` at exit 0.
    #[test]
    fn an_unusable_database_is_reported() {
        let root = tmpdir("history_no_db");
        let session = test_session(&root);
        // `scoop.db` as a directory makes every connection attempt fail.
        std::fs::create_dir_all(root.join("scoop.db")).unwrap();

        assert!(refresh(&session).is_err());

        std::fs::remove_dir_all(root.join("scoop.db")).unwrap();
    }
}
