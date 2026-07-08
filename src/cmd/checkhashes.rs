use clap::Parser;
use crossterm::style::Stylize;
use scoop_hash::ChecksumBuilder;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::Result;

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

pub fn execute(args: Args) -> Result<()> {
    let dir = &args.dir;
    if !dir.is_dir() {
        eprintln!("error: '{}' is not a directory", dir.display());
        return Ok(());
    }

    let cache_dir = args
        .cache
        .unwrap_or_else(|| std::env::temp_dir().join("hok-checkhashes"));
    std::fs::create_dir_all(&cache_dir)?;

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

        // Apply app filter
        if args.app[0] != "*" {
            if !args.app.iter().any(|p| name.contains(p.as_str())) {
                continue;
            }
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let mut manifest: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Get architecture-specific or top-level URL + hash
        let entries = extract_url_hash_pairs(&manifest);

        if entries.is_empty() {
            continue;
        }

        print!("{} ... ", name);
        total += 1;

        // Pick the first entry for check (most manifests have 1 file)
        let (url, declared_hash) = &entries[0];

        if declared_hash.is_empty() || *declared_hash == "TODO" {
            println!("{}", "skipped (no hash)".yellow());
            continue;
        }

        // Download the file
        let filename = url
            .split('/')
            .last()
            .unwrap_or("download")
            .split('?')
            .next()
            .unwrap_or("download");
        let cache_path = cache_dir.join(&filename);

        if !cache_path.exists() || args.force {
            let status = std::process::Command::new("curl.exe")
                .args(["-sSLo", cache_path.to_str().unwrap(), url])
                .status()
                .map_err(|e| {
                    anyhow::anyhow!("failed to run curl.exe (install curl?): {}", e)
                })?;

            if !status.success() {
                println!("{}", "download failed".red());
                failed += 1;
                continue;
            }
        }

        // Compute hash
        let algo = if declared_hash.starts_with("md5:") {
            "md5"
        } else if declared_hash.starts_with("sha1:") {
            "sha1"
        } else if declared_hash.starts_with("sha512:") {
            "sha512"
        } else {
            "sha256"
        };

        let actual_hash = match compute_hash(&cache_path, algo) {
            Ok(h) => h,
            Err(e) => {
                println!("{}: {}", "hash error".red(), e);
                failed += 1;
                continue;
            }
        };

        let declared_clean = declared_hash
            .strip_prefix("md5:")
            .or_else(|| declared_hash.strip_prefix("sha1:"))
            .or_else(|| declared_hash.strip_prefix("sha512:"))
            .or_else(|| declared_hash.strip_prefix("sha256:"))
            .unwrap_or(declared_hash);

        if actual_hash == declared_clean && !args.force {
            println!("{}", "ok".green());
            passed += 1;
            continue;
        }

        if actual_hash == declared_clean && args.force {
            // Force update — rewrite with same hash (reformatting + re-hashing)
        }

        // Hash mismatch (or force update)
        if args.update || args.force {
            update_manifest_hash(&mut manifest, &actual_hash, algo)?;

            let formatted = serde_json::to_string_pretty(&manifest)
                .map_err(|e| anyhow::anyhow!("serialize error: {}", e))?;
            std::fs::write(&path, formatted.as_bytes())?;

            if args.force {
                println!("{}", format!("re-hashed -> {}", &actual_hash[..12]).blue());
            } else {
                println!(
                    "{} {} -> {}",
                    "hash mismatch! updated".yellow(),
                    &declared_clean[..12],
                    &actual_hash[..12]
                );
            }
            updated += 1;
        } else {
            println!(
                "{} expected {} got {}",
                "hash mismatch".red(),
                &declared_clean[..12],
                &actual_hash[..12]
            );
            failed += 1;
        }
    }

    println!(
        "\n{}",
        format!(
            "Scanned {}: {} ok, {} failed, {} updated.",
            total, passed, failed, updated
        )
        .yellow()
    );

    Ok(())
}

/// Extract (url, hash) pairs from a manifest, checking arch-specific fields.
fn extract_url_hash_pairs(manifest: &serde_json::Value) -> Vec<(String, String)> {
    let mut result = Vec::new();

    // Try arch-specific first (64bit, 32bit, arm64)
    if let Some(arch) = manifest.get("architecture") {
        for key in &["64bit", "32bit", "arm64"] {
            if let Some(cfg) = arch.get(*key) {
                let urls = get_string_or_array(cfg, "url");
                let hashes = get_string_or_array(cfg, "hash");
                for (u, h) in urls.into_iter().zip(hashes) {
                    result.push((u, h));
                }
            }
        }
    }

    // Fall back to top-level url/hash if no arch-specific found
    if result.is_empty() {
        let urls = get_string_or_array(manifest, "url");
        let hashes = get_string_or_array(manifest, "hash");
        for (u, h) in urls.into_iter().zip(hashes) {
            result.push((u, h));
        }
    }

    result
}

/// Get a field as Vec<String>, handling both single string and array.
fn get_string_or_array(value: &serde_json::Value, key: &str) -> Vec<String> {
    match value.get(key) {
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        Some(serde_json::Value::Array(arr)) => {
            arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
        }
        _ => Vec::new(),
    }
}

/// Compute hash of a file using the specified algorithm.
fn compute_hash(path: &Path, algo: &str) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut builder = ChecksumBuilder::new().algo(algo)
        .map_err(|_| anyhow::anyhow!("unsupported hash algorithm: {}", algo))?;
    let mut hasher = builder.build();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.consume(&buf[..n]);
    }
    Ok(hasher.finalize())
}

/// Update hash value(s) in a manifest JSON, preserving the algorithm prefix.
fn update_manifest_hash(
    manifest: &mut serde_json::Value,
    actual_hash: &str,
    algo: &str,
) -> Result<()> {
    let prefixed = match algo {
        "md5" => format!("md5:{}", actual_hash),
        "sha1" => format!("sha1:{}", actual_hash),
        "sha512" => format!("sha512:{}", actual_hash),
        _ => actual_hash.to_string(), // sha256: prefix is optional
    };

    // Update arch-specific first, then top-level
    let mut updated = false;

    if let Some(arch) = manifest.get_mut("architecture") {
        for key in &["64bit", "32bit", "arm64"] {
            if let Some(cfg) = arch.get_mut(*key) {
                if set_hash_field(cfg, &prefixed) {
                    updated = true;
                }
            }
        }
    }

    if !updated {
        set_hash_field(manifest, &prefixed);
    }

    Ok(())
}

/// Set the `hash` field in a value node (handles both string and array).
fn set_hash_field(node: &mut serde_json::Value, hash: &str) -> bool {
    match node.get_mut("hash") {
        Some(serde_json::Value::String(s)) => {
            *s = hash.to_string();
            true
        }
        Some(serde_json::Value::Array(arr)) => {
            if let Some(first) = arr.get_mut(0) {
                if let Some(s) = first.as_str() {
                    // Keep the algorithm prefix from the original
                    let prefix = if s.starts_with("md5:") { "md5:" }
                        else if s.starts_with("sha1:") { "sha1:" }
                        else if s.starts_with("sha512:") { "sha512:" }
                        else { "" };
                    *first = serde_json::Value::String(format!("{}{}", prefix, hash));
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}
