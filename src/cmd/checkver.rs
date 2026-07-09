use clap::Parser;
use crossterm::style::Stylize;
use libscoop::{operation, Manifest, Session};
use regex::Regex;
use scoop_hash::ChecksumBuilder;
use std::io::Read;
use std::path::PathBuf;

use crate::Result;

/// Check manifest for a newer version
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = ".")]
    dir: PathBuf,

    /// Specific app(s) to check (supports wildcards, default: all)
    #[arg(default_value = "*")]
    app: Vec<String>,

    /// Update manifest with new version and trigger autoupdate
    #[arg(short = 'u', long, action = clap::ArgAction::SetTrue)]
    update: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let dir = &args.dir;
    if !dir.is_dir() {
        eprintln!("error: '{}' is not a directory", dir.display());
        return Ok(());
    }

    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }

        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        if args.app[0] != "*" && !args.app.iter().any(|p| stem.contains(p.as_str())) {
            continue;
        }

        let manifest = match Manifest::parse(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let cv = match manifest.checkver() {
            Some(c) => c,
            None => continue,
        };

        print!("{} ... ", stem);

        // Determine URL and regex to use
        let url = match &cv.url {
            Some(u) => u.clone(),
            None if cv.sourceforge.is_some() => {
                println!("{}", "sourceforge checkver not supported".yellow());
                continue;
            }
            None if is_github_checkver(&cv) => {
                match github_api_url(manifest.homepage()) {
                    Some(api_url) => api_url,
                    None => {
                        println!("{}", "could not extract GitHub repo from homepage".yellow());
                        continue;
                    }
                }
            }
            None => {
                println!("{}", "no checkver url".yellow());
                continue;
            }
        };

        // Automatically add `$.tag_name` JSONPath for GitHub API responses
        let mut effective_jsonpath = cv.jsonpath.clone();
        if effective_jsonpath.is_none() && url.contains("api.github.com") {
            effective_jsonpath = Some("$.tag_name".to_string());
        }

        // Fetch page content
        let raw = match operation::download_page(session, &url) {
            Ok(t) => t,
            Err(e) => {
                println!("{}: {}", "fetch error".red(), e);
                continue;
            }
        };

        // Extract version
        let current = manifest.version().to_string();
        let extract_result = extract_version(&raw, cv, effective_jsonpath.as_deref());

        match extract_result {
            Some((ref ver, ref captures)) if ver == &current => {
                println!("{} ({})", "up to date".green(), ver);
            }
            Some((ref ver, ref captures)) => {
                println!("{} {} -> {}", "update available".yellow(), current, ver.as_str().blue());
                if args.update {
                    match apply_autoupdate(session, &path, &manifest, ver, captures) {
                        Ok(()) => println!("  {} updated to {}", "✓".green(), ver),
                        Err(e) => println!("  {}: {}", "update failed".red(), e),
                    }
                }
            }
            None => {
                println!("{}", "could not extract version".red());
            }
        }
    }

    Ok(())
}

fn is_github_checkver(cv: &libscoop::Checkver) -> bool {
    cv.regex.as_deref().map_or(false, |r| r.contains("/releases/tag/"))
}

fn github_api_url(homepage: &str) -> Option<String> {
    let re = Regex::new(r"github\.com[:/]([^/]+/[^/]+?)(?:/|$)").ok()?;
    let caps = re.captures(homepage)?;
    let repo = caps.get(1)?.as_str().trim_end_matches('/');
    Some(format!("https://api.github.com/repos/{}/releases/latest", repo))
}

/// Extract version + capture groups from page content.
fn extract_version(content: &str, cv: &libscoop::Checkver, jsonpath_override: Option<&str>) -> Option<(String, Vec<String>)> {
    if let Some(jp) = jsonpath_override.or(cv.jsonpath.as_deref()) {
        use jsonpath_rust::JsonPath;
        let value: serde_json::Value = serde_json::from_str(content).ok()?;
        let found = value.query(jp).ok()?;
        let ver = found.first()?.as_str()?;
        if !ver.is_empty() {
            return Some((ver.to_string(), vec![ver.to_string()]));
        }
    }

    if let Some(regex_str) = &cv.regex {
        let re = Regex::new(regex_str).ok()?;
        let caps = re.captures(content)?;
        let ver = caps.get(1).or_else(|| caps.get(0))?.as_str().to_string();
        let captures: Vec<String> = caps.iter()
            .map(|m| m.map(|s| s.as_str().to_string()).unwrap_or_default())
            .collect();
        return Some((ver, captures));
    }

    let trimmed = content.trim();
    if !trimmed.is_empty() {
        Some((trimmed.to_string(), vec![trimmed.to_string()]))
    } else {
        None
    }
}

// ─── Autoupdate ────────────────────────────────────────────────────────────

/// Apply autoupdate: substitute variables, download files, compute/extract
/// hashes, write updated manifest.
fn apply_autoupdate(session: &Session, path: &PathBuf, manifest: &Manifest, new_version: &str, captures: &[String]) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let mut root: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| anyhow::anyhow!("parse: {}", e))?;

    root["version"] = serde_json::Value::String(new_version.to_string());

    // Build variable substitution
    let mut vars: Vec<(String, String)> = vec![
        ("$version".to_string(), new_version.to_string()),
    ];
    for (i, cap) in captures.iter().enumerate().skip(1) {
        vars.push((format!("$match{}", i), cap.clone()));
    }
    if captures.len() > 1 {
        let m1 = &captures[1];
        if !m1.is_empty() {
            let head = m1.chars().next().map(|c| c.to_string()).unwrap_or_default();
            let tail = m1[1..].to_string();
            vars.push(("$matchHead".to_string(), head));
            vars.push(("$matchTail".to_string(), tail));
        }
    }

    let sub_first = |s: &str| -> String {
        let mut r = s.to_string();
        for (k, v) in &vars {
            r = r.replace(k, v);
        }
        r
    };

    let au = match manifest.autoupdate() {
        Some(a) => a,
        None => { write_json(path, &root)?; return Ok(()); }
    };

    let tmp_dir = std::env::temp_dir().join("hok-autoupdate");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir)?;

    // ── Compute $basename from first URL after initial substitution ─────────
    if let Some(urls) = &au.url {
        let first_url = sub_first(urls.devectorize().first().copied().unwrap_or(""));
        let basename = url_basename(&first_url);
        vars.push(("$basename".to_string(), basename));
    }

    // Full substitution including $basename
    let sub = |s: &str| -> String {
        let mut r = s.to_string();
        for (k, v) in &vars {
            r = r.replace(k.as_str(), v.as_str());
        }
        r
    };

    // ── Read hash extractions from JSON (before mutable borrow) ────────────
    let top_hash_extractions: Vec<serde_json::Value> = root
        .get("autoupdate").and_then(|au| au.get("hash"))
        .and_then(|h| h.as_array()).cloned().unwrap_or_default();
    let arch_hash_extractions: [(&str, Vec<serde_json::Value>); 3] = [
        ("32bit", root.pointer("/architecture/32bit/autoupdate/hash")
            .and_then(|v| v.as_array()).cloned().unwrap_or_default()),
        ("64bit", root.pointer("/architecture/64bit/autoupdate/hash")
            .and_then(|v| v.as_array()).cloned().unwrap_or_default()),
        ("arm64", root.pointer("/architecture/arm64/autoupdate/hash")
            .and_then(|v| v.as_array()).cloned().unwrap_or_default()),
    ];

    // ── Top-level URLs ─────────────────────────────────────────────────────
    if let Some(urls) = &au.url {
        let substituted: Vec<String> = urls.devectorize().iter().map(|u| sub(u)).collect();
        let hashes = download_and_hash_multi(session, &substituted, &top_hash_extractions, &tmp_dir)?;

        root["url"] = json_str_array(&substituted);
        if !hashes.is_empty() {
            root["hash"] = json_str_array(&hashes);
        }
    }

    // ── Per-architecture URLs ──────────────────────────────────────────────
    if let Some(arch) = &au.architecture {
        for (arch_name, spec_opt, arch_extractions) in [
            ("32bit", arch.ia32.as_ref(), &arch_hash_extractions[0].1),
            ("64bit", arch.amd64.as_ref(), &arch_hash_extractions[1].1),
            ("arm64", arch.aarch64.as_ref(), &arch_hash_extractions[2].1),
        ] {
            let Some(spec) = spec_opt else { continue };
            if let Some(urls) = &spec.url {
                let substituted: Vec<String> = urls.devectorize().iter().map(|u| sub(u)).collect();
                let hashes = download_and_hash_multi(session, &substituted, arch_extractions, &tmp_dir)?;

                let ptr = format!("/architecture/{}", arch_name);

                if let Some(obj) = root.pointer_mut(&ptr) {
                    obj["url"] = json_str_array(&substituted);
                    if !hashes.is_empty() {
                        obj["hash"] = json_str_array(&hashes);
                    }
                }
            }
        }
    }

    // ── extract_dir ────────────────────────────────────────────────────────
    if let Some(dirs) = &au.extract_dir {
        let substituted: Vec<String> = dirs.devectorize().iter().map(|d| sub(d)).collect();
        root["extract_dir"] = json_str_array(&substituted);
    }

    let _ = std::fs::remove_dir_all(&tmp_dir);
    write_json(path, &root)?;
    Ok(())
}

/// Download files and determine hashes.
/// For each URL, if a corresponding hash extraction config exists with a URL,
/// fetch that page and extract the hash; otherwise download the file and compute SHA256.
fn download_and_hash_multi(
    session: &Session, urls: &[String],
    extractions: &[serde_json::Value], tmp_dir: &std::path::Path,
) -> Result<Vec<String>> {
    let mut hashes = Vec::new();
    for (i, url) in urls.iter().enumerate() {
        let extraction = extractions.get(i);

        // Check if hash should be extracted from a remote page
        let hash = if let Some(ext) = extraction {
            let mode = ext.get("mode").and_then(|m| m.as_str()).unwrap_or("extract");
            let has_url = ext.get("url").and_then(|u| u.as_str()).is_some();

            if mode == "download" || !has_url {
                // Download file and compute hash
                let filename = url.rsplit('/').next().unwrap_or("download");
                let dest = tmp_dir.join(filename);
                operation::download_file(session, url, &dest)
                    .map_err(|e| anyhow::anyhow!("download {}: {}", url, e))?;

                // Determine algorithm from the extraction (default sha256)
                let algo = ext.get("algorithm").and_then(|a| a.as_str()).unwrap_or("sha256");
                compute_hash(&dest, algo)?
            } else {
                // Fetch hash page and extract
                let hash_url = ext["url"].as_str().unwrap_or(url);
                let page_url = sub_url(hash_url, url); // substitute any remaining vars
                let page = operation::download_page(session, &page_url)
                    .map_err(|e| anyhow::anyhow!("fetch hash page {}: {}", page_url, e))?;
                extract_hash_from_page(&page, ext)?
            }
        } else {
            // No hash extraction config: download file and compute SHA256
            let filename = url.rsplit('/').next().unwrap_or("download");
            let dest = tmp_dir.join(filename);
            operation::download_file(session, url, &dest)
                .map_err(|e| anyhow::anyhow!("download {}: {}", url, e))?;
            compute_hash(&dest, "sha256")?
        };

        hashes.push(hash);
    }
    Ok(hashes)
}

/// Extract hash from page content using HashExtraction rules.
fn extract_hash_from_page(content: &str, ext: &serde_json::Value) -> Result<String> {
    // JSONPath first
    if let Some(jp) = ext.get("jp").or(ext.get("jsonpath")).and_then(|v| v.as_str()) {
        use jsonpath_rust::JsonPath;
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(content) {
            if let Ok(found) = val.query(jp) {
                let found_str = found.first().and_then(|v| match v {
                    serde_json::Value::String(s) => Some(s.clone()),
                    _ => v.as_str().map(|s| s.to_string()),
                });
                if let Some(h) = found_str {
                    if !h.is_empty() { return Ok(h); }
                }
            }
        }
    }

    // Regex
    if let Some(re_str) = ext.get("regex").and_then(|v| v.as_str()) {
        let url_for_re = ext.get("url").and_then(|u| u.as_str()).unwrap_or("");
        let re = Regex::new(re_str).map_err(|e| anyhow::anyhow!("bad hash regex: {}", e))?;
        if let Some(caps) = re.captures(content) {
            if let Some(h) = caps.get(1).or_else(|| caps.get(0)) {
                return Ok(h.as_str().to_string());
            }
        }
    }

    // Find (simple substring + next whitespace-delimited hex token)
    if let Some(find_str) = ext.get("find").and_then(|v| v.as_str()) {
        if let Some(pos) = content.find(find_str) {
            let after = &content[pos + find_str.len()..];
            // Scoop heuristic: look for the first hex token
            if let Some(hash) = after.split_whitespace().next() {
                let hash = hash.trim_matches(&['"', '\'', ',', ';', ':', '=', ' '][..]);
                if is_hex_hash(hash) {
                    return Ok(hash.to_string());
                }
                // Also check next token if first is an equals sign
            }
        }
    }

    Err(anyhow::anyhow!("could not extract hash from page"))
}

/// Substitute variables in a hash URL using the download URL's context.
fn sub_url(hash_url: &str, _download_url: &str) -> String {
    // Most hash URLs use the same $version etc. that were already substituted
    hash_url.to_string()
}

fn url_basename(url: &str) -> String {
    let filename = url.rsplit('/').next().unwrap_or(url);
    let dot = filename.rfind('.');
    match dot {
        Some(pos) => filename[..pos].to_string(),
        None => filename.to_string(),
    }
}

fn is_hex_hash(s: &str) -> bool {
    if s.is_empty() { return false; }
    let len = s.len();
    // MD5=32, SHA1=40, SHA256=64, SHA512=128 + algorithm prefixes
    let valid_len = matches!(len, 32 | 40 | 64 | 128)
        || (len > 5 && matches!(&s[..5], "md5:" | "sha1:" | "sha256" | "sha51"))
        || (len > 7 && &s[..7] == "sha512:");
    valid_len && s.chars().all(|c| c.is_ascii_hexdigit() || c == ':')
}

/// Compute hash of a file on disk using the given algorithm name.
fn compute_hash(path: &std::path::Path, algo: &str) -> Result<String> {
    let builder = ChecksumBuilder::new()
        .algo(algo)
        .map_err(|_| anyhow::anyhow!("unsupported hash algorithm: {}", algo))?;
    let mut hasher = builder.build();
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 { break; }
        hasher.consume(&buf[..n]);
    }
    Ok(hasher.finalize())
}

fn json_str_array(items: &[String]) -> serde_json::Value {
    serde_json::Value::Array(
        items.iter().map(|s| serde_json::Value::String(s.clone())).collect()
    )
}

fn write_json(path: &PathBuf, root: &serde_json::Value) -> Result<()> {
    let formatted = serde_json::to_string_pretty(root)
        .map_err(|e| anyhow::anyhow!("serialize: {}", e))?;
    std::fs::write(path, formatted.as_bytes())?;
    Ok(())
}
