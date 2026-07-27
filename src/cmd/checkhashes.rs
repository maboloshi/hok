use clap::Parser;
use libscoop::operation;
use libscoop::{Manifest, Session};
use std::path::{Path, PathBuf};

use crate::{output, util, Result};

/// Verify and update manifest hashes
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = ".")]
    dir: PathBuf,

    /// Specific app(s) to check (supports wildcards, default: all)
    #[arg(default_value = "*")]
    app: Vec<String>,

    /// Update manifest with correct hash when mismatch found
    #[arg(short = 'u', long, action = clap::ArgAction::SetTrue)]
    update: bool,

    /// Force update manifest even when hash matches (re-hash)
    #[arg(short = 'f', long, action = clap::ArgAction::SetTrue)]
    force: bool,

    /// Use cache directory for downloaded files
    #[arg(short = 'c', long)]
    cache: Option<PathBuf>,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let dir = &args.dir;
    if !dir.is_dir() {
        output::err(rust_i18n::t!("cmd.checkhashes_err_dir", path = dir.display()));
        return Ok(());
    }

    let cache_dir = args
        .cache
        .unwrap_or_else(|| std::env::temp_dir().join("hok-checkhashes"));
    std::fs::create_dir_all(&cache_dir)?;

    let _proxy = session.config().proxy().map(|s| s.to_owned());

    let mut total = 0u32;
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut updated = 0u32;

    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }

        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        if args.app[0] != "*" && !args.app.iter().any(|p| name.contains(p.as_str())) {
            continue;
        }

        let manifest = match Manifest::parse(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let urls = manifest.url();
        let hashes = manifest.hash();
        if urls.is_empty() || hashes.is_empty() {
            continue;
        }

        print!("{name} ... ");
        total += 1;

        let url = urls[0].split('#').next().unwrap_or(urls[0]);
        let hash_str = &hashes[0];

        // Skip placeholders
        let raw_hash = hash_str.value();
        if raw_hash.is_empty() || raw_hash == "TODO" {
            output::warn(rust_i18n::t!("cmd.checkhashes_skipped"));
            continue;
        }

        // Download file
        let filename = url.split('/').last().unwrap_or("download");
        let cache_path = cache_dir.join(filename);

        if !cache_path.exists() || args.force {
            match operation::download_file(session, url, &cache_path) {
                Ok(()) => {}
                Err(e) => {
                    output::err(rust_i18n::t!("cmd.err_download", e = e));
                    failed += 1;
                    continue;
                }
            }
        }

        // Compute hash
        let actual_hash = match scoop_hash::compute_file_hash(&cache_path, hash_str.algorithm()) {
            Ok(h) => h,
            Err(e) => {
                output::err(rust_i18n::t!("cmd.err_hash", e = e));
                failed += 1;
                continue;
            }
        };

        if actual_hash == *raw_hash && !args.force {
            output::ok();
            passed += 1;
            continue;
        }

        // Hash mismatch or force update
        if args.update || args.force {
            if args.force {
                output::change(rust_i18n::t!("cmd.checkhashes_rehashed"), "->", &actual_hash[..12]);
            } else {
                output::change(rust_i18n::t!("cmd.checkhashes_mismatch_upd"), "->", &actual_hash[..12]);
            }
            update_json_hash(&path, hash_str.algorithm(), &actual_hash)?;
            updated += 1;
        } else {
            output::err(rust_i18n::t!("cmd.checkhashes_mismatch", expected = &raw_hash[..12], actual = &actual_hash[..12]));
            failed += 1;
        }
    }

    output::info(rust_i18n::t!("cmd.checkhashes_summary", total = total, passed = passed, failed = failed, updated = updated));

    Ok(())
}


/// Update hash in a manifest JSON file, preserving original formatting.
fn update_json_hash(path: &Path, algo: &str, actual_hash: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)?;

    let prefixed = match algo {
        "md5" => format!("md5:{actual_hash}"),
        "sha1" => format!("sha1:{actual_hash}"),
        "sha512" => format!("sha512:{actual_hash}"),
        _ => actual_hash.to_string(),
    };

    let old_root: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| anyhow::anyhow!("parse error: {e}"))?;

    // Find and modify the hash (arch-specific first, then top-level)
    let mut new_root = old_root.clone();
    let hash_paths = ["/architecture/64bit/hash", "/architecture/32bit/hash",
                       "/architecture/arm64/hash", "/hash"];
    let mut patched = content.clone();

    for ptr in &hash_paths {
        if let Some(old_val) = old_root.pointer(ptr) {
            if let Some(old_str) = old_val.as_str() {
                if !old_str.is_empty() {
                    // Set new value in the modified root
                    if let Some(target) = new_root.pointer_mut(ptr) {
                        *target = serde_json::Value::String(prefixed.clone());
                    }
                    // Text-patch only this field
                    if let Some(p) = util::patch_json_field(&patched, "hash", old_val,
                        &serde_json::Value::String(prefixed.clone()))
                    {
                        patched = p;
                        std::fs::write(path, patched.as_bytes())?;
                        return Ok(());
                    }
                }
            }
        }
    }

    Err(anyhow::anyhow!("no hash field found or replacement failed"))
}
