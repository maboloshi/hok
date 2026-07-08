use clap::Parser;
use crossterm::style::Stylize;
use libscoop::operation;
use libscoop::{Manifest, Session};
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

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let dir = &args.dir;
    if !dir.is_dir() {
        eprintln!("error: '{}' is not a directory", dir.display());
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

        print!("{} ... ", name);
        total += 1;

        let url = urls[0].split('#').next().unwrap_or(urls[0]);
        let hash_str = &hashes[0];

        // Skip placeholders
        let raw_hash = hash_str.value();
        if raw_hash.is_empty() || raw_hash == "TODO" {
            println!("{}", "skipped (no hash)".yellow());
            continue;
        }

        // Download file
        let filename = url.split('/').last().unwrap_or("download");
        let cache_path = cache_dir.join(filename);

        if !cache_path.exists() || args.force {
            match operation::download_file(session, url, &cache_path) {
                Ok(()) => {}
                Err(e) => {
                    println!("{}: {}", "download failed".red(), e);
                    failed += 1;
                    continue;
                }
            }
        }

        // Compute hash
        let actual_hash = match compute_hash(&cache_path, hash_str.algorithm()) {
            Ok(h) => h,
            Err(e) => {
                println!("{}: {}", "hash error".red(), e);
                failed += 1;
                continue;
            }
        };

        if actual_hash == *raw_hash && !args.force {
            println!("{}", "ok".green());
            passed += 1;
            continue;
        }

        // Hash mismatch or force update
        if args.update || args.force {
            if args.force {
                println!("{} -> {}", "re-hashed".blue(), &actual_hash[..12]);
            } else {
                println!(
                    "{} {} -> {}",
                    "hash mismatch! updated".yellow(),
                    &raw_hash[..12],
                    &actual_hash[..12]
                );
            }
            update_json_hash(&path, hash_str.algorithm(), &actual_hash)?;
            updated += 1;
        } else {
            println!(
                "{} expected {} got {}",
                "hash mismatch".red(),
                &raw_hash[..12],
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

/// Compute hash of a file using specified algorithm name.
fn compute_hash(path: &Path, algo: &str) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let builder = ChecksumBuilder::new()
        .algo(algo)
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

/// Update hash value in a manifest JSON file, preserving algorithm prefix.
fn update_json_hash(path: &Path, algo: &str, actual_hash: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let mut manifest: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| anyhow::anyhow!("parse error: {}", e))?;

    let prefixed = match algo {
        "md5" => format!("md5:{}", actual_hash),
        "sha1" => format!("sha1:{}", actual_hash),
        "sha512" => format!("sha512:{}", actual_hash),
        _ => actual_hash.to_string(),
    };

    // Update arch-specific first, then top-level
    let mut updated = false;
    if let Some(arch) = manifest.get_mut("architecture") {
        for key in &["64bit", "32bit", "arm64"] {
            if let Some(cfg) = arch.get_mut(*key) {
                if set_hash_node(cfg, &prefixed) {
                    updated = true;
                }
            }
        }
    }
    if !updated {
        set_hash_node(&mut manifest, &prefixed);
    }

    let formatted = serde_json::to_string_pretty(&manifest)
        .map_err(|e| anyhow::anyhow!("serialize error: {}", e))?;
    std::fs::write(path, formatted.as_bytes())?;
    Ok(())
}

fn set_hash_node(node: &mut serde_json::Value, hash: &str) -> bool {
    match node.get_mut("hash") {
        Some(serde_json::Value::String(s)) => {
            *s = hash.to_string();
            true
        }
        Some(serde_json::Value::Array(arr)) => {
            if let Some(first) = arr.get_mut(0) {
                *first = serde_json::Value::String(hash.to_string());
                return true;
            }
            false
        }
        _ => false,
    }
}
