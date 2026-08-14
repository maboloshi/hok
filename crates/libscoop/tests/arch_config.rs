//! End-to-end test for the `default_architecture` config override
//! (Scoop's `Get-DefaultArchitecture` config path).
//!
//! Runs in its own test binary so the process-wide `OnceLock` override set
//! here cannot leak into other libscoop tests.

use libscoop::Arch;
use libscoop::Session;
use std::io::Write;

#[test]
fn test_default_architecture_config_override() {
    let dir = std::env::temp_dir().join("hok-arch-config-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hok.json");

    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(br#"{"default_architecture": "x86"}"#)
        .unwrap();

    let _session = Session::new_with(&path).unwrap();
    assert_eq!(
        Arch::current(),
        Arch::Ia32,
        "default_architecture=x86 must override runtime detection"
    );

    std::fs::remove_dir_all(&dir).ok();
}
