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
