use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    assert_sqlite_link_mode();
    embed_hok_shim();
    embed_known_buckets();
}

/// Reject the two ways the "link Windows' own `winsqlite3.dll` instead of
/// compiling the bundled amalgamation" setup goes wrong. Both are silent or
/// opaque without this check:
///
/// * `libsqlite3-sys` tests `LIBSQLITE3_SYS_USE_PKG_CONFIG` *before* the
///   `bundled` feature, so `SQLITE3_LIB_DIR` on its own is ignored and the C
///   amalgamation is compiled as usual — the build succeeds, just not the one
///   that was asked for.
/// * On x86 the `winsqlite3.lib` import library exports `__stdcall`-decorated
///   names (`winsqlite3.h` defines `SQLITE_APICALL` as `__stdcall` for
///   `NTDDI_VERSION >= NTDDI_WIN10_RS1`) while `rusqlite`/`libsqlite3-sys`
///   declare the same functions as cdecl `extern "C"`, so the link fails.
///
/// Both messages name the variables and print the commands to run instead.
fn assert_sqlite_link_mode() {
    for var in [
        "LIBSQLITE3_SYS_USE_PKG_CONFIG",
        "SQLITE3_LIB_DIR",
        "SQLITE3_INCLUDE_DIR",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }

    // Mirror libsqlite3-sys' own test: any value but "0" (including empty)
    // selects the linked path.
    let linked = env::var_os("LIBSQLITE3_SYS_USE_PKG_CONFIG").is_some_and(|v| v != "0");
    let lib_dir = env::var_os("SQLITE3_LIB_DIR");
    let include_dir = env::var_os("SQLITE3_INCLUDE_DIR");

    // The workspace root, without `..`: `CARGO_MANIFEST_DIR` is
    // `<root>/crates/libscoop`.
    let stage_dir = env::var("CARGO_MANIFEST_DIR")
        .ok()
        .map(std::path::PathBuf::from)
        .and_then(|dir| dir.parent().and_then(|p| p.parent()).map(Path::to_path_buf))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".winsqlite3");

    if !linked && (lib_dir.is_some() || include_dir.is_some()) {
        let set = if lib_dir.is_some() {
            "SQLITE3_LIB_DIR"
        } else {
            "SQLITE3_INCLUDE_DIR"
        };
        panic!(
            "\n\
             {set} is set, but LIBSQLITE3_SYS_USE_PKG_CONFIG is not — the setting would be \
             ignored.\n\
             \n\
             libsqlite3-sys only takes its \"link the system SQLite\" path when \
             LIBSQLITE3_SYS_USE_PKG_CONFIG is\n\
             non-zero. hok enables the `bundled` feature, and libsqlite3-sys checks that flag \
             first, so this\n\
             build would silently compile the bundled C amalgamation instead: it succeeds, but \
             it is not the\n\
             winsqlite3.dll build you asked for.\n\
             \n\
             Set all three variables and rebuild (winsqlite3 is x64/arm64 only). The\n\
             build script stages the SDK's winsqlite3 and sets them for its own process:\n\
             \n\
             \x20   pwsh scripts/build-hok.ps1 -Mode winsqlite3 -Verify\n\
             \n\
             Or do it by hand:\n\
             \n\
             \x20   $env:LIBSQLITE3_SYS_USE_PKG_CONFIG = '1'\n\
             \x20   $env:SQLITE3_LIB_DIR     = '{}'\n\
             \x20   $env:SQLITE3_INCLUDE_DIR = '{}'\n\
             \x20   cargo build -p hok --release\n\
             \n\
             Or drop the variable to get the default bundled build:\n\
             \n\
             \x20   Remove-Item Env:SQLITE3_LIB_DIR\n",
            stage_dir.display(),
            stage_dir.display(),
        );
    }

    let x86_windows = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86");

    if linked && x86_windows {
        panic!(
            "\n\
             Building for an x86 Windows target with LIBSQLITE3_SYS_USE_PKG_CONFIG set.\n\
             \n\
             winsqlite3.dll exports its API with __stdcall on x86 (winsqlite3.h defines \
             SQLITE_APICALL as\n\
             __stdcall when NTDDI_VERSION >= NTDDI_WIN10_RS1), so winsqlite3.lib carries \
             decorated names such\n\
             as _sqlite3_column_name@8. rusqlite and libsqlite3-sys declare those functions as \
             cdecl\n\
             `extern \"C\"`, so the link cannot resolve them and fails with:\n\
             \n\
             \x20   LNK2019: unresolved external symbol _sqlite3_column_name\n\
             \n\
             winsqlite3 cannot be used for i686/i586. Build the default bundled amalgamation \
             for that target:\n\
             \n\
             \x20   pwsh scripts/build-hok.ps1 -Target i686-pc-windows-msvc\n\
             \n\
             Or drop the variables and build by hand:\n\
             \n\
             \x20   Remove-Item Env:LIBSQLITE3_SYS_USE_PKG_CONFIG, Env:SQLITE3_LIB_DIR, \
             Env:SQLITE3_INCLUDE_DIR\n\
             \x20   cargo build --target i686-pc-windows-msvc --release\n"
        );
    }
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

    // The shim may live in a Cargo target directory outside the workspace
    // (`CARGO_TARGET_DIR`, `--target-dir`, `[build] target-dir`), so probe
    // every candidate instead of assuming `<workspace>/target`.
    let target_dirs = candidate_target_dirs(&out_dir, workspace_dir);
    println!("cargo:rerun-if-env-changed=CARGO_TARGET_DIR");

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

        // Prefer the release binary, fall back to debug. Every profile is
        // probed across every candidate target directory before moving to the
        // next profile, so "release wins over debug" stays global rather than
        // becoming per-directory.
        let mut candidates = Vec::new();
        for profile in ["release", "debug"] {
            for dir in &target_dirs {
                candidates.push(dir.join(profile).join(&shim_name));
            }
        }

        let Some(shim_src) = candidates.iter().find(|p| p.exists()).cloned() else {
            panic!(
                "hok-shim binary not found at {}\n  Run `cargo build -p hok-shim` first.",
                candidates[0].display()
            );
        };

        let shim_dest = Path::new(&out_dir).join(&shim_name);
        fs::copy(&shim_src, &shim_dest).expect("copy hok-shim to OUT_DIR");

        // Use include_bytes! on the copy in OUT_DIR (stable at compile time)
        embedded.push_str(&format!(
            "pub const {const_name}: &[u8] = include_bytes!({:?});\n",
            shim_dest.display()
        ));

        // Re-embed whenever any candidate changes (the picked one this run may
        // differ on the next).
        for candidate in &candidates {
            println!("cargo:rerun-if-changed={}", candidate.display());
        }
    }
    println!("cargo:rerun-if-changed=build.rs");

    let embedded_file = Path::new(&out_dir).join("embedded_shim.rs");
    fs::write(&embedded_file, embedded).expect("write embedded_shim.rs");
}

/// Every Cargo target directory that may hold a shim binary for this build,
/// most authoritative first.
///
/// `CARGO_TARGET_DIR` reaches a build script only when the user exported it
/// (Cargo does not re-export `--target-dir` or `[build] target-dir`), so the
/// directory is also derived from `OUT_DIR`, which always reflects the target
/// directory Cargo actually used. The workspace `target/` stays as the last
/// resort so an in-tree build keeps working even if `OUT_DIR` is unrecognised.
fn candidate_target_dirs(out_dir: &str, workspace_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = env::var_os("CARGO_TARGET_DIR").filter(|dir| !dir.is_empty()) {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(dir) = target_dir_from_out_dir(out_dir) {
        dirs.push(dir);
    }
    dirs.push(workspace_dir.join("target"));
    dirs.dedup();
    dirs
}

/// Walk up from `OUT_DIR` (`<target>/<profile>/build/<crate>-<hash>/out`) to
/// the first ancestor holding a `release` or `debug` directory; that ancestor
/// is Cargo's target directory. Walking instead of counting levels also covers
/// the extra `<triple>` level of a `--target <triple>` build.
fn target_dir_from_out_dir(out_dir: &str) -> Option<PathBuf> {
    Path::new(out_dir)
        .ancestors()
        .find(|dir| dir.join("release").is_dir() || dir.join("debug").is_dir())
        .map(Path::to_path_buf)
}
