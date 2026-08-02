//! Test utilities shared across libscoop test modules.
//!
//! Only compiled under `#[cfg(test)]`.

/// Create a temporary directory for testing, automatically cleaned up.
pub fn tmpdir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("hok_test_{}", name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Create a [`Session`][1] rooted at `root`: writes a minimal `config.json`
/// (with `root_path` and `cache_path` pointing at `root`) and loads it via
/// `Session::new_with`.
///
/// `cache_path` is pinned to `<root>/cache`: without it, the config falls
/// back to the global default cache dir (`~/scoop/cache`), which tests
/// running in parallel would clobber (same download filenames → same files).
///
/// [1]: crate::Session
pub fn test_session(root: &std::path::Path) -> crate::Session {
    let config_path = root.join("config.json");
    let root_escaped = root.to_string_lossy().replace('\\', "\\\\");
    let cache_escaped = root.join("cache").to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        &config_path,
        format!(
            r#"{{"root_path": "{}", "cache_path": "{}"}}"#,
            root_escaped, cache_escaped
        ),
    )
    .unwrap();
    crate::Session::new_with(&config_path).unwrap()
}

/// Write a Scoop manifest into `<root>/buckets/<bucket>/bucket/<name>.json`,
/// making it visible to `query_synced` / `query_installed`.
pub fn write_bucket_manifest(root: &std::path::Path, bucket: &str, name: &str, json: &str) {
    let dir = root.join("buckets").join(bucket).join("bucket");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{}.json", name)), json).unwrap();
}

/// Mark `name` as installed under `root` by creating
/// `<root>/apps/<name>/current/{manifest.json, install.json}`.
pub fn mark_installed(
    root: &std::path::Path,
    name: &str,
    bucket: &str,
    manifest_json: &str,
    held: bool,
) {
    let current = root.join("apps").join(name).join("current");
    std::fs::create_dir_all(&current).unwrap();
    std::fs::write(current.join("manifest.json"), manifest_json).unwrap();
    let install_info = format!(
        r#"{{"architecture": "64bit", "bucket": "{}", "hold": {}}}"#,
        bucket, held
    );
    std::fs::write(current.join("install.json"), install_info).unwrap();
}
