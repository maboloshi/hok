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
            // GitHub shortcut: construct API URL from homepage
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

/// Check if the checkver uses the GitHub shortcut (regex matches /releases/tag/).
fn is_github_checkver(cv: &libscoop::Checkver) -> bool {
    cv.regex.as_deref().map_or(false, |r| r.contains("/releases/tag/"))
}

/// Extract GitHub API URL from a homepage URL.
/// e.g. "https://github.com/owner/repo" → "https://api.github.com/repos/owner/repo/releases/latest"
fn github_api_url(homepage: &str) -> Option<String> {
    let re = Regex::new(r"github\.com[:/]([^/]+/[^/]+?)(?:/|$)").ok()?;
    let caps = re.captures(homepage)?;
    let repo = caps.get(1)?.as_str().trim_end_matches('/');
    Some(format!("https://api.github.com/repos/{}/releases/latest", repo))
}

/// Extract version string from page content using checkver rules.
/// Returns `(version, captures)` where captures[0] is full match (if regex used).
fn extract_version(content: &str, cv: &libscoop::Checkver, jsonpath_override: Option<&str>) -> Option<(String, Vec<String>)> {
    // JSONPath: use override first (for GitHub API), then cv.jsonpath
    if let Some(jp) = jsonpath_override.or(cv.jsonpath.as_deref()) {
        use jsonpath_rust::JsonPath;
        let value: serde_json::Value = serde_json::from_str(content).ok()?;
        let found = value.query(jp).ok()?;
        let ver = found.first()?.as_str()?;
        if !ver.is_empty() {
            return Some((ver.to_string(), vec![ver.to_string()]));
        }
    }

    // Regex extraction
    if let Some(regex_str) = &cv.regex {
        let re = Regex::new(regex_str).ok()?;
        let caps = re.captures(content)?;
        let ver = caps.get(1).or_else(|| caps.get(0))?.as_str().to_string();
        let captures: Vec<String> = caps.iter()
            .map(|m| m.map(|s| s.as_str().to_string()).unwrap_or_default())
            .collect();
        return Some((ver, captures));
    }

    // No JSONPath or regex: treat content itself as version string
    let trimmed = content.trim();
    if !trimmed.is_empty() {
        Some((trimmed.to_string(), vec![trimmed.to_string()]))
    } else {
        None
    }
}

/// Apply autoupdate to a manifest: replace $version/$matchN in URLs, download files,
/// compute hashes, and write the updated manifest.
fn apply_autoupdate(session: &Session, path: &PathBuf, manifest: &Manifest, new_version: &str, captures: &[String]) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let mut root: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| anyhow::anyhow!("parse: {}", e))?;

    // Update version field
    root["version"] = serde_json::Value::String(new_version.to_string());

    // Build substitution function for $version, $matchN, $matchHead, $matchTail
    let sub = |s: &str| -> String {
        let mut r = s.replace("$version", new_version);
        // $match1 = captures[1] (first regex capture group), $match2 = captures[2], etc.
        for (i, cap) in captures.iter().enumerate().skip(1) {
            let pat = format!("$match{}", i);
            r = r.replace(&pat, cap);
        }
        // $matchHead = first char of $match1, $matchTail = rest of $match1
        if captures.len() > 1 {
            let m1 = &captures[1];
            if !m1.is_empty() {
                let head = m1.chars().next().map(|c| c.to_string()).unwrap_or_default();
                let tail = m1[1..].to_string();
                r = r.replace("$matchHead", &head);
                r = r.replace("$matchTail", &tail);
            }
        }
        r
    };

    // Look for autoupdate section
    let au = match manifest.autoupdate() {
        Some(a) => a,
        None => {
            // No autoupdate — just update version
            write_json(path, &root)?;
            return Ok(());
        }
    };

    let tmp_dir = std::env::temp_dir().join("hok-autoupdate");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir)?;

    // Collect all (url_template, hash_dest) pairs to process
    // Top-level URL → update root["url"], root["hash"]
    if let Some(urls) = &au.url {
        let substituted: Vec<String> = urls.devectorize().into_iter().map(|u| sub(u)).collect();
        let hashes = download_and_hash(session, &substituted, &tmp_dir)?;

        root["url"] = serde_json::Value::Array(
            substituted.iter().map(|u| serde_json::Value::String(u.clone())).collect()
        );
        root["hash"] = serde_json::Value::Array(
            hashes.iter().map(|h| serde_json::Value::String(h.clone())).collect()
        );
    }

    // Per-architecture URLs
    if let Some(arch) = &au.architecture {
        let arch_pairs: [(&str, Option<&_>); 3] = [
            ("32bit", arch.ia32.as_ref()),
            ("64bit", arch.amd64.as_ref()),
            ("arm64", arch.aarch64.as_ref()),
        ];
        for (arch_name, arch_spec) in arch_pairs {
            let Some(spec) = arch_spec else { continue };

            if let Some(urls) = &spec.url {
                let substituted: Vec<String> = urls.devectorize().into_iter().map(|u| sub(u)).collect();
                let hashes = download_and_hash(session, &substituted, &tmp_dir)?;

                let ptr = format!("/architecture/{}", arch_name);
                if let Some(obj) = root.pointer_mut(&ptr) {
                    obj["url"] = serde_json::Value::Array(
                        substituted.iter().map(|u| serde_json::Value::String(u.clone())).collect()
                    );
                    obj["hash"] = serde_json::Value::Array(
                        hashes.iter().map(|h| serde_json::Value::String(h.clone())).collect()
                    );
                }
            }
        }
    }

    // Update extract_dir if present in autoupdate
    if let Some(extract_dirs) = &au.extract_dir {
        let substituted: Vec<String> = extract_dirs.devectorize().into_iter().map(|d| sub(d)).collect();
        root["extract_dir"] = serde_json::Value::Array(
            substituted.into_iter().map(|d| serde_json::Value::String(d)).collect()
        );
    }

    // Cleanup temp directory
    let _ = std::fs::remove_dir_all(&tmp_dir);

    write_json(path, &root)?;
    Ok(())
}

/// Download files from URLs, compute SHA256 hash for each, return hash strings.
fn download_and_hash(session: &Session, urls: &[String], tmp_dir: &std::path::Path) -> Result<Vec<String>> {
    let mut hashes = Vec::new();
    for url in urls {
        let filename = url.rsplit('/').next().unwrap_or("download");
        let dest = tmp_dir.join(filename);

        operation::download_file(session, url, &dest)
            .map_err(|e| anyhow::anyhow!("download {}: {}", url, e))?;

        let hex = compute_hash(&dest, "sha256")?;
        hashes.push(hex);
    }
    Ok(hashes)
}

/// Pretty-print and write updated JSON to disk, preserving original formatting
fn write_json(path: &PathBuf, root: &serde_json::Value) -> Result<()> {
    let formatted = serde_json::to_string_pretty(root)
        .map_err(|e| anyhow::anyhow!("serialize: {}", e))?;
    std::fs::write(path, formatted.as_bytes())?;
    Ok(())
}

/// Compute hash of a file using specified algorithm name.
fn compute_hash(path: &std::path::Path, algo: &str) -> Result<String> {
    let builder = ChecksumBuilder::new()
        .algo(algo)
        .map_err(|_| anyhow::anyhow!("unsupported hash algorithm: {}", algo))?;
    let mut hasher = builder.build();
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.consume(&buf[..n]);
    }
    Ok(hasher.finalize())
}
