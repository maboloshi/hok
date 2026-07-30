use clap::Parser;
use libscoop::operation;
use libscoop::{Manifest, Session};
use std::path::{Path, PathBuf};

use crate::{output, Result};

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

    /// Suppress output for manifests with correct hashes
    #[arg(short = 's', long = "skip-correct", action = clap::ArgAction::SetTrue)]
    skip_correct: bool,

    /// Keep downloaded files in cache after check completes
    #[arg(short = 'k', long = "keep-cache", action = clap::ArgAction::SetTrue)]
    keep_cache: bool,

    /// Use cache directory for downloaded files
    #[arg(short = 'c', long)]
    cache: Option<PathBuf>,
}

/// Origin information for a single URL/hash entry in the manifest.
struct HashEntry {
    /// JSON pointer to the hash array this entry belongs to.
    /// e.g. "/hash" for top-level, "/architecture/64bit/hash" for 64bit.
    path: String,
    /// Index within the hash array at that path.
    index: usize,
}

/// Collect URL/hash pairs from a manifest in union order (top-level + all archs),
/// tracking the origin of each entry for later update.
fn collect_entries(manifest: &Manifest) -> Option<(Vec<String>, Vec<HashEntry>)> {
    let urls = manifest.all_urls();
    let hashes = manifest.all_hashes();

    if urls.is_empty() || hashes.is_empty() {
        return None;
    }
    if urls.len() != hashes.len() {
        return None; // count mismatch — skip
    }

    let segments = manifest.all_hash_segments();
    let mut entries: Vec<HashEntry> = Vec::new();
    for (path, count) in &segments {
        for i in 0..*count {
            entries.push(HashEntry {
                path: path.clone(),
                index: i,
            });
        }
    }

    // Safety check: entries count must match urls count
    if entries.len() != urls.len() {
        return None;
    }

    Some((
        urls.into_iter().map(|s| s.to_string()).collect(),
        entries,
    ))
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

    let mut total = 0u32;
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut updated = 0u32;
    let mut _skipped = 0u32;

    // Recursively collect all JSON files
    let mut json_files: Vec<PathBuf> = Vec::new();
    collect_json_files(dir, &mut json_files)?;

    for path in &json_files {
        let name = match path.file_stem() {
            Some(s) => s.to_string_lossy().to_string(),
            None => continue,
        };

        if args.app[0] != "*" && !args.app.iter().any(|p| name.contains(p.as_str())) {
            continue;
        }

        let manifest = match Manifest::parse(path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        // Skip nightly manifests (their hash validation is skipped in Scoop)
        if manifest.version() == "nightly" {
            continue;
        }

        // Collect all URL/hash pairs with origin tracking
        let Some((urls, entries)) = collect_entries(&manifest) else {
            // Has URLs or hashes but count mismatch or empty
            let has_urls = !manifest.all_urls().is_empty();
            let has_hashes = !manifest.all_hashes().is_empty();
            if has_urls || has_hashes {
                if has_urls != has_hashes || has_urls {
                    output::err(format!("{name}: URL and hash count mismatch"));
                }
                total += 1;
                failed += 1;
            }
            continue;
        };

        print!("{name} ... ");
        total += 1;

        // Compute actual hashes for each URL
        let mut actual_hashes: Vec<String> = Vec::with_capacity(urls.len());
        let mut has_any_failure = false;

        for (i, url_str) in urls.iter().enumerate() {
            let hash_str = manifest.all_hashes()[i];

            // Skip placeholders
            let raw_hash = hash_str.value();
            if raw_hash.is_empty() || raw_hash == "TODO" {
                output::warn(rust_i18n::t!("cmd.checkhashes_skipped"));
                _skipped += 1;
                has_any_failure = true;
                break;
            }

            // Extract clean URL (strip fragment)
            let url = url_str.split('#').next().unwrap_or(url_str);

            // Download file to app-specific cache path
            let filename = url.split('/').last().unwrap_or("download");
            let cache_filename = format!("{}-{}-{}", name, hash_entry_index_prefix(&entries, i), filename);
            let cache_path = cache_dir.join(&cache_filename);

            if !cache_path.exists() || args.force {
                match operation::download_file(session, url, &cache_path) {
                    Ok(()) => {}
                    Err(e) => {
                        output::err(rust_i18n::t!("cmd.err_download", e = e));
                        has_any_failure = true;
                        break;
                    }
                }
            }

            // Compute hash
            let actual_hash = match scoop_hash::compute_file_hash(&cache_path, hash_str.algorithm()) {
                Ok(h) => h,
                Err(e) => {
                    output::err(rust_i18n::t!("cmd.err_hash", e = e));
                    has_any_failure = true;
                    break;
                }
            };

            // Format with algorithm prefix (Scoop convention: sha256 bare, others prefixed)
            let formatted = format_hash_value(hash_str.algorithm(), &actual_hash);
            actual_hashes.push(formatted);
        }

        if has_any_failure {
            continue;
        }

        // Compare each computed hash against expected
        let mut mismatches: Vec<(usize, &HashEntry)> = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            let expected = manifest.all_hashes()[i];
            let formatted_expected = format_hash_value(expected.algorithm(), expected.value());
            let actual = &actual_hashes[i];
            if actual != &formatted_expected || args.force {
                mismatches.push((i, entry));
            }
        }

        if mismatches.is_empty() {
            if !args.skip_correct {
                output::ok();
            }
            passed += 1;
            continue;
        }

        // ── Hash mismatch or force update ────────────────────────────────
        if args.update || args.force {
            if mismatches.is_empty() && !args.force {
                if !args.skip_correct {
                    output::ok();
                }
                passed += 1;
                continue;
            }

            // ── Need to update hashes ──────────────────────────────────────────────
            let content = std::fs::read_to_string(path)?;
            let mut root: serde_json::Value = serde_json::from_str(&content)?;

            // Group entries by path
            let mut path_map: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
            for (i, entry) in entries.iter().enumerate() {
                path_map.entry(&entry.path).or_insert_with(Vec::new).push(i);
            }

            // Update each path's hash array/value
            for (path_prefix, indices) in path_map {
                let new_hashes: Vec<String> = indices.iter().map(|&idx| actual_hashes[idx].clone()).collect();
                let new_val = if new_hashes.len() == 1 {
                    serde_json::Value::String(new_hashes[0].clone())
                } else {
                    serde_json::Value::Array(new_hashes.into_iter().map(serde_json::Value::String).collect())
                };

                if let Some(obj) = root.pointer_mut(path_prefix) {
                    *obj = new_val;
                } else {
                    // Should not happen if entries are consistent with manifest
                    output::err(format!("{name}: path {} not found in JSON", path_prefix));
                    failed += 1;
                    continue;
                }
            }

            // Write updated JSON
            libscoop::internal::fs::write_json(path, &root)?;

            // Report changes
            if !args.force {
                // mismatch updates
                for (i, _entry) in &mismatches {
                    let actual = &actual_hashes[*i];
                    output::change(
                        rust_i18n::t!("cmd.checkhashes_mismatch_upd"),
                        "->",
                        &actual[..std::cmp::min(12, actual.len())],
                    );
                }
            } else {
                // force rehash
                for (i, _entry) in mismatches.iter().copied().chain(
                    // Also include entries that were correct but force updated
                    if args.force {
                        (0..entries.len())
                            .filter(|i| !mismatches.iter().any(|(idx, _)| idx == i))
                            .map(|i| (i, &entries[i]))
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    }
                ) {
                    let actual = &actual_hashes[i];
                    output::change(
                        rust_i18n::t!("cmd.checkhashes_rehashed"),
                        "->",
                        &actual[..std::cmp::min(12, actual.len())],
                    );
                }
            }
            updated += 1;
        } else {
            // Report all mismatches with details
            for (i, _entry) in &mismatches {
                let expected = manifest.all_hashes()[*i];
                let expected_str = format_hash_value(expected.algorithm(), expected.value());
                let actual = &actual_hashes[*i];
                let truncated_exp = &expected_str[..std::cmp::min(12, expected_str.len())];
                let truncated_act = &actual[..std::cmp::min(12, actual.len())];
                output::err(rust_i18n::t!(
                    "cmd.checkhashes_mismatch",
                    expected = truncated_exp,
                    actual = truncated_act
                ));
                // Print URL for context
                if *i < urls.len() {
                    eprintln!("       URL: {}", urls[*i]);
                }
            }
            failed += 1;
        }
    }

    // Cleanup cache unless --keep-cache
    if !args.keep_cache {
        let _ = std::fs::remove_dir_all(&cache_dir);
    }

    output::info(rust_i18n::t!(
        "cmd.checkhashes_summary",
        total = total,
        passed = passed,
        failed = failed,
        updated = updated
    ));

    Ok(())
}

/// Recursively collect all `.json` files under `dir`.
fn collect_json_files(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, files)?;
        } else if path.extension().map(|e| e == "json").unwrap_or(false) {
            files.push(path);
        }
    }
    Ok(())
}

/// Build a short prefix string for cache filenames from a hash entry.
fn hash_entry_index_prefix(entries: &[HashEntry], i: usize) -> String {
    if i < entries.len() {
        // Use a simplified path indicator
        let entry = &entries[i];
        let arch_hint = entry.path.trim_start_matches("/architecture/")
            .trim_end_matches("/hash")
            .replace('/', "-");
        if arch_hint.is_empty() || arch_hint == "hash" {
            format!("{}", entry.index)
        } else {
            format!("{}-{}", arch_hint, entry.index)
        }
    } else {
        i.to_string()
    }
}

/// Format a hash value with algorithm prefix, matching Scoop conventions:
/// sha256 → bare hex, others (md5/sha1/sha512) → "algo:hex"
fn format_hash_value(algo: &str, hash: &str) -> String {
    match algo {
        "md5" => format!("md5:{hash}"),
        "sha1" => format!("sha1:{hash}"),
        "sha512" => format!("sha512:{hash}"),
        _ => hash.to_string(), // sha256 is bare
    }
}
