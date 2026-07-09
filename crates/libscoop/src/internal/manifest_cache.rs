//! SQLite manifest cache — compatible with Scoop's `use_sqlite_cache` schema.
//!
//! Table `app` stores denormalized manifest data for fast querying without
//! re-parsing thousands of JSON files on every command.
//!
//! The cache is populated when:
//!   1. Buckets are updated (`hok update`)
//!   2. On first query if the DB is empty

use std::io::Read;

use rusqlite::{params, Connection};

use crate::{
    error::Fallible,
    internal, package::Manifest, Session,
};

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

/// Open or create the SQLite manifest cache database at `{cache}/scoop.db`.
pub fn open(session: &Session) -> Fallible<Connection> {
    let config = session.config();
    let cache_dir = config.cache_path();
    let db_path = cache_dir.join("scoop.db");

    internal::fs::ensure_dir(&cache_dir)?;

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
        );"
    )?;

    conn.execute_batch("PRAGMA journal_mode=WAL;")?;

    Ok(conn)
}

/// Check if the cache has any data.
pub fn is_populated(conn: &Connection) -> Fallible<bool> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM app", [], |row| row.get(0))?;
    Ok(count > 0)
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
            let name = match path.file_stem().and_then(|s| s.to_str()) {
                Some(n) => n.to_owned(),
                None => continue,
            };

            // Read raw file content for lossless storage
            let raw_json = match (|| -> Option<String> {
                let mut f = std::fs::File::open(&path).ok()?;
                let mut s = String::new();
                f.read_to_string(&mut s).ok()?;
                Some(s)
            })() {
                Some(s) => s,
                None => continue,
            };

            // Validate by parsing
            let manifest = match Manifest::from_json(&name, &raw_json) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let description = manifest.description().unwrap_or("").to_owned();
            let version = manifest.version().to_owned();
            let bin = manifest.bin().map(|v| {
                serde_json::to_string(&v).unwrap_or_default()
            });
            let shortcut = manifest.shortcuts().map(|v| {
                serde_json::to_string(&v).unwrap_or_default()
            });
            let dependency = manifest.depends().map(|v| {
                serde_json::to_string(&v).unwrap_or_default()
            });
            let suggest = manifest.suggest().map(|v| {
                serde_json::to_string(&v).unwrap_or_default()
            });

            tx.execute(
                "INSERT OR REPLACE INTO app (name, description, version, bucket, manifest, binary, shortcut, dependency, suggest)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    name,
                    description,
                    version,
                    bucket_name,
                    raw_json,
                    bin,
                    shortcut,
                    dependency,
                    suggest,
                ],
            )?;
        }
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
    let (where_clause, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match (bucket_filter, name_filter) {
        (Some(b), Some(n)) => ("WHERE bucket = ?1 AND name = ?2".into(), vec![Box::new(b.to_owned()), Box::new(n.to_owned())]),
        (Some(b), None) => ("WHERE bucket = ?1".into(), vec![Box::new(b.to_owned())]),
        (None, Some(n)) => ("WHERE name = ?1".into(), vec![Box::new(n.to_owned())]),
        (None, None) => ("".into(), vec![]),
    };

    let sql = format!(
        "SELECT name, description, version, bucket, manifest, binary, shortcut, dependency, suggest FROM app {}",
        where_clause
    );
    let mut stmt = conn.prepare(&sql)?;

    let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

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
