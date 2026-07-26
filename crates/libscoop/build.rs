// Build hok-shim and embed it into libscoop.
// The hok-shim binary is built and included via include_bytes! at compile time.

use std::path::Path;
use std::process::Command;

fn main() {
    // Determine hok project root (parent of the crates/ directory)
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let hok_root = Path::new(&manifest_dir).parent().unwrap().parent().unwrap().to_owned();

    // Build hok-shim release binary
    let status = Command::new("cargo")
        .args(["build", "-p", "hok-shim", "--release"])
        .current_dir(&hok_root)
        .status()
        .expect("failed to run cargo build for hok-shim");
    assert!(status.success(), "hok-shim build failed");

    // Generate the embedded code
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("embedded_shim.rs");
    // Use a relative path from the OUT_DIR to the built binary.
    // Since target/release is at workspace root, and OUT_DIR is
    // somewhere deep in target/debug/build/libscoop-<hash>/out,
    // we need a relative path that works. The simplest: use a
    // path relative to the CARGO_MANIFEST_DIR (libscoop root).
    // But include_bytes! requires a path that exists at compile time.
    //
    // OUT_DIR is stable during the build, so we can copy the shim
    // there and include it.
    let shim_src = hok_root.join("target").join("release").join("hok-shim.exe");
    let shim_dest = Path::new(&out_dir).join("hok-shim.exe");
    std::fs::copy(&shim_src, &shim_dest).expect("failed to copy hok-shim.exe to OUT_DIR");

    std::fs::write(
        &dest,
        "pub const HOK_SHIM_BYTES: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/hok-shim.exe\"));\n",
    )
    .expect("failed to write embedded_shim.rs");

    // Rerun if hok-shim source, Cargo.toml, or this build.rs changes
    let shim_src_dir = hok_root.join("crates").join("hok-shim").join("src");
    println!("cargo:rerun-if-changed={}", shim_src_dir.join("main.rs").display());
    println!("cargo:rerun-if-changed={}", hok_root.join("crates").join("hok-shim").join("Cargo.toml").display());
    println!("cargo:rerun-if-changed={}", Path::new(&manifest_dir).join("build.rs").display());
}
