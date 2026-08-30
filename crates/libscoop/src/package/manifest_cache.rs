//! SQLite manifest cache — compatible with Scoop's `use_sqlite_cache` schema.
//!
//! Table `app` stores denormalized manifest data for fast querying without
//! re-parsing thousands of JSON files on every command.
//!
//! The cache is populated when:
//!   1. Buckets are updated (`hok update`)
//!   2. On first query if the DB is empty

use std::io::Read;

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

    Ok(conn)
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
fn apply_incremental(conn: &Connection, changes: &crate::bucket::UpdateChanges) -> Fallible<()> {
    let tx = conn.unchecked_transaction()?;

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

        match insert_manifest_row(&tx, &name, bucket, &raw_json)? {
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

    tx.commit()?;
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

/// Refresh the SQLite manifest cache after bucket updates.
///
/// Applies the manifest diff produced by `crate::bucket::update()`
/// incrementally — only the changed manifests are re-parsed. Falls back to a
/// full re-populate when the cache is empty or a bucket could not be cleanly
/// diffed. No-op if the SQLite cache is not enabled in config.
pub fn refresh(session: &Session, changes: &crate::bucket::UpdateChanges) {
    if !session.config().use_sqlite_cache() {
        return;
    }
    let conn = match open(session) {
        Ok(conn) => conn,
        Err(_) => {
            tracing::warn!("failed to open manifest cache");
            return;
        }
    };

    // Full rebuild on first use (empty cache), or when any bucket could not be
    // cleanly diffed (no remote, failed pull/diff) and must be re-scanned to
    // keep the cache complete. Otherwise apply only the changed manifests.
    let needs_full = match is_populated(&conn) {
        Ok(populated) => !populated,
        Err(_) => true,
    } || !changes.full_scan.is_empty();

    if needs_full {
        let _ = populate(&conn, session);
    } else if !changes.upsert.is_empty() || !changes.delete.is_empty() {
        let _ = apply_incremental(&conn, changes);
    }
    // Nothing changed and the cache is populated: no-op.
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
        };
        apply_incremental(&conn, &changes).unwrap();

        assert!(
            row_count(&conn, "foo", "main").is_empty(),
            "unparseable manifest must not leave stale cached rows"
        );
    }
}
