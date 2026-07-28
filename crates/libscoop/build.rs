use std::env;
use std::fs;
use std::path::Path;

fn main() {
    embed_hok_shim();
}

fn embed_hok_shim() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let shim_name = if cfg!(windows) { "hok-shim.exe" } else { "hok-shim" };

    // CARGO_MANIFEST_DIR = crates/libscoop/ → workspace root = crates/libscoop/../../
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let workspace_dir = Path::new(&manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root");

    let debug_path = workspace_dir.join("target/debug").join(&shim_name);
    let release_path = workspace_dir.join("target/release").join(&shim_name);

    // Prefer release binary, fall back to debug
    let shim_src = if release_path.exists() {
        release_path
    } else if debug_path.exists() {
        debug_path
    } else {
        panic!(
            "hok-shim binary not found at {}\n  Run `cargo build -p hok-shim` first.",
            release_path.display()
        );
    };

    let shim_dest = Path::new(&out_dir).join(&shim_name);
    fs::copy(&shim_src, &shim_dest).expect("copy hok-shim to OUT_DIR");

    let embedded = Path::new(&out_dir).join("embedded_shim.rs");
    // Use include_bytes! on the copy in OUT_DIR (stable at compile time)
    fs::write(
        &embedded,
        format!(
            "pub const HOK_SHIM_BYTES: &[u8] = include_bytes!({:?});\n",
            shim_dest.display()
        ),
    )
    .unwrap();

    println!("cargo:rerun-if-changed={}", shim_src.display());
    println!("cargo:rerun-if-changed=build.rs");
}
