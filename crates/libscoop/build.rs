use std::env;
use std::fs;
use std::path::Path;

fn main() {
    embed_hok_shim();
    embed_known_buckets();
}

/// Generate `OUT_DIR/known_buckets.rs` from `assets/buckets.json` on every
/// build. The file is read at compile time, parsed, and baked into
/// `KNOWN_BUCKETS` / `BUCKET_PRIORITY` Rust constants — update the JSON and
/// the next build regenerates them.
fn embed_known_buckets() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let source = Path::new(&manifest_dir).join("assets/buckets.json");
    let content = fs::read_to_string(&source).unwrap_or_else(|e| {
        panic!("failed to read {}: {e}", source.display());
    });

    let map: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&content).expect("assets/buckets.json must be a JSON object");

    let mut entries = String::new();
    let mut keys = String::new();
    for (name, repo) in &map {
        let url = repo.as_str().expect("bucket repo must be a string");
        entries.push_str(&format!("    (\"{name}\", \"{url}\"),\n"));
        keys.push_str(&format!("    \"{name}\",\n"));
    }

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("known_buckets.rs");
    fs::write(
        &dest,
        format!(
            "pub const KNOWN_BUCKETS: &[(&str, &str)] = &[\n{entries}];\n\
             pub const BUCKET_PRIORITY_ORDER: &[&str] = &[\n{keys}];\n"
        ),
    )
    .expect("write known_buckets.rs");

    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed=build.rs");
}

fn embed_hok_shim() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let workspace_dir = Path::new(&manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root");

    // Two shim variants, selected at shim-creation time by the target's PE
    // subsystem:
    // - hok-shim (console)     — console targets; the shell waits for the
    //   shim so interactive children keep working
    // - hok-shim-gui (GUI)     — GUI targets; no console window on
    //   double-click and the shell does not wait
    let shims = [
        ("hok-shim", "HOK_SHIM_BYTES"),
        ("hok-shim-gui", "HOK_SHIM_GUI_BYTES"),
    ];
    let mut embedded = String::new();
    for (stem, const_name) in shims {
        // CARGO_CFG_TARGET_OS (not cfg!(windows), which is host-based) so a
        // cross-compile from a non-Windows host embeds the right name.
        let is_windows_target = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
        let shim_name = if is_windows_target {
            format!("{stem}.exe")
        } else {
            stem.to_string()
        };

        // Prefer release binary, fall back to debug
        let debug_path = workspace_dir.join("target/debug").join(&shim_name);
        let release_path = workspace_dir.join("target/release").join(&shim_name);

        let shim_src = if release_path.exists() {
            release_path.clone()
        } else if debug_path.exists() {
            debug_path.clone()
        } else {
            panic!(
                "hok-shim binary not found at {}\n  Run `cargo build -p hok-shim` first.",
                release_path.display()
            );
        };

        let shim_dest = Path::new(&out_dir).join(&shim_name);
        fs::copy(&shim_src, &shim_dest).expect("copy hok-shim to OUT_DIR");

        // Use include_bytes! on the copy in OUT_DIR (stable at compile time)
        embedded.push_str(&format!(
            "pub const {const_name}: &[u8] = include_bytes!({:?});\n",
            shim_dest.display()
        ));

        // Re-embed whenever either candidate changes (the picked one this
        // run may differ on the next).
        println!("cargo:rerun-if-changed={}", debug_path.display());
        println!("cargo:rerun-if-changed={}", release_path.display());
    }
    println!("cargo:rerun-if-changed=build.rs");

    let embedded_file = Path::new(&out_dir).join("embedded_shim.rs");
    fs::write(&embedded_file, embedded).expect("write embedded_shim.rs");
}
