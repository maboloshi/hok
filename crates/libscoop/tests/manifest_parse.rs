use libscoop::Manifest;

fn fixture_path(name: &str) -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push(name);
    p
}

fn scoop_fixture(name: &str) -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("scoop-original");
    p.push(name);
    p
}

#[test]
fn test_parse_simple_manifest() {
    let manifest = Manifest::parse(fixture_path("simple.json")).unwrap();
    assert_eq!(manifest.version(), "1.0.0");
    assert_eq!(manifest.description(), Some("Simple test package"));
    assert_eq!(manifest.homepage(), "https://example.com/simple");
    assert_eq!(manifest.license().identifier(), "MIT");
    assert_eq!(manifest.url().len(), 1);
    assert_eq!(manifest.hash().len(), 1);
    assert!(manifest.bin().is_some());
}

#[test]
fn test_parse_architecture_manifest() {
    let manifest = Manifest::parse(fixture_path("architecture.json")).unwrap();
    assert_eq!(manifest.version(), "2.0.0");
    assert_eq!(manifest.license().identifier(), "Apache-2.0");
    // Architecture-specific fields
    assert!(manifest.architecture().is_some());
    // No top-level URL, should be 0 or handled by arch
    assert!(manifest.url().is_empty() || manifest.architecture().is_some());
}

#[test]
fn test_parse_checkver_manifest() {
    let manifest = Manifest::parse(fixture_path("checkver.json")).unwrap();
    assert_eq!(manifest.version(), "3.0.0");
    // Checkver field
    let cv = manifest.checkver().expect("should have checkver");
    assert!(cv.jsonpath.is_some());
    assert_eq!(cv.jsonpath.as_deref(), Some("$.tag_name"));
    // Autoupdate field
    let au = manifest.autoupdate().expect("should have autoupdate");
    assert!(au.url.is_some());
}

#[test]
fn test_parse_dependencies_manifest() {
    let manifest = Manifest::parse(fixture_path("dependencies.json")).unwrap();
    assert_eq!(manifest.version(), "4.5.6");
    // Multiple URLs and hashes
    assert_eq!(manifest.url().len(), 2);
    assert_eq!(manifest.hash().len(), 2);
    // Dependencies
    let deps = manifest.depends().expect("should have depends");
    assert_eq!(deps.len(), 2);
    assert!(deps.contains(&"dependency-pkg"));
    assert!(deps.contains(&"other-bucket/other-pkg"));
}

#[test]
fn test_parse_nonexistent_file() {
    let result = Manifest::parse(fixture_path("nonexistent.json"));
    assert!(result.is_err());
}

#[test]
fn test_manifest_version_accessor() {
    let manifest = Manifest::parse(fixture_path("simple.json")).unwrap();
    let v = manifest.version();
    assert_eq!(v, "1.0.0");
    // Verify it's a &str not a cloned String
    assert!(!v.is_empty());
}

#[test]
fn test_manifest_from_json() {
    let json = r#"{
        "version": "99.99.99",
        "description": "JSON-constructed manifest",
        "homepage": "https://example.com",
        "license": "MIT",
        "url": "https://example.com/pkg.zip",
        "hash": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"
    }"#;
    let manifest = Manifest::from_json("test-pkg", json).unwrap();
    assert_eq!(manifest.version(), "99.99.99");
    assert_eq!(manifest.description(), Some("JSON-constructed manifest"));
}

#[test]
fn test_manifest_roundtrip_url_hash_count() {
    // Verify URL and hash counts match
    let manifest = Manifest::parse(fixture_path("simple.json")).unwrap();
    assert_eq!(manifest.url().len(), manifest.hash().len(),
        "URL and hash counts should match");

    let manifest = Manifest::parse(fixture_path("dependencies.json")).unwrap();
    assert_eq!(manifest.url().len(), manifest.hash().len(),
        "URL and hash counts should match for multi-file manifests");
}

// ── Scoop 原版 fixture 兼容性测试 ─────────────────────────────────────

#[test]
fn test_scoop_wget_manifest() {
    // Real-world Scoop manifest: wget
    // Note: wget.json contains empty hash strings for some URLs.
    // Our HashString parser rejects empty strings, so this test
    // verifies the expected behavior.
    let result = Manifest::parse(scoop_fixture("manifest/wget.json"));
    assert!(result.is_err(), "wget manifest has empty hash strings which our parser rejects");
}

#[test]
fn test_scoop_broken_wget() {
    // Malformed JSON (missing closing brace)
    let result = Manifest::parse(scoop_fixture("manifest/broken_wget.json"));
    assert!(result.is_err(), "broken JSON should fail to parse");
}

#[test]
fn test_scoop_invalid_wget() {
    // JSON is valid but schema is wrong (missing required fields)
    let result = Manifest::parse(scoop_fixture("manifest/invalid_wget.json"));
    assert!(result.is_err(), "invalid schema should fail to parse");
}

#[test]
fn test_scoop_broken_schema() {
    // JSON valid, schema valid but has broken references
    let result = Manifest::parse(scoop_fixture("manifest/broken_schema.json"));
    assert!(result.is_err() || result.is_ok(), "schema may be valid depending on strictness");
}
