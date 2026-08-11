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

/// Write a minimal-but-parseable PE image with the given subsystem into
/// `path` (creating parent dirs). Used to exercise the shim-variant
/// selection against GUI (2) vs console (3) targets.
pub fn write_fake_pe(path: &std::path::Path, subsystem: u16) {
    use std::io::Write;
    let mut data = vec![0u8; 0x100];
    data[0] = b'M';
    data[1] = b'Z';
    // e_lfanew = 0x80
    data[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    // PE signature
    data[0x80..0x84].copy_from_slice(b"PE\0\0");
    // Subsystem at PE + 0x5C = 0x80 + 0x5C = 0xDC
    data[0xDC..0xDE].copy_from_slice(&subsystem.to_le_bytes());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(&data).unwrap();
}

/// Create a [`Session`][1] rooted at `root`: writes a minimal `hok.json`
/// (with `root_path` and `cache_path` pointing at `root`) and loads it via
/// `Session::new_with`.
///
/// `cache_path` is pinned to `<root>/cache`: without it, the config falls
/// back to the global default cache dir (`~/scoop/cache`), which tests
/// running in parallel would clobber (same download filenames → same files).
///
/// [1]: crate::Session
pub fn test_session(root: &std::path::Path) -> crate::Session {
    let config_path = root.join("hok.json");
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
