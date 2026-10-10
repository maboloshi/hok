use std::path::PathBuf;

use crate::package::{manifest, manifest_walker};
use crate::{Manifest, Session, error::Fallible, internal::url::url_remote_filename, network};

/// Options for a checkhashes run, filled by the CLI layer from its clap options.
///
/// Deliberately *not* a clap struct: the CLI owns argument parsing, this is
/// the plain domain options object (clap additions must be mapped here
/// explicitly, giving compile-time protection against the two drifting
/// apart).
#[derive(Debug, Clone)]
pub struct CheckhashesOptions {
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

/// One `(url, hash)` download entry of a manifest, in check order.
struct Entry {
    url: String,
    /// Manifest algorithm of the expected hash (`sha256` for an empty hash).
    algorithm: String,
    /// Expected digest as written in the manifest (lowercase, without prefix).
    /// Empty when the manifest carries `""` — official Scoop compares it like
    /// any other sha256 expectation, so it always mismatches and `--update`
    /// writes the real digest back.
    expected: String,
    /// JSON pointer of the `hash` field this entry is written back to.
    path: &'static str,
    /// Index inside the `hash` field at `path`.
    index: usize,
}

impl Entry {
    /// Expected hash formatted the way [`crate::internal::hash::format_hash_value`]
    /// formats a computed one (`sha256` has no prefix, everything else does).
    fn formatted_expected(&self) -> String {
        if self.expected.is_empty() {
            String::new()
        } else {
            crate::internal::hash::format_hash_value(&self.algorithm, &self.expected)
        }
    }
}

pub async fn execute(
    options: CheckhashesOptions,
    session: &Session,
    on_item: impl FnMut(&CheckHashesItem),
) -> Fallible<CheckHashesReport> {
    let mut on_item = on_item;
    let cache_dir = options
        .cache
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join("hok-checkhashes"));
    std::fs::create_dir_all(&cache_dir)?;

    let mut report = CheckHashesReport::default();
    let discovered = manifest_walker::discover_matching(&options.dir, &options.app)?;

    for (path, name) in &discovered {
        let manifest = match manifest::parse_file(path) {
            Ok(m) => m,
            Err(e) => {
                // Official Scoop parses manifests with `parse_json` and keeps
                // the file in the run; hok's typed parser rejects shapes it
                // cannot model (e.g. `"hash": "TODO"`, unsupported algorithms
                // and `{ "url": ..., "hash": ... }` hash objects). Never drop such
                // a manifest silently — report it and move on.
                report.total += 1;
                report.failed += 1;
                on_item(&CheckHashesItem {
                    name: name.clone(),
                    status: CheckHashesStatus::Failed,
                    messages: vec![
                        hok_i18n::t!("cmd.checkhashes_parse_failed", e = e.to_string()).to_string(),
                    ],
                });
                continue;
            }
        };
        // Not `Package::is_nightly()`: upstream's comparison here is
        // case-insensitive (`$manifest.version -eq 'nightly'` is `-ieq`,
        // `bin/checkhashes.ps1:68`) while `is_nightly` matches exactly. See
        // that method's note on the registered deviation.
        if manifest.version().eq_ignore_ascii_case("nightly") {
            continue;
        }

        let mut item = CheckHashesItem {
            name: name.clone(),
            status: CheckHashesStatus::Passed,
            messages: vec![],
        };
        report.total += 1;

        let entries = collect_entries(&manifest);
        if entries.is_empty() {
            // Mirrors `bin/checkhashes.ps1:84-87`.
            item.status = CheckHashesStatus::Failed;
            item.messages
                .push(hok_i18n::t!("cmd.checkhashes_no_url").to_string());
            report.failed += 1;
            on_item(&item);
            report.items.push(item);
            continue;
        }

        // Cached downloads are keyed by version so `--keep-cache` cannot make
        // a later version masquerade as already-checked (official Scoop does
        // the same through `<app>#<version>#…` cache keys for `-UseCache`).
        let run_cache_dir = if options.keep_cache {
            cache_dir.join(manifest.version())
        } else {
            cache_dir.clone()
        };

        let mut actual_hashes: Vec<String> = Vec::with_capacity(entries.len());
        let mut url_error = false;

        for (i, entry) in entries.iter().enumerate() {
            let url_stripped = entry.url.split('#').next().unwrap_or(&entry.url);
            let cache_path = run_cache_dir.join(cache_filename(name, &entries, i, &entry.url));

            if (!cache_path.exists() || options.force)
                && let Err(e) = network::download_file(session, url_stripped, &cache_path).await
            {
                item.messages
                    .push(hok_i18n::t!("cmd.checkhashes_download_failed", e = e).to_string());
                url_error = true;
                break;
            }

            match crate::internal::hash::compute_file_hash(&cache_path, &entry.algorithm) {
                Ok(h) => actual_hashes.push(crate::internal::hash::format_hash_value(
                    &entry.algorithm,
                    &h,
                )),
                Err(e) => {
                    item.messages
                        .push(hok_i18n::t!("cmd.checkhashes_hash_failed", e = e).to_string());
                    url_error = true;
                    break;
                }
            }
        }

        if url_error {
            item.status = CheckHashesStatus::Failed;
            report.failed += 1;
            on_item(&item);
            report.items.push(item);
            continue;
        }

        // Every entry is compared independently: a manifest whose hash is empty
        // (`""`) still yields a real mismatch for that entry instead of
        // aborting the whole manifest (official Scoop compares it too).
        let mismatches: Vec<usize> = (0..entries.len())
            .filter(|&i| actual_hashes[i] != entries[i].formatted_expected() || options.force)
            .collect();

        if mismatches.is_empty() {
            item.status = CheckHashesStatus::Passed;
            report.passed += 1;
            on_item(&item);
            report.items.push(item);
            continue;
        }

        if options.update || options.force {
            match update_manifest(path, &entries, &actual_hashes) {
                Ok(()) => {
                    item.status = CheckHashesStatus::Updated;
                    report.updated += 1;
                }
                Err(e) => {
                    item.status = CheckHashesStatus::Failed;
                    item.messages.push(e.to_string());
                    report.failed += 1;
                }
            }
        } else {
            for &i in &mismatches {
                let entry = &entries[i];
                let cache_path = run_cache_dir.join(cache_filename(name, &entries, i, &entry.url));
                item.messages.push(
                    hok_i18n::t!(
                        "cmd.checkhashes_mismatch",
                        expected = entry.formatted_expected(),
                        actual = actual_hashes[i],
                        url = entry.url,
                        first_bytes = magic_bytes_of(&cache_path)
                    )
                    .to_string(),
                );
            }
            item.status = CheckHashesStatus::Failed;
            report.failed += 1;
        }
        on_item(&item);
        report.items.push(item);
    }

    if !options.keep_cache {
        let _ = std::fs::remove_dir_all(&cache_dir);
    }
    Ok(report)
}

/// Cache file name for entry `i`, shared by the download path and the
/// mismatch report so both always agree.
fn cache_filename(name: &str, entries: &[Entry], i: usize, url: &str) -> String {
    format!(
        "{}-{}-{}",
        name,
        hash_entry_index_prefix(entries, i),
        url_remote_filename(url)
    )
}

/// Collect the download entries of a manifest in check order.
///
/// Mirrors official Scoop's either/or selection: a top-level `url` wins over
/// the `architecture` section; otherwise the architecture entries are walked
/// 64bit → 32bit → arm64 (see [`Manifest::checkhashes_accessors`]).
fn collect_entries(manifest: &Manifest) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut index_by_path: std::collections::HashMap<&'static str, usize> =
        std::collections::HashMap::new();
    for (url, hash, path) in manifest.checkhashes_accessors() {
        let index = index_by_path.entry(path).or_insert(0);
        entries.push(Entry {
            url: url.to_owned(),
            algorithm: hash.algorithm().to_owned(),
            expected: hash.value().to_lowercase(),
            path,
            index: *index,
        });
        *index += 1;
    }
    entries
}

/// Write the computed hashes back into the manifest, preserving the original
/// JSON shape of each `hash` field.
fn update_manifest(
    path: &std::path::Path,
    entries: &[Entry],
    actual_hashes: &[String],
) -> Fallible<()> {
    let content = std::fs::read_to_string(path)?;
    // Tolerant read: upstream `bin/checkhashes.ps1:65` reads the manifest
    // through `parse_json`, so a BOM/JSON5 manifest that passed the hash check
    // must also parse for the rewrite.
    let mut root: serde_json::Value = crate::internal::json::parse_str(&content)?;
    let mut path_map: std::collections::HashMap<&'static str, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, entry) in entries.iter().enumerate() {
        path_map.entry(entry.path).or_default().push(i);
    }

    let mut missing = Vec::new();
    for (path_prefix, indices) in path_map {
        let new_hashes: Vec<String> = indices
            .iter()
            .map(|&idx| actual_hashes[idx].clone())
            .collect();
        if let Some(obj) = root.pointer_mut(path_prefix) {
            let existing = obj.clone();
            *obj = manifest::update_hash_preserving_style(&existing, &new_hashes, None);
        } else {
            missing.push(path_prefix);
        }
    }
    if !missing.is_empty() {
        return Err(crate::Error::Custom(
            hok_i18n::t!("cmd.checkhashes_path_not_found", path = missing.join(", ")).to_string(),
        ));
    }
    crate::internal::fs::write_json(path, &root)?;
    Ok(())
}

/// First bytes of a file formatted like Scoop's `get_magic_bytes_pretty`
/// (uppercase hex, space separated, at most 8 bytes). Empty when the file is
/// gone (e.g. the download was skipped).
fn magic_bytes_of(path: &std::path::Path) -> String {
    let Ok(data) = std::fs::read(path) else {
        return String::new();
    };
    data.iter()
        .take(8)
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

fn hash_entry_index_prefix(entries: &[Entry], i: usize) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── entry fixtures ──────────────────────────────────────────────────────

    fn entry(path: &'static str, index: usize) -> Entry {
        Entry {
            url: "http://example.com/x.zip".to_owned(),
            algorithm: "sha256".to_owned(),
            expected: String::new(),
            path,
            index,
        }
    }

    // ── hash_entry_index_prefix ─────────────────────────────────────────────

    #[test]
    fn index_prefix_out_of_bounds_returns_index_string() {
        let entries: Vec<Entry> = vec![];
        assert_eq!(hash_entry_index_prefix(&entries, 0), "0");
        assert_eq!(hash_entry_index_prefix(&entries, 5), "5");
    }

    #[test]
    fn index_prefix_top_level_hash_returns_index() {
        let entries = vec![entry("/hash", 0)];
        assert_eq!(hash_entry_index_prefix(&entries, 0), "0");
    }

    #[test]
    fn index_prefix_arch_path_includes_arch_hint() {
        let entries = vec![entry("/architecture/64bit/hash", 0)];
        let prefix = hash_entry_index_prefix(&entries, 0);
        assert!(
            prefix.contains("64bit"),
            "prefix should contain arch hint: {}",
            prefix
        );
    }

    // ── Entry::formatted_expected ───────────────────────────────────────────

    #[test]
    fn empty_expected_formats_empty_never_prefixed() {
        let e = Entry {
            algorithm: "sha256".to_owned(),
            expected: String::new(),
            ..entry("/hash", 0)
        };
        assert_eq!(e.formatted_expected(), "");
    }

    /// An empty expected hash must never compare equal to a real digest: that
    /// is what turns `"hash": ""` into a mismatch (`-u` then writes the digest
    /// back) instead of the old "hash skipped" abort.
    #[test]
    fn empty_expected_never_equals_a_real_digest() {
        let e = Entry {
            algorithm: "sha256".to_owned(),
            expected: String::new(),
            ..entry("/hash", 0)
        };
        let real = "ab".repeat(32);
        assert_ne!(e.formatted_expected(), real);
    }

    #[test]
    fn sha256_expected_has_no_prefix() {
        let digest = "ab".repeat(32);
        let e = Entry {
            algorithm: "sha256".to_owned(),
            expected: digest.clone(),
            ..entry("/hash", 0)
        };
        assert_eq!(e.formatted_expected(), digest);
    }

    #[test]
    fn non_sha256_expected_keeps_prefix() {
        let digest = "ab".repeat(20);
        let e = Entry {
            algorithm: "sha1".to_owned(),
            expected: digest.clone(),
            ..entry("/hash", 0)
        };
        assert_eq!(e.formatted_expected(), format!("sha1:{digest}"));
    }

    // ── CheckHashesReport default ───────────────────────────────────────────

    #[test]
    fn report_default_is_zeroed() {
        let r = CheckHashesReport::default();
        assert_eq!(r.total, 0);
        assert_eq!(r.passed, 0);
        assert_eq!(r.failed, 0);
        assert_eq!(r.updated, 0);
        assert!(r.items.is_empty());
    }

    // ── CheckHashesStatus equality ──────────────────────────────────────────

    #[test]
    fn status_equality() {
        assert_eq!(CheckHashesStatus::Passed, CheckHashesStatus::Passed);
        assert_ne!(CheckHashesStatus::Passed, CheckHashesStatus::Failed);
        assert_ne!(CheckHashesStatus::Failed, CheckHashesStatus::Updated);
    }

    // ── collect_entries ─────────────────────────────────────────────────────

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
            assert!(collect_entries(&m).is_empty());
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
            let entries = collect_entries(&m);
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].path, "/hash");
            assert_eq!(entries[0].index, 0);
        }
    }

    /// A top-level `url` wins: the architecture section is not checked
    /// (official `bin/checkhashes.ps1:73-83`).
    #[test]
    fn collect_entries_prefers_top_level_url_over_architecture() {
        let m = manifest_from_json(
            r#"{
            "version": "1.0",
            "url": "https://example.com/top.zip",
            "hash": "aa",
            "architecture": {
                "64bit": { "url": "https://example.com/64.zip", "hash": "bb" }
            }
        }"#,
        );
        if let Some(m) = m {
            let entries = collect_entries(&m);
            assert_eq!(entries.len(), 1, "architecture must be ignored");
            assert_eq!(entries[0].url, "https://example.com/top.zip");
            assert_eq!(entries[0].path, "/hash");
        }
    }

    #[test]
    fn collect_entries_arch_order_is_64_32_arm64() {
        let m = manifest_from_json(
            r#"{
            "version": "1.0",
            "architecture": {
                "arm64": { "url": "https://example.com/arm.zip", "hash": "cc" },
                "32bit": { "url": "https://example.com/32.zip", "hash": "bb" },
                "64bit": { "url": "https://example.com/64.zip", "hash": "aa" }
            }
        }"#,
        );
        if let Some(m) = m {
            let entries = collect_entries(&m);
            let urls: Vec<&str> = entries.iter().map(|e| e.url.as_str()).collect();
            assert_eq!(
                urls,
                vec![
                    "https://example.com/64.zip",
                    "https://example.com/32.zip",
                    "https://example.com/arm.zip"
                ]
            );
            assert_eq!(entries[0].path, "/architecture/64bit/hash");
            assert_eq!(entries[2].path, "/architecture/arm64/hash");
        }
    }

    #[test]
    fn collect_entries_arch_hash_indices_are_per_segment() {
        let m = manifest_from_json(
            r#"{
            "version": "1.0",
            "architecture": {
                "64bit": {
                    "url": ["https://example.com/a.zip", "https://example.com/b.zip"],
                    "hash": ["aa", "bb"]
                },
                "32bit": { "url": "https://example.com/c.zip", "hash": "cc" }
            }
        }"#,
        );
        if let Some(m) = m {
            let entries = collect_entries(&m);
            assert_eq!(entries.len(), 3);
            assert_eq!(entries[0].index, 0);
            assert_eq!(entries[1].index, 1);
            // The 32bit segment restarts its own index.
            assert_eq!(entries[2].index, 0);
            assert_eq!(entries[2].path, "/architecture/32bit/hash");
        }
    }

    #[test]
    fn collect_entries_empty_hash_is_a_normal_entry() {
        let m = manifest_from_json(
            r#"{
            "version": "1.0",
            "url": "https://example.com/app.zip",
            "hash": ""
        }"#,
        );
        if let Some(m) = m {
            let entries = collect_entries(&m);
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].algorithm, "sha256");
            assert_eq!(entries[0].expected, "");
            assert_eq!(entries[0].formatted_expected(), "");
        }
    }
}
