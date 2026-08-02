use std::path::PathBuf;

use crate::package::manifest_walker;
use crate::{error::Fallible, network, Manifest, Session};

#[derive(Debug, Clone)]
pub struct CheckHashesOptions {
    pub dir: PathBuf,
    pub app: Vec<String>,
    pub update: bool,
    pub force: bool,
    pub skip_correct: bool,
    pub keep_cache: bool,
    pub cache: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct CheckHashesReport {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub updated: u32,
    pub skipped: u32,
    pub items: Vec<CheckHashesItem>,
}

#[derive(Debug, Clone)]
pub struct CheckHashesItem {
    pub name: String,
    pub status: CheckHashesStatus,
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckHashesStatus {
    Passed,
    Failed,
    Updated,
}

struct HashEntry {
    path: String,
    index: usize,
}

pub fn check_hashes(session: &Session, opts: &CheckHashesOptions) -> Fallible<CheckHashesReport> {
    let cache_dir = opts
        .cache
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join("hok-checkhashes"));
    std::fs::create_dir_all(&cache_dir)?;

    let mut report = CheckHashesReport::default();
    let json_files = manifest_walker::discover(&opts.dir)?;

    for path in &json_files {
        let name = match path.file_stem() {
            Some(s) => s.to_string_lossy().to_string(),
            None => continue,
        };
        if opts.app.first().map(|s| s.as_str()) != Some("*")
            && !opts.app.iter().any(|p| name.contains(p))
        {
            continue;
        }

        let manifest = match Manifest::parse(path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if manifest.version() == "nightly" {
            continue;
        }

        let mut item = CheckHashesItem {
            name: name.clone(),
            status: CheckHashesStatus::Passed,
            messages: vec![],
        };
        report.total += 1;

        let Some((urls, entries)) = collect_entries(&manifest) else {
            let has_urls = !manifest.all_urls().is_empty();
            let has_hashes = !manifest.all_hashes().is_empty();
            if has_urls || has_hashes {
                item.status = CheckHashesStatus::Failed;
                item.messages
                    .push("URL and hash count mismatch".to_string());
                report.failed += 1;
                report.items.push(item);
            }
            continue;
        };

        let mut actual_hashes = Vec::with_capacity(urls.len());
        let mut has_any_failure = false;

        for (i, url_str) in urls.iter().enumerate() {
            let hash_str = manifest.all_hashes()[i];
            let raw_hash = hash_str.value();
            if raw_hash.is_empty() || raw_hash == "TODO" {
                item.status = CheckHashesStatus::Failed;
                item.messages.push("hash skipped (empty/TODO)".to_string());
                report.skipped += 1;
                has_any_failure = true;
                break;
            }

            let url = url_str.split('#').next().unwrap_or(url_str);
            let filename = url.rsplit('/').next().unwrap_or("download");
            let cache_filename = format!(
                "{}-{}-{}",
                name,
                hash_entry_index_prefix(&entries, i),
                filename
            );
            let cache_path = cache_dir.join(&cache_filename);

            if !cache_path.exists() || opts.force {
                if let Err(e) = network::download_file(session, url, &cache_path) {
                    item.status = CheckHashesStatus::Failed;
                    item.messages.push(format!("download failed: {}", e));
                    has_any_failure = true;
                    break;
                }
            }

            let actual_hash = match crate::internal::hash::compute_file_hash(&cache_path, hash_str.algorithm())
            {
                Ok(h) => h,
                Err(e) => {
                    item.status = CheckHashesStatus::Failed;
                    item.messages.push(format!("hash failed: {}", e));
                    has_any_failure = true;
                    break;
                }
            };
            actual_hashes.push(format_hash_value(hash_str.algorithm(), &actual_hash));
        }

        if has_any_failure {
            report.failed += 1;
            report.items.push(item);
            continue;
        }

        let mut mismatches: Vec<(usize, &HashEntry)> = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            let expected = manifest.all_hashes()[i];
            let formatted_expected = format_hash_value(expected.algorithm(), expected.value());
            let actual = &actual_hashes[i];
            if actual != &formatted_expected || opts.force {
                mismatches.push((i, entry));
            }
        }

        if mismatches.is_empty() {
            item.status = CheckHashesStatus::Passed;
            report.passed += 1;
            report.items.push(item);
            continue;
        }

        if opts.update || opts.force {
            let content = std::fs::read_to_string(path)?;
            let mut root: serde_json::Value = serde_json::from_str(&content)?;
            let mut path_map: std::collections::HashMap<&str, Vec<usize>> =
                std::collections::HashMap::new();
            for (i, entry) in entries.iter().enumerate() {
                path_map.entry(&entry.path).or_default().push(i);
            }

            for (path_prefix, indices) in path_map {
                let new_hashes: Vec<String> = indices
                    .iter()
                    .map(|&idx| actual_hashes[idx].clone())
                    .collect();
                let new_val = if new_hashes.len() == 1 {
                    serde_json::Value::String(new_hashes[0].clone())
                } else {
                    serde_json::Value::Array(
                        new_hashes
                            .into_iter()
                            .map(serde_json::Value::String)
                            .collect(),
                    )
                };
                if let Some(obj) = root.pointer_mut(path_prefix) {
                    *obj = new_val;
                } else {
                    item.status = CheckHashesStatus::Failed;
                    item.messages
                        .push(format!("path {} not found in JSON", path_prefix));
                }
            }
            crate::internal::fs::write_json(path, &root)?;
            item.status = CheckHashesStatus::Updated;
            report.updated += 1;
        } else {
            for (i, _entry) in &mismatches {
                let expected = manifest.all_hashes()[*i];
                let expected_str = format_hash_value(expected.algorithm(), expected.value());
                let actual = &actual_hashes[*i];
                item.messages.push(format!(
                    "mismatch expected={} actual={} url={}",
                    &expected_str[..std::cmp::min(12, expected_str.len())],
                    &actual[..std::cmp::min(12, actual.len())],
                    urls.get(*i).cloned().unwrap_or_default()
                ));
            }
            item.status = CheckHashesStatus::Failed;
            report.failed += 1;
        }
        report.items.push(item);
    }

    if !opts.keep_cache {
        let _ = std::fs::remove_dir_all(&cache_dir);
    }
    Ok(report)
}

fn collect_entries(manifest: &Manifest) -> Option<(Vec<String>, Vec<HashEntry>)> {
    let urls = manifest.all_urls();
    let hashes = manifest.all_hashes();
    if urls.is_empty() || hashes.is_empty() || urls.len() != hashes.len() {
        return None;
    }
    let segments = manifest.all_hash_segments();
    let mut entries = Vec::new();
    for (path, count) in &segments {
        for i in 0..*count {
            entries.push(HashEntry {
                path: path.clone(),
                index: i,
            });
        }
    }
    if entries.len() != urls.len() {
        return None;
    }
    Some((urls.into_iter().map(|s| s.to_string()).collect(), entries))
}
fn hash_entry_index_prefix(entries: &[HashEntry], i: usize) -> String {
    if i < entries.len() {
        let entry = &entries[i];
        let arch_hint = entry
            .path
            .trim_start_matches("/architecture/")
            .trim_end_matches("/hash")
            .replace('/', "-");
        if arch_hint.is_empty() || arch_hint == "hash" {
            entry.index.to_string()
        } else {
            format!("{}-{}", arch_hint, entry.index)
        }
    } else {
        i.to_string()
    }
}

fn format_hash_value(algo: &str, hash: &str) -> String {
    match algo {
        "md5" => format!("md5:{hash}"),
        "sha1" => format!("sha1:{hash}"),
        "sha512" => format!("sha512:{hash}"),
        _ => hash.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── format_hash_value ───────────────────────────────────────────────────

    #[test]
    fn format_sha256_returns_bare_value() {
        assert_eq!(format_hash_value("sha256", "abc123"), "abc123");
    }

    #[test]
    fn format_md5_adds_prefix() {
        assert_eq!(format_hash_value("md5", "deadbeef"), "md5:deadbeef");
    }

    #[test]
    fn format_sha1_adds_prefix() {
        assert_eq!(format_hash_value("sha1", "aabbcc"), "sha1:aabbcc");
    }

    #[test]
    fn format_sha512_adds_prefix() {
        assert_eq!(format_hash_value("sha512", "longvalue"), "sha512:longvalue");
    }

    #[test]
    fn format_unknown_algo_returns_bare() {
        assert_eq!(format_hash_value("crc32", "deadcode"), "deadcode");
    }

    // ── hash_entry_index_prefix ─────────────────────────────────────────────

    #[test]
    fn index_prefix_out_of_bounds_returns_index_string() {
        let entries: Vec<HashEntry> = vec![];
        assert_eq!(hash_entry_index_prefix(&entries, 0), "0");
        assert_eq!(hash_entry_index_prefix(&entries, 5), "5");
    }

    #[test]
    fn index_prefix_top_level_hash_returns_index() {
        let entries = vec![HashEntry {
            path: "/hash".to_string(),
            index: 0,
        }];
        assert_eq!(hash_entry_index_prefix(&entries, 0), "0");
    }

    #[test]
    fn index_prefix_arch_path_includes_arch_hint() {
        let entries = vec![HashEntry {
            path: "/architecture/64bit/hash".to_string(),
            index: 0,
        }];
        let prefix = hash_entry_index_prefix(&entries, 0);
        assert!(
            prefix.contains("64bit"),
            "prefix should contain arch hint: {}",
            prefix
        );
    }

    // ── CheckHashesReport default ───────────────────────────────────────────

    #[test]
    fn report_default_is_zeroed() {
        let r = CheckHashesReport::default();
        assert_eq!(r.total, 0);
        assert_eq!(r.passed, 0);
        assert_eq!(r.failed, 0);
        assert_eq!(r.updated, 0);
        assert_eq!(r.skipped, 0);
        assert!(r.items.is_empty());
    }

    // ── CheckHashesStatus equality ──────────────────────────────────────────

    #[test]
    fn status_equality() {
        assert_eq!(CheckHashesStatus::Passed, CheckHashesStatus::Passed);
        assert_ne!(CheckHashesStatus::Passed, CheckHashesStatus::Failed);
        assert_ne!(CheckHashesStatus::Failed, CheckHashesStatus::Updated);
    }

    // ── collect_entries on real manifests ───────────────────────────────────

    /// Helper: build a Manifest from an inline JSON string.
    fn manifest_from_json(json: &str) -> Option<crate::Manifest> {
        crate::Manifest::from_json("test-pkg", json).ok()
    }

    #[test]
    fn collect_entries_empty_when_no_urls() {
        let m = manifest_from_json(
            r#"{
            "version": "1.0",
            "homepage": "https://example.com",
            "license": "MIT"
        }"#,
        );
        if let Some(m) = m {
            assert!(collect_entries(&m).is_none());
        }
    }

    #[test]
    fn collect_entries_mismatch_returns_none() {
        // One URL but two hashes — should return None.
        let m = manifest_from_json(
            r#"{
            "version": "1.0",
            "homepage": "https://example.com",
            "license": "MIT",
            "url": "https://example.com/app.zip",
            "hash": [
                "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
                "b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3"
            ]
        }"#,
        );
        // Manifest parser may reject mismatched counts; either way collect_entries
        // should return None for mismatch.
        if let Some(m) = m {
            let result = collect_entries(&m);
            assert!(
                result.is_none(),
                "URL/hash count mismatch should yield None"
            );
        }
    }

    #[test]
    fn collect_entries_matching_url_hash() {
        let m = manifest_from_json(
            r#"{
            "version": "1.0",
            "homepage": "https://example.com",
            "license": "MIT",
            "url": "https://example.com/app.zip",
            "hash": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"
        }"#,
        );
        if let Some(m) = m {
            let result = collect_entries(&m);
            assert!(result.is_some(), "matching URL/hash should yield Some");
            let (urls, entries) = result.unwrap();
            assert_eq!(urls.len(), 1);
            assert_eq!(entries.len(), 1);
        }
    }
}
