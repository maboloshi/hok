//! SQLite manifest cache — compatible with Scoop's `use_sqlite_cache` schema.
//!
//! Table `app` stores denormalized manifest data for fast querying without
//! re-parsing thousands of JSON files on every command.
//!
//! Table `hok_manifest_sync` is hok-only (upstream never touches it) and holds,
//! per bucket, the commit whose manifests the cached rows reflect. It is what
//! lets an update diff from the *last successful sync* instead of from this
//! run's pre-pull HEAD: a pull that turns out to be a no-op would otherwise
//! hide every manifest that changed since, leaving permanently stale rows.
//!
//! The cache is populated when:
//!   1. Buckets are updated (`hok update`)
//!   2. On first query if the DB is empty

use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{Connection, params};

use crate::{Session, error::Fallible, internal, package::Manifest};

/// Serialize an optional manifest field to JSON for the cache. Returns
/// `Some(None)` for a missing field, `Some(Some(s))` on success, and `None`
/// when serialization fails (caller skips the manifest instead of caching
/// an empty string).
fn json_field<T: serde::Serialize>(v: Option<T>) -> Option<Option<String>> {
    match v {
        None => Some(None),
        Some(val) => serde_json::to_string(&val).ok().map(Some),
    }
}

/// Cache entry matching the `app` table schema.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub name: String,
    pub description: String,
    pub version: String,
    pub bucket: String,
    pub manifest_json: String,
    pub binary: Option<String>,
    pub shortcut: Option<String>,
    pub dependency: Option<String>,
    pub suggest: Option<String>,
}

/// Open or create the SQLite manifest cache database at `{SCOOP}/scoop.db`.
pub fn open(session: &Session) -> Fallible<Connection> {
    let config = session.config();
    let db_path = config.root_path().join("scoop.db");

    internal::fs::ensure_dir(config.root_path())?;

    let conn = Connection::open(&db_path)?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS 'app' (
            name TEXT NOT NULL COLLATE NOCASE,
            description TEXT NOT NULL,
            version TEXT NOT NULL,
            bucket VARCHAR NOT NULL,
            manifest JSON NOT NULL,
            binary TEXT,
            shortcut TEXT,
            dependency TEXT,
            suggest TEXT,
            PRIMARY KEY (name, version, bucket)
        );",
    )?;

    conn.execute_batch("PRAGMA journal_mode=WAL;")?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS 'hok_manifest_sync' (
            bucket TEXT PRIMARY KEY,
            synced_commit TEXT NOT NULL
        );",
    )?;

    Ok(conn)
}

/// The commit each bucket's cached rows were last synced at.
///
/// A bucket without an entry cannot be diffed, so the caller must re-scan it
/// fully — that is also how a cache built before this table existed (or one
/// whose refresh failed) repairs itself.
pub fn synced_baselines(session: &Session) -> Fallible<HashMap<String, String>> {
    let conn = open(session)?;
    synced_baselines_conn(&conn)
}

/// [`synced_baselines`] against an already-open connection.
fn synced_baselines_conn(conn: &Connection) -> Fallible<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT bucket, synced_commit FROM hok_manifest_sync")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut baselines = HashMap::new();
    for row in rows {
        let (bucket, commit) = row?;
        baselines.insert(bucket, commit);
    }
    Ok(baselines)
}

/// Record the commit every listed bucket's cached rows now reflect.
pub(crate) fn record_synced(conn: &Connection, synced: &[(String, String)]) -> Fallible<()> {
    let tx = conn.unchecked_transaction()?;
    for (bucket, commit) in synced {
        tx.execute(
            "INSERT OR REPLACE INTO hok_manifest_sync (bucket, synced_commit) VALUES (?1, ?2)",
            params![bucket, commit],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Drop every sync marker, forcing the next refresh to rebuild the cache.
pub(crate) fn clear_synced(conn: &Connection) -> Fallible<()> {
    conn.execute("DELETE FROM hok_manifest_sync", [])?;
    Ok(())
}

/// Buckets whose cached rows may no longer match the bucket on disk: the sync
/// marker is missing, or the bucket's HEAD moved past it (a pull outside hok,
/// or a refresh that failed).
///
/// Empty when the SQLite cache is disabled — nothing is served from it then.
///
/// Buckets without a git HEAD (a plain directory) are skipped: there is nothing
/// to compare against, and every update re-scans them fully anyway.
pub fn stale_buckets(session: &Session) -> Fallible<Vec<String>> {
    if !session.config().use_sqlite_cache() {
        return Ok(Vec::new());
    }
    let conn = open(session)?;
    stale_buckets_of(&conn, session)
}

/// [`stale_buckets`] against an already-open connection.
fn stale_buckets_of(conn: &Connection, session: &Session) -> Fallible<Vec<String>> {
    let baselines = synced_baselines_conn(conn)?;
    let git = session.git();
    let mut stale = Vec::new();

    for bucket in crate::bucket::bucket_added(session)? {
        let name = bucket.name();
        let Some(head) = git.head_commit_id(bucket.path()) else {
            continue;
        };
        if baselines.get(name) != Some(&head) {
            stale.push(name.to_owned());
        }
    }
    Ok(stale)
}

/// Once-per-process latch for [`warn_if_cache_stale`].
static STALE_WARNED: AtomicBool = AtomicBool::new(false);

/// Warn once per process when the cache about to be served may trail the
/// buckets.
///
/// This is the only way the user learns that `info` / `search` are about to
/// answer from stale rows: the cache is refreshed by `hok update` alone, so the
/// notice names that command.
pub(crate) fn warn_if_cache_stale(session: &Session, conn: &Connection) {
    if STALE_WARNED.swap(true, Ordering::Relaxed) {
        return;
    }

    if let Ok(stale) = stale_buckets_of(conn, session)
        && !stale.is_empty()
    {
        session
            .output()
            .warn(hok_i18n::t!("cmd.cache_stale", count = stale.len()));
    }
}

/// Reset the once-per-process warning latch (tests only).
#[cfg(test)]
pub(crate) fn reset_stale_warning() {
    STALE_WARNED.store(false, Ordering::Relaxed);
}

/// A cached manifest whose manifest file on disk disagrees with the cache row.
///
/// Produced when an explicit query re-reads the file behind a cache hit
/// (`package::query`): the cache is only refreshed by `hok update`, so a bucket
/// changed outside it — a dirty working tree, a local directory bucket edited
/// by hand — leaves rows that trail the files until the next update.
pub(crate) enum CacheMismatch {
    /// The file on disk declares a different version than the cache row.
    Version {
        /// The version the cache served.
        cached: String,
        /// The version the file declares.
        file: String,
    },
    /// The file the cache row came from no longer exists: the row is phantom.
    Missing,
}

/// Packages already reported by [`warn_if_cache_mismatch`] in this process.
///
/// One command can reach the same package through several code paths
/// (`depends` resolves the package and then resolves its tree), and the notice
/// is about the cache, not about the command — once per package is enough.
static MISMATCH_WARNED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Warn that a cache hit disagreed with the file it came from, which the caller
/// has already resolved in favour of the file.
///
/// The notice is not optional: without it the user cannot tell that the cache
/// trailed the buckets, and would have no reason to run `hok update`.
pub(crate) fn warn_if_cache_mismatch(
    session: &Session,
    bucket: &str,
    name: &str,
    mismatch: &CacheMismatch,
) {
    let key = format!("{bucket}/{name}");
    {
        let mut warned = MISMATCH_WARNED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if warned.iter().any(|seen| seen == &key) {
            return;
        }
        warned.push(key);
    }

    let msg = match mismatch {
        CacheMismatch::Version { cached, file } => {
            hok_i18n::t!(
                "cmd.cache_mismatch",
                name = name,
                cached = cached,
                file = file
            )
        }
        CacheMismatch::Missing => hok_i18n::t!("cmd.cache_missing", name = name),
    };
    session.output().warn(msg);
}

/// Reset the once-per-package mismatch latch (tests only).
#[cfg(test)]
pub(crate) fn reset_mismatch_warning() {
    MISMATCH_WARNED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
}

/// Check if the cache has any data.
pub fn is_populated(conn: &Connection) -> Fallible<bool> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM app", [], |row| row.get(0))?;
    Ok(count > 0)
}

/// Serialize one manifest and insert its row into the cache transaction.
///
/// Returns `Ok(Some(version))` on success, or `Ok(None)` when the manifest is
/// not cacheable (unparseable JSON or a field that fails to serialize). The
/// caller decides what to do with an uncacheable manifest — a full `populate`
/// simply skips it, while an incremental upsert drops any previously cached
/// row to stay consistent.
fn insert_manifest_row(
    tx: &rusqlite::Transaction,
    name: &str,
    bucket: &str,
    raw_json: &str,
) -> Fallible<Option<String>> {
    // Validate by parsing; the version is needed to prune stale rows.
    let manifest = match Manifest::from_json(name, raw_json) {
        Ok(m) => m,
        Err(_) => return Ok(None),
    };
    // Not a package manifest: `version` is required, and upstream's DB writer
    // skips a JSON that has none (lib/database.ps1:176). A bucket directory can
    // hold unrelated JSON (`.markdownlint.json`, `appsettings.json`), which
    // must not surface as a package with an empty version.
    //
    // Slightly stricter than upstream: `$null -ne $manifest.version` accepts an
    // explicitly empty string, while the parsed manifest here cannot tell
    // "absent" from "empty", so both are skipped. No shipping bucket has been
    // seen to rely on an empty version.
    if manifest.version().is_empty() {
        return Ok(None);
    }

    let description = manifest.description().unwrap_or("").to_owned();
    let version = manifest.version().to_owned();
    // A field that fails to serialize is a cache-entry problem, not a
    // reason to corrupt the cache with an empty string — skip the
    // whole manifest (the cache is only an accelerator; queries fall
    // back to reading the file).
    let bin = match json_field(manifest.bin()) {
        Some(s) => s,
        None => return Ok(None),
    };
    let shortcut = match json_field(manifest.shortcuts()) {
        Some(s) => s,
        None => return Ok(None),
    };
    let dependency = match json_field(manifest.depends()) {
        Some(s) => s,
        None => return Ok(None),
    };
    let suggest = match json_field(manifest.suggest()) {
        Some(s) => s,
        None => return Ok(None),
    };

    tx.execute(
        "INSERT OR REPLACE INTO app (name, description, version, bucket, manifest, binary, shortcut, dependency, suggest)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            name,
            description,
            version,
            bucket,
            raw_json,
            bin,
            shortcut,
            dependency,
            suggest,
        ],
    )?;
    Ok(Some(version))
}

/// Read the raw text of a manifest file, or `None` if it is unreadable.
fn read_manifest_file(path: &std::path::Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    Some(s)
}

/// Populate the cache by reading all manifests from all added buckets.
///
/// Entirely replaces existing cache content in a single transaction.
pub fn populate(conn: &Connection, session: &Session) -> Fallible<()> {
    let buckets = crate::bucket::bucket_added(session)?;

    conn.execute("DELETE FROM app", [])?;
    let tx = conn.unchecked_transaction()?;

    for bucket in &buckets {
        let bucket_name = bucket.name().to_owned();
        let manifest_files = match bucket.manifests() {
            Ok(files) => files,
            Err(_) => continue,
        };

        for entry in &manifest_files {
            let path = entry.path();
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };

            // Read raw file content for lossless storage.
            let Some(raw_json) = read_manifest_file(&path) else {
                continue;
            };
            // The table was cleared above, so there are no stale rows to
            // prune — insert only.
            let _ = insert_manifest_row(&tx, name, &bucket_name, &raw_json)?;
        }
    }

    tx.commit()?;
    Ok(())
}

/// Apply a bucket-update manifest diff to the cache incrementally, without a
/// full re-scan.
///
/// For every upserted manifest the current version is (re)inserted and any
/// stale-version rows for the same `(name, bucket)` are dropped, so the cache
/// holds exactly the latest version per package (no historical accumulation).
/// An unparseable upsert drops the previously cached rows. Deleted manifests
/// remove their rows entirely.
/// [`apply_incremental_tx`] in its own transaction (the incremental tests drive
/// the diff apply directly).
#[cfg(test)]
fn apply_incremental(conn: &Connection, changes: &crate::bucket::UpdateChanges) -> Fallible<()> {
    let tx = conn.unchecked_transaction()?;
    apply_incremental_tx(&tx, changes)?;
    tx.commit()?;
    Ok(())
}

/// [`apply_incremental`] inside a caller-owned transaction.
fn apply_incremental_tx(
    tx: &rusqlite::Transaction<'_>,
    changes: &crate::bucket::UpdateChanges,
) -> Fallible<()> {
    for (bucket, path) in &changes.upsert {
        let Some(name) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_owned())
        else {
            continue;
        };

        let Some(raw_json) = read_manifest_file(path) else {
            // File unreadable (e.g. removed between diff and apply): drop rows.
            tx.execute(
                "DELETE FROM app WHERE name = ?1 AND bucket = ?2",
                params![name, bucket],
            )?;
            continue;
        };

        match insert_manifest_row(tx, &name, bucket, &raw_json)? {
            Some(version) => {
                // Keep only the latest version for this package.
                tx.execute(
                    "DELETE FROM app WHERE name = ?1 AND bucket = ?2 AND version <> ?3",
                    params![name, bucket, version],
                )?;
            }
            None => {
                // Manifest no longer cacheable: drop any previously cached rows.
                tx.execute(
                    "DELETE FROM app WHERE name = ?1 AND bucket = ?2",
                    params![name, bucket],
                )?;
            }
        }
    }

    for (bucket, name) in &changes.delete {
        tx.execute(
            "DELETE FROM app WHERE name = ?1 AND bucket = ?2",
            params![name, bucket],
        )?;
    }

    Ok(())
}

/// Re-scan one bucket: drop its rows and re-insert every manifest it holds.
///
/// This is the per-bucket rebuild — the cost is proportional to that bucket,
/// not to the whole cache. It is the fallback for a bucket the diff cannot
/// cover (no sync marker, no git remote, a failed pull).
///
/// The manifest list is collected *before* the rows are deleted: a bucket whose
/// directory cannot be read fails the refresh (which the caller reports and the
/// user retries) instead of silently emptying that bucket's cache.
fn rescan_bucket_tx(
    tx: &rusqlite::Transaction<'_>,
    session: &Session,
    bucket_name: &str,
) -> Fallible<usize> {
    let Some(bucket) = crate::bucket::bucket_added(session)?
        .into_iter()
        .find(|bucket| bucket.name() == bucket_name)
    else {
        // No longer an added bucket: its rows are dropped by the
        // not-added cleanup, not here.
        return Ok(0);
    };

    let files = bucket
        .manifests()?
        .into_iter()
        .map(|entry| {
            let name = entry
                .path()
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_owned);
            (name, entry.path())
        })
        .collect::<Vec<_>>();

    tx.execute("DELETE FROM app WHERE bucket = ?1", params![bucket_name])?;

    let mut inserted = 0;
    for (name, path) in files {
        let Some(name) = name else { continue };
        let Some(raw_json) = read_manifest_file(&path) else {
            continue;
        };
        if insert_manifest_row(tx, &name, bucket_name, &raw_json)?.is_some() {
            inserted += 1;
        }
    }
    Ok(inserted)
}

/// Drop the rows of buckets that are no longer added.
///
/// A per-bucket rescan only clears the buckets it re-scans, so a bucket removed
/// from the config would otherwise leave its rows in the cache for good
/// (`bucket::remove` does not touch the cache).
fn drop_buckets_not_added(tx: &rusqlite::Transaction<'_>, session: &Session) -> Fallible<()> {
    let added = crate::bucket::bucket_added(session)?;
    if added.is_empty() {
        tx.execute("DELETE FROM app", [])?;
        return Ok(());
    }

    let names = added
        .iter()
        .map(|bucket| bucket.name().to_owned())
        .collect::<Vec<_>>();
    let placeholders = (1..=names.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let params = names
        .iter()
        .map(|name| name as &dyn rusqlite::types::ToSql)
        .collect::<Vec<_>>();

    tx.execute(
        &format!("DELETE FROM app WHERE bucket NOT IN ({placeholders})"),
        params.as_slice(),
    )?;
    Ok(())
}

/// Query cache entries, optionally filtered by bucket and/or exact name.
///
/// Returns all entries if both filters are `None`.
pub fn query(
    conn: &Connection,
    bucket_filter: Option<&str>,
    name_filter: Option<&str>,
) -> Fallible<Vec<CacheEntry>> {
    let (where_clause, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
        match (bucket_filter, name_filter) {
            (Some(b), Some(n)) => (
                "WHERE bucket = ?1 AND name = ?2".into(),
                vec![Box::new(b.to_owned()), Box::new(n.to_owned())],
            ),
            (Some(b), None) => ("WHERE bucket = ?1".into(), vec![Box::new(b.to_owned())]),
            (None, Some(n)) => ("WHERE name = ?1".into(), vec![Box::new(n.to_owned())]),
            (None, None) => ("".into(), vec![]),
        };

    let sql = format!(
        "SELECT name, description, version, bucket, manifest, binary, shortcut, dependency, suggest FROM app {}",
        where_clause
    );
    let mut stmt = conn.prepare(&sql)?;

    let params_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();

    let rows = stmt.query_map(params_refs.as_slice(), |row| {
        Ok(CacheEntry {
            name: row.get(0)?,
            description: row.get(1)?,
            version: row.get(2)?,
            bucket: row.get(3)?,
            manifest_json: row.get(4)?,
            binary: row.get(5)?,
            shortcut: row.get(6)?,
            dependency: row.get(7)?,
            suggest: row.get(8)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Rebuild a `Manifest` from a cache entry.
pub fn entry_to_manifest(entry: &CacheEntry) -> Option<Manifest> {
    Manifest::from_json(&entry.name, &entry.manifest_json).ok()
}

/// What a [`refresh`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// The SQLite cache is disabled in config — nothing was refreshed.
    Disabled,
    /// Only the changed manifests were re-parsed.
    Incremental,
    /// These buckets were re-scanned in full; every other bucket kept its rows.
    Rescanned(Vec<String>),
    /// The whole cache was rebuilt (it was empty).
    Rebuilt,
}

/// Refresh the SQLite manifest cache after bucket updates.
///
/// Applies the manifest diff produced by `crate::bucket::update()`
/// incrementally — only the changed manifests are re-parsed. A bucket the diff
/// could not cover (no sync marker, no git remote, a failed pull) is re-scanned
/// on its own; the whole cache is rebuilt only when it is empty.
///
/// On success the commits the cache now reflects are recorded, which is the
/// baseline the next update diffs from. On failure the markers are dropped
/// (so a retry rebuilds from scratch) and the error is returned — the caller
/// reports it and the user retries, rather than a silently stale cache being
/// served from then on.
pub fn refresh(
    session: &Session,
    changes: &crate::bucket::UpdateChanges,
) -> Fallible<RefreshOutcome> {
    if !session.config().use_sqlite_cache() {
        return Ok(RefreshOutcome::Disabled);
    }
    let conn = open(session)?;

    // An empty cache has nothing to preserve: rebuild it in one go. Otherwise
    // one transaction covers everything — the per-bucket re-scans, the diffed
    // manifests and the removed-bucket cleanup are all-or-nothing.
    let cache_empty = match is_populated(&conn) {
        Ok(populated) => !populated,
        Err(_) => true,
    };
    let result = (|| -> Fallible<RefreshOutcome> {
        if cache_empty {
            populate(&conn, session)?;
            return Ok(RefreshOutcome::Rebuilt);
        }

        let tx = conn.unchecked_transaction()?;
        for bucket in &changes.full_scan {
            rescan_bucket_tx(&tx, session, bucket)?;
        }
        apply_incremental_tx(&tx, changes)?;
        drop_buckets_not_added(&tx, session)?;
        tx.commit()?;

        Ok(if changes.full_scan.is_empty() {
            RefreshOutcome::Incremental
        } else {
            RefreshOutcome::Rescanned(changes.full_scan.clone())
        })
    })();

    let outcome = match result {
        Ok(outcome) => outcome,
        Err(e) => {
            // Drop the markers: a retry (or the next update) re-scans every
            // bucket, which repairs whatever this failed refresh left behind.
            if let Err(e) = clear_synced(&conn) {
                tracing::warn!("failed to clear manifest sync markers: {e}");
            }
            return Err(e);
        }
    };

    if let Err(e) = record_synced(&conn, &changes.synced) {
        let _ = clear_synced(&conn);
        return Err(e);
    }

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bucket::UpdateChanges;
    use crate::test_utils::{test_session, tmpdir, write_bucket_manifest};

    /// Populate the cache from the session's buckets.
    fn populated(_root: &std::path::Path, session: &Session) -> Connection {
        let conn = open(session).unwrap();
        populate(&conn, session).unwrap();
        conn
    }

    fn row_count(conn: &Connection, name: &str, bucket: &str) -> Vec<CacheEntry> {
        query(conn, Some(bucket), Some(name)).unwrap()
    }

    fn manifest_path(root: &std::path::Path, bucket: &str, name: &str) -> std::path::PathBuf {
        root.join("buckets")
            .join(bucket)
            .join("bucket")
            .join(format!("{name}.json"))
    }

    #[test]
    fn incremental_version_bump_keeps_single_row() {
        let root = tmpdir("cache_bump");
        let session = test_session(&root);
        write_bucket_manifest(&root, "main", "foo", r#"{"version": "1.0"}"#);
        let conn = populated(&root, &session);

        let rows = row_count(&conn, "foo", "main");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].version, "1.0");

        // Bucket bumps foo to 2.0; the incremental diff upserts the file.
        write_bucket_manifest(&root, "main", "foo", r#"{"version": "2.0"}"#);
        let changes = UpdateChanges {
            upsert: vec![("main".to_owned(), manifest_path(&root, "main", "foo"))],
            delete: vec![],
            full_scan: vec![],
            synced: vec![],
        };
        apply_incremental(&conn, &changes).unwrap();

        // Exactly one row remains, at the new version — no orphaned 1.0 row.
        let rows = row_count(&conn, "foo", "main");
        assert_eq!(
            rows.len(),
            1,
            "version bump must not accumulate rows: {rows:?}"
        );
        assert_eq!(rows[0].version, "2.0");
    }

    #[test]
    fn incremental_delete_removes_rows() {
        let root = tmpdir("cache_delete");
        let session = test_session(&root);
        write_bucket_manifest(&root, "main", "foo", r#"{"version": "1.0"}"#);
        write_bucket_manifest(&root, "main", "bar", r#"{"version": "1.0"}"#);
        let conn = populated(&root, &session);
        assert_eq!(row_count(&conn, "bar", "main").len(), 1);

        // bar.json was deleted upstream; drop its rows.
        let changes = UpdateChanges {
            upsert: vec![],
            delete: vec![("main".to_owned(), "bar".to_owned())],
            full_scan: vec![],
            synced: vec![],
        };
        apply_incremental(&conn, &changes).unwrap();

        assert!(
            row_count(&conn, "bar", "main").is_empty(),
            "deleted manifest must vanish"
        );
        assert_eq!(
            row_count(&conn, "foo", "main").len(),
            1,
            "unrelated manifest must survive"
        );
    }

    #[test]
    fn incremental_unparseable_upsert_drops_rows() {
        let root = tmpdir("cache_broken");
        let session = test_session(&root);
        write_bucket_manifest(&root, "main", "foo", r#"{"version": "1.0"}"#);
        let conn = populated(&root, &session);
        assert_eq!(row_count(&conn, "foo", "main").len(), 1);

        // The manifest becomes unparseable; the upsert must drop the stale row.
        std::fs::write(manifest_path(&root, "main", "foo"), "{ not json").unwrap();
        let changes = UpdateChanges {
            upsert: vec![("main".to_owned(), manifest_path(&root, "main", "foo"))],
            delete: vec![],
            full_scan: vec![],
            synced: vec![],
        };
        apply_incremental(&conn, &changes).unwrap();

        assert!(
            row_count(&conn, "foo", "main").is_empty(),
            "unparseable manifest must not leave stale cached rows"
        );
    }

    /// A cache that trailed its bucket is repaired by re-scanning *that*
    /// bucket, and the marker is recorded afterwards.
    #[test]
    fn bucket_rescan_repairs_a_stale_row_and_records_the_marker() {
        let root = tmpdir("cache_repair");
        let session = test_session(&root);
        session
            .config_mut()
            .unwrap()
            .set("use_sqlite_cache", "true")
            .unwrap();
        write_bucket_manifest(&root, "main", "foo", r#"{"version": "1.0"}"#);
        let conn = populated(&root, &session);
        assert_eq!(row_count(&conn, "foo", "main")[0].version, "1.0");
        drop(conn);

        // The bucket moved on to 2.0 while the cache stayed at 1.0.
        write_bucket_manifest(&root, "main", "foo", r#"{"version": "2.0"}"#);

        let changes = UpdateChanges {
            upsert: vec![],
            delete: vec![],
            full_scan: vec!["main".to_owned()],
            synced: vec![("main".to_owned(), "c2".to_owned())],
        };
        assert_eq!(
            refresh(&session, &changes).unwrap(),
            RefreshOutcome::Rescanned(vec!["main".to_owned()])
        );

        let conn = open(&session).unwrap();
        assert_eq!(
            row_count(&conn, "foo", "main")[0].version,
            "2.0",
            "the rescan must replace the stale row"
        );
        assert_eq!(
            synced_baselines(&session)
                .unwrap()
                .get("main")
                .map(String::as_str),
            Some("c2"),
            "the synced commit becomes the next update's baseline"
        );
    }

    /// A re-scanned bucket must not disturb the buckets that were diffed: only
    /// the named bucket's rows are replaced.
    #[test]
    fn bucket_rescan_leaves_other_buckets_alone() {
        let root = tmpdir("cache_rescan_scope");
        let session = test_session(&root);
        session
            .config_mut()
            .unwrap()
            .set("use_sqlite_cache", "true")
            .unwrap();
        write_bucket_manifest(&root, "main", "foo", r#"{"version": "1.0"}"#);
        write_bucket_manifest(&root, "extras", "bar", r#"{"version": "1.0"}"#);
        let conn = populated(&root, &session);
        assert_eq!(row_count(&conn, "bar", "extras")[0].version, "1.0");
        drop(conn);

        // Both buckets moved on, but only `main` is flagged for a rescan.
        write_bucket_manifest(&root, "main", "foo", r#"{"version": "2.0"}"#);
        write_bucket_manifest(&root, "extras", "bar", r#"{"version": "2.0"}"#);

        let changes = UpdateChanges {
            upsert: vec![],
            delete: vec![],
            full_scan: vec!["main".to_owned()],
            synced: vec![("main".to_owned(), "c1".to_owned())],
        };
        refresh(&session, &changes).unwrap();

        let conn = open(&session).unwrap();
        assert_eq!(row_count(&conn, "foo", "main")[0].version, "2.0");
        assert_eq!(
            row_count(&conn, "bar", "extras")[0].version,
            "1.0",
            "a bucket that was not flagged keeps its rows (no whole-cache rebuild)"
        );
    }

    /// Rows of a bucket that is no longer added are dropped: a per-bucket
    /// rescan cannot be relied on to clean up after a removed bucket.
    #[test]
    fn removed_bucket_rows_are_dropped() {
        let root = tmpdir("cache_removed_bucket");
        let session = test_session(&root);
        session
            .config_mut()
            .unwrap()
            .set("use_sqlite_cache", "true")
            .unwrap();
        write_bucket_manifest(&root, "main", "foo", r#"{"version": "1.0"}"#);
        write_bucket_manifest(&root, "extras", "bar", r#"{"version": "1.0"}"#);
        let conn = populated(&root, &session);
        assert_eq!(row_count(&conn, "bar", "extras").len(), 1);
        drop(conn);

        std::fs::remove_dir_all(root.join("buckets").join("extras")).unwrap();
        refresh(&session, &UpdateChanges::default()).unwrap();

        let conn = open(&session).unwrap();
        assert!(
            row_count(&conn, "bar", "extras").is_empty(),
            "a removed bucket must not leave rows behind"
        );
        assert_eq!(
            row_count(&conn, "foo", "main").len(),
            1,
            "the remaining bucket is untouched"
        );
    }

    /// An empty cache is rebuilt as a whole (nothing to preserve).
    #[test]
    fn empty_cache_is_rebuilt() {
        let root = tmpdir("cache_empty_rebuild");
        let session = test_session(&root);
        session
            .config_mut()
            .unwrap()
            .set("use_sqlite_cache", "true")
            .unwrap();
        write_bucket_manifest(&root, "main", "foo", r#"{"version": "1.0"}"#);

        let changes = UpdateChanges {
            synced: vec![("main".to_owned(), "c1".to_owned())],
            ..Default::default()
        };
        assert_eq!(
            refresh(&session, &changes).unwrap(),
            RefreshOutcome::Rebuilt
        );

        let conn = open(&session).unwrap();
        assert_eq!(row_count(&conn, "foo", "main")[0].version, "1.0");
    }

    /// A JSON without a `version` is not a manifest (upstream skips it when
    /// writing its DB): it must not be cached, and it must not be returned by
    /// the file-based query either.
    #[test]
    fn json_without_a_version_is_not_a_package() {
        let root = tmpdir("cache_junk_json");
        let session = test_session(&root);
        session
            .config_mut()
            .unwrap()
            .set("use_sqlite_cache", "true")
            .unwrap();
        write_bucket_manifest(&root, "main", "foo", r#"{"version": "1.0"}"#);
        write_bucket_manifest(&root, "main", ".markdownlint", r#"{"MD013": false}"#);

        let conn = populated(&root, &session);
        assert_eq!(row_count(&conn, "foo", "main").len(), 1);
        assert!(
            row_count(&conn, ".markdownlint", "main").is_empty(),
            "a version-less JSON must not be cached as a package"
        );

        // The file-based path agrees with the cached one.
        let fresh = crate::package::query::query_synced_files(&session, &["*"], &[]).unwrap();
        let names = fresh.iter().map(|pkg| pkg.name()).collect::<Vec<_>>();
        assert!(names.contains(&"foo"), "names: {names:?}");
        assert!(!names.contains(&".markdownlint"), "names: {names:?}");
    }

    /// A refresh that fails is reported as an error and drops the markers, so
    /// the retry the user is asked for rebuilds the cache from scratch.
    #[test]
    fn failed_refresh_reports_an_error_and_drops_the_sync_markers() {
        let root = tmpdir("cache_fail_marker");
        let session = test_session(&root);
        session
            .config_mut()
            .unwrap()
            .set("use_sqlite_cache", "true")
            .unwrap();
        write_bucket_manifest(&root, "main", "foo", r#"{"version": "1.0"}"#);
        {
            let conn = open(&session).unwrap();
            populate(&conn, &session).unwrap();
            record_synced(&conn, &[("main".to_owned(), "c1".to_owned())]).unwrap();
            // Corrupt the cache table so the next refresh cannot write to it
            // (`open` keeps an existing table as it is).
            conn.execute("DROP TABLE app", []).unwrap();
            conn.execute("CREATE TABLE app (name TEXT)", []).unwrap();
        }

        let changes = UpdateChanges {
            upsert: vec![],
            delete: vec![],
            full_scan: vec![],
            synced: vec![("main".to_owned(), "c2".to_owned())],
        };
        assert!(
            refresh(&session, &changes).is_err(),
            "a failed refresh must surface as an error, not a silent flag"
        );

        assert!(
            synced_baselines(&session).unwrap().is_empty(),
            "dropped markers force a full rebuild on the retry"
        );
    }

    /// The staleness check that drives the read-path warning: a bucket with no
    /// marker, or whose HEAD moved past its marker, is reported — a synced one
    /// is not, and a disabled cache never reports anything.
    #[test]
    fn stale_buckets_reports_unsynced_and_moved_buckets() {
        use crate::test_utils::FakeGit;
        use std::sync::Arc;

        let root = tmpdir("cache_stale");
        let session = test_session(&root);
        write_bucket_manifest(&root, "main", "foo", r#"{"version": "1.0"}"#);
        session
            .config_mut()
            .unwrap()
            .set("use_sqlite_cache", "true")
            .unwrap();

        let bucket_dir = root.join("buckets").join("main");
        let fake = Arc::new(FakeGit::default());
        fake.head_ids
            .lock()
            .unwrap()
            .insert(bucket_dir.clone(), "head-1".to_owned());
        session.set_git(fake.clone()).unwrap();

        // Never synced: no marker at all.
        assert_eq!(stale_buckets(&session).unwrap(), vec!["main".to_owned()]);

        // Synced at the current HEAD: fresh.
        {
            let conn = open(&session).unwrap();
            record_synced(&conn, &[("main".to_owned(), "head-1".to_owned())]).unwrap();
        }
        assert!(stale_buckets(&session).unwrap().is_empty());

        // The bucket moved on (a pull outside hok): stale again.
        fake.head_ids
            .lock()
            .unwrap()
            .insert(bucket_dir, "head-2".to_owned());
        assert_eq!(stale_buckets(&session).unwrap(), vec!["main".to_owned()]);

        // With the cache disabled nothing is served from it.
        session
            .config_mut()
            .unwrap()
            .set("use_sqlite_cache", "false")
            .unwrap();
        assert!(stale_buckets(&session).unwrap().is_empty());
    }

    /// Serving rows from a stale cache warns through the session's output sink,
    /// which is how the user learns to run `hok update` instead of trusting
    /// them.
    #[test]
    fn serving_a_stale_cache_warns_once() {
        use crate::test_utils::FakeGit;
        use std::sync::{Arc, Mutex};

        let root = tmpdir("cache_warn");
        let session = test_session(&root);
        write_bucket_manifest(&root, "main", "foo", r#"{"version": "1.0"}"#);
        session
            .config_mut()
            .unwrap()
            .set("use_sqlite_cache", "true")
            .unwrap();

        let fake = Arc::new(FakeGit::default());
        fake.head_ids
            .lock()
            .unwrap()
            .insert(root.join("buckets").join("main"), "head-1".to_owned());
        session.set_git(fake).unwrap();

        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        session
            .set_output(Arc::new(move |out| sink.lock().unwrap().push(out)))
            .unwrap();

        reset_stale_warning();
        let packages = crate::package::query::query_synced(&session, &["*"], &[]).unwrap();
        assert_eq!(packages.len(), 1, "the cached query still returns the row");

        let warnings = seen.lock().unwrap().clone();
        assert!(
            warnings
                .iter()
                .any(|out| matches!(out, crate::output::Output::Warn(_))),
            "a stale cache must warn: {warnings:?}"
        );
    }

    /// Collect the warning texts a sink received.
    fn warnings_of(seen: &std::sync::Mutex<Vec<crate::output::Output>>) -> Vec<String> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|out| match out {
                crate::output::Output::Warn(msg) => Some(msg.clone()),
                _ => None,
            })
            .collect()
    }

    /// Install a sink that records everything the session emits.
    fn record_output(
        session: &Session,
    ) -> std::sync::Arc<std::sync::Mutex<Vec<crate::output::Output>>> {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        session
            .set_output(std::sync::Arc::new(move |out| {
                sink.lock().unwrap().push(out)
            }))
            .unwrap();
        seen
    }

    /// An explicit query re-reads the file behind the hit: a cache row that
    /// trails its bucket is answered with the version the file declares, and
    /// the disagreement is reported (the row would never heal by itself).
    #[test]
    fn explicit_query_serves_the_file_version() {
        let root = tmpdir("cache_verify_explicit");
        let session = test_session(&root);
        session
            .config_mut()
            .unwrap()
            .set("use_sqlite_cache", "true")
            .unwrap();
        write_bucket_manifest(&root, "main", "v_bump", r#"{"version": "1.0"}"#);
        let conn = populated(&root, &session);
        assert_eq!(row_count(&conn, "v_bump", "main")[0].version, "1.0");
        drop(conn);

        // The bucket moved on without an update: only the file knows.
        write_bucket_manifest(&root, "main", "v_bump", r#"{"version": "2.0"}"#);

        let seen = record_output(&session);
        reset_mismatch_warning();

        let packages = crate::package::query::query_synced(
            &session,
            &["v_bump"],
            &[crate::package::QueryOption::Explicit],
        )
        .unwrap();

        assert_eq!(packages.len(), 1);
        assert_eq!(
            packages[0].version(),
            "2.0",
            "an explicit hit must answer with the file's version"
        );
        let warnings = warnings_of(&seen);
        assert!(
            warnings
                .iter()
                .any(|msg| msg.contains("v_bump") && msg.contains("2.0")),
            "the mismatch must be reported, naming the package: {warnings:?}"
        );
    }

    /// A row whose file is gone is a phantom, not a package: an explicit query
    /// drops it instead of answering from the cache.
    #[test]
    fn explicit_query_drops_a_phantom_row() {
        let root = tmpdir("cache_verify_phantom");
        let session = test_session(&root);
        session
            .config_mut()
            .unwrap()
            .set("use_sqlite_cache", "true")
            .unwrap();
        write_bucket_manifest(&root, "main", "v_phantom", r#"{"version": "1.0"}"#);
        let _ = populated(&root, &session);

        std::fs::remove_file(manifest_path(&root, "main", "v_phantom")).unwrap();

        let seen = record_output(&session);
        reset_mismatch_warning();

        let packages = crate::package::query::query_synced(
            &session,
            &["v_phantom"],
            &[crate::package::QueryOption::Explicit],
        )
        .unwrap();

        assert!(
            packages.is_empty(),
            "a phantom row must not answer the query: {packages:?}"
        );
        let warnings = warnings_of(&seen);
        assert!(
            warnings.iter().any(|msg| msg.contains("v_phantom")),
            "the dropped row must be reported: {warnings:?}"
        );
    }

    /// A wildcard query has no single hit to check and stays purely cached: a
    /// listing of 20k manifests must not turn into 20k file reads.
    #[test]
    fn wildcard_query_stays_cached() {
        let root = tmpdir("cache_verify_wildcard");
        let session = test_session(&root);
        session
            .config_mut()
            .unwrap()
            .set("use_sqlite_cache", "true")
            .unwrap();
        write_bucket_manifest(&root, "main", "v_list", r#"{"version": "1.0"}"#);
        let _ = populated(&root, &session);

        write_bucket_manifest(&root, "main", "v_list", r#"{"version": "2.0"}"#);

        let packages = crate::package::query::query_synced(&session, &["*"], &[]).unwrap();

        assert_eq!(packages.len(), 1);
        assert_eq!(
            packages[0].version(),
            "1.0",
            "a listing must be served from the cache, not re-read from disk"
        );
    }

    /// WAL is what lets a reader and a writer share the cache: the reader keeps
    /// the snapshot it started on while a refresh commits, instead of the
    /// writer blocking (or failing with `SQLITE_BUSY`) until the read ends.
    /// With the default `delete` journal the concurrent write below cannot
    /// commit while that snapshot is open, so this also pins the mode down.
    #[test]
    fn wal_lets_a_reader_snapshot_survive_a_concurrent_write() {
        let root = tmpdir("cache_wal");
        let session = test_session(&root);
        write_bucket_manifest(&root, "main", "foo", r#"{"version": "1.0"}"#);
        let conn = populated(&root, &session);

        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal", "the cache must run in WAL mode");

        // An open read transaction pins the pre-write snapshot...
        let db_path = session.config().root_path().join("scoop.db");
        let reader = Connection::open(&db_path).unwrap();
        let tx = reader.unchecked_transaction().unwrap();
        let before: i64 = tx
            .query_row("SELECT COUNT(*) FROM app", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before, 1);

        // ...while a refresh committed through another connection succeeds.
        write_bucket_manifest(&root, "main", "bar", r#"{"version": "1.0"}"#);
        let changes = UpdateChanges {
            upsert: vec![("main".to_owned(), manifest_path(&root, "main", "bar"))],
            delete: vec![],
            full_scan: vec![],
            synced: vec![],
        };
        apply_incremental(&conn, &changes).unwrap();
        assert_eq!(row_count(&conn, "bar", "main").len(), 1);

        // The reader keeps its own snapshot rather than seeing the new row.
        let during: i64 = tx
            .query_row("SELECT COUNT(*) FROM app", [], |row| row.get(0))
            .unwrap();
        assert_eq!(during, 1, "an open reader must keep its snapshot");
        tx.finish().unwrap();

        let after: i64 = reader
            .query_row("SELECT COUNT(*) FROM app", [], |row| row.get(0))
            .unwrap();
        assert_eq!(after, 2, "a new snapshot sees the committed row");
    }
}
