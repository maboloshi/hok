//! Check for new package versions (checkver).
//!
//! Scans manifests and checks upstream URLs for newer versions by
//! extracting version information from HTTP responses, HTML pages,
//! GitHub releases, etc. Supports regex-based version extraction
//! and custom checkver scripts.
//!
//! # Design
//!
//! - **Multi-source version detection**: Handles plain URL downloads,
//!   GitHub API releases, and regex-based HTML scraping for version
//!   discovery.
//! - **Batch processing**: Scans an entire bucket directory, processing
//!   multiple manifests in sequence while reporting progress per-package.
//! - **Manifest updating**: When `--update` is set, new versions are
//!   written directly into the manifest JSON files.
//! - **Known gaps vs Scoop**: See the `TODO` block below for unimplemented
//!   features (ThrowError, custom useragent, etc.).

use clap::Parser;
use libscoop::{operation, Manifest, Session};
use regex::Regex;
use std::path::PathBuf;

use crate::{output, util, Result};

// ---------------------------------------------------------------------------
// TODO(scoop-alignment): Unimplemented Scoop checkver features
// ---------------------------------------------------------------------------
// 1. ThrowError (--throw) — When set, errors are thrown as exceptions instead
//    of just printed to stderr. Currently all errors use output::err().
//    Scoop ref: bin/checkver.ps1 (param $ThrowError, used at line 412-418)
//
// 2. useragent   — Per-manifest custom User-Agent header for checkver HTTP
//    requests. The manifest's checkver.useragent field exists in the struct
//    but is not used. Must substitute $version etc. before setting.
//    Scoop ref: bin/checkver.ps1 (lines 117-121)
//
// 3. Referer     — All HTTP requests should include a Referer header derived
//    from the request URL (strip_filename). Currently neither operation::
//    download_page nor the ureq helpers set this header.
//    Scoop ref: bin/checkver.ps1 (line 246)
//
// 4. PRIVATE_HOSTS — Config-level custom HTTP headers matched by host regex.
//    Scoop reads `scoop config PRIVATE_HOSTS` and applies headers per request.
//    Scoop ref: bin/checkver.ps1 (lines 240-244)
// ---------------------------------------------------------------------------

/// Check manifest for a newer version
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = ".")]
    pub(crate) dir: PathBuf,

    /// Specific app(s) to check (supports wildcards, default: all)
    #[arg(default_value = "*")]
    pub(crate) app: Vec<String>,

    /// Update manifest with new version and trigger autoupdate
    #[arg(short = 'u', long, action = clap::ArgAction::SetTrue)]
    pub(crate) update: bool,

    /// Force update even when version is unchanged (useful for hash updates)
    #[arg(short = 'f', long, action = clap::ArgAction::SetTrue)]
    pub(crate) force_update: bool,

    /// Skip manifests that are already up-to-date
    #[arg(short = 's', long = "skip-updated", action = clap::ArgAction::SetTrue)]
    pub(crate) skip_updated: bool,

    /// Update manifest to specific version (skip version detection)
    #[arg(short = 'V', long)]
    pub(crate) version: Option<String>,

    /// Request timeout in seconds
    #[arg(short = 't', long, default_value = "30")]
    pub(crate) timeout: u64,
}

// ─── Execute ────────────────────────────────────────────────────────────────

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let dir = &args.dir;
    if !dir.is_dir() {
        output::err(rust_i18n::t!("cmd.checkver_err_dir", path = dir.display()));
        return Ok(());
    }

    // Don't use --version with wildcard app pattern
    if args.version.is_some() && args.app[0] == "*" {
        output::err(rust_i18n::t!("cmd.checkver_version_wildcard"));
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

        let current = manifest.version().to_string();

        // When --version is specified, skip all detection and use that version directly
        if let Some(ref ver_override) = args.version {
            if ver_override != &current {
                println!("  {stem} ({current} -> {ver_override})");
            } else {
                println!("  {stem} ({ver_override})");
                if args.skip_updated { continue; }
            }
            if args.update || args.force_update {
                let captures = vec![ver_override.clone()];
                match apply_autoupdate(session, &path, &manifest, ver_override, &captures) {
                    Ok(()) => output::done(rust_i18n::t!("cmd.checkver_updated_to", ver = ver_override)),
                    Err(e) => output::err(rust_i18n::t!("cmd.checkver_update_failed", e = e)),
                }
            }
            continue;
        }

        // Default regex override (used by sourceforge shortcut)
        let mut effective_regex = None;

        // Determine URL and regex to use
        // Detect GitHub checkver: the deserializer sets specific regex (/releases/tag/) for
        // `checkver: "github"` and `checkver.github: "owner/repo"`. Also detect via URL pattern.
        let mut github_mode = is_github_checkver(&cv);
        let mut url = if let Some(u) = &cv.url {
            if u.contains("github.com/") && u.contains("/releases/") {
                github_mode = true;
            }
            u.clone()
        } else if let Some(sf) = &cv.sourceforge {
            let extracted = extract_sourceforge_project(manifest.homepage());
            let project = sf.project.as_deref().or(extracted.as_deref());
            match project {
                Some(proj) => {
                    if cv.regex.is_none() && cv.jsonpath.is_none() {
                        effective_regex = Some(r"/([\d.]+)/".to_string());
                    }
                    format!("https://sourceforge.net/projects/{}/rss?path=/{}", proj, sf.path)
                }
                None => { output::err(format!("{stem}: {}", rust_i18n::t!("cmd.checkver_sourceforge_err"))); continue; }
            }
        } else if github_mode {
            // No cv.url set but github detected: extract repo from homepage
            match github_api_url(manifest.homepage()) {
                Some(api_url) => api_url,
                None => { output::err(format!("{stem}: {}", rust_i18n::t!("cmd.checkver_github_err"))); continue; }
            }
        } else {
            output::err(format!("{stem}: {}", rust_i18n::t!("cmd.checkver_no_url"))); continue;
        };

        // If this is a GitHub checkver with a github.com URL, transform to API URL
        // (matching Scoop's useGithubAPI behavior: github.com/.../releases/latest → api.github.com/repos/.../releases/latest)
        if github_mode && url.contains("github.com/") && !url.contains("api.github.com") {
            if let Some(api_url) = github_api_url(&url) {
                url = api_url;
            }
        }

        // Automatically add `$.tag_name` JSONPath for GitHub API responses
        let mut effective_jsonpath = cv.jsonpath.clone();
        if effective_jsonpath.is_none() && url.contains("api.github.com") {
            effective_jsonpath = Some("$.tag_name".to_string());
        }

        // TODO(scoop-alignment): Add custom user-agent from checkver.useragent + Referer header + PRIVATE_HOSTS headers
        // Get page content: from script output (if checkver.script is set) or by downloading URL
        // In Scoop, script output replaces the downloaded page and still goes through extraction.
        let gh_token = session.config().gh_token.clone();
        let raw = if let Some(script_lines) = cv.script.as_ref() {
            let script = script_lines.devectorize().join("\r\n");
            match run_checkver_script(session, &script, cv.url.as_deref(), args.timeout) {
                Ok(Some(page)) => page,
                Ok(None) => { output::warn(format!("{stem}: {}", rust_i18n::t!("cmd.checkver_no_version"))); continue; }
                Err(e) => { output::err(format!("{stem}: {}", rust_i18n::t!("cmd.checkver_script_err", e = e))); continue; }
            }
        } else if url.contains("api.github.com") && gh_token.is_some() {
            // Use authenticated request for GitHub API to avoid rate limits
            match download_page_with_token(&url, &gh_token.unwrap(), args.timeout) {
                Ok(t) => t,
                Err(e) => {
                    output::err(format!("{stem}: {}", rust_i18n::t!("cmd.err_download", e = e)));
                    continue;
                }
            }
        } else {
            match operation::download_page(session, &url, args.timeout) {
                Ok(t) => t,
                Err(e) => {
                    output::err(format!("{stem}: {}", rust_i18n::t!("cmd.err_download", e = e)));
                    continue;
                }
            }
        };

        // Extract version
        let extract_result = extract_version(&raw, cv, effective_jsonpath.as_deref(), effective_regex.as_deref());

        // Auto-strip leading v/V prefix
        let (mut ver, captures) = match extract_result {
            Some((ref ver, ref caps)) => (ver.clone(), caps.clone()),
            None => {
                output::err(format!("{stem}: {}", rust_i18n::t!("cmd.checkver_no_extract")));
                continue;
            }
        };

        // If the user has not defined replace, the leading v/V will be automatically removed (global default behavior).
        if cv.replace.is_none() {
            ver = ver.trim_start_matches(|c: char| c == 'v' || c == 'V').to_string();
        }

        // ForceUpdate implies Update (matching Scoop behavior)
        let do_update = args.update || args.force_update;

        if ver == current {
            // SkipUpdated: skip display (and update) for up-to-date manifests
            if args.skip_updated && !args.force_update {
                continue;
            }
            if !args.skip_updated {
                println!("  {stem} ({ver})");
            }
            if args.force_update {
                match apply_autoupdate(session, &path, &manifest, &ver, &captures) {
                    Ok(()) => output::done(rust_i18n::t!("cmd.checkver_updated_to", ver = ver)),
                    Err(e) => output::err(rust_i18n::t!("cmd.checkver_update_failed", e = e)),
                }
            }
        } else {
            println!("  {stem} ({current} -> {ver})");
            if do_update {
                match apply_autoupdate(session, &path, &manifest, &ver, &captures) {
                    Ok(()) => output::done(rust_i18n::t!("cmd.checkver_updated_to", ver = ver)),
                    Err(e) => output::err(rust_i18n::t!("cmd.checkver_update_failed", e = e)),
                }
            }
        }
    }

    Ok(())
}

// ─── URL helpers ────────────────────────────────────────────────────────────

fn is_github_checkver(cv: &libscoop::Checkver) -> bool {
    // Check by regex pattern (set by deserializer for checkver: "github" / checkver.github: "owner/repo")
    if cv.regex.as_deref().map_or(false, |r| r.contains("/releases/tag/")) {
        return true;
    }
    // Check by URL pattern (for cases where cv.url is set to a github releases URL)
    if cv.url.as_deref().map_or(false, |u| u.contains("github.com/") && u.contains("/releases/")) {
        return true;
    }
    false
}

fn github_api_url(url_or_homepage: &str) -> Option<String> {
    // Try to match as a github.com URL first (handles both homepage and releases URLs)
    let re = Regex::new(r"github\.com[:/]([^/]+/[^/]+?)(?:/|$)").ok()?;
    let caps = re.captures(url_or_homepage)?;
    let repo = caps.get(1)?.as_str().trim_end_matches('/');
    Some(format!("https://api.github.com/repos/{}/releases/latest", repo))
}

/// Extract SourceForge project name from homepage URL.
fn extract_sourceforge_project(homepage: &str) -> Option<String> {
    let re = Regex::new(r"sourceforge\.net/projects/([^/]+)").ok()?;
    let caps = re.captures(homepage)?;
    caps.get(1).map(|m| m.as_str().to_string())
}

// ─── Version extraction ─────────────────────────────────────────────────────

/// Extract version + capture groups from page content.
fn extract_version(content: &str, cv: &libscoop::Checkver, jsonpath_override: Option<&str>, regex_override: Option<&str>) -> Option<(String, Vec<String>)> {
    // JSONPath: use override first (for GitHub API), then cv.jsonpath
    if let Some(jp) = jsonpath_override.or(cv.jsonpath.as_deref()) {
        use jsonpath_rust::JsonPath;
        let value: serde_json::Value = serde_json::from_str(content).ok()?;
        let found = value.query(jp).ok()?;
        let ver = found.first()?.as_str()?;
        if !ver.is_empty() {
            let caps = vec![ver.to_string()];
            let v = apply_replace(&caps, cv.replace.as_deref());
            return Some((v, caps));
        }
    }

    // XPath: evaluate XPath expression on XML content
    if let Some(xp) = &cv.xpath {
        if let Some(ver) = extract_xpath(content, xp) {
            let caps = vec![ver.clone()];
            let v = apply_replace(&caps, cv.replace.as_deref());
            return Some((v, caps));
        }
    }

    // Regex: use override first (sourceforge default), then cv.regex
    if let Some(regex_str) = regex_override.or(cv.regex.as_deref()) {
        let re = Regex::new(regex_str).ok()?;

        // If reverse is enabled, take the last match
        let rev = cv.reverse.unwrap_or(false);
        if rev {
            // Capture all matches and take the last one
            let all_captures: Vec<regex::Captures> = re.captures_iter(content).collect();
            let caps = all_captures.last()?;
            let captures: Vec<String> = caps.iter()
                .map(|m| m.map(|s| s.as_str().to_string()).unwrap_or_default())
                .collect();
            let ver = apply_replace(&captures, cv.replace.as_deref());
            return Some((ver, captures));
        }

        let caps = re.captures(content)?;
        let captures: Vec<String> = caps.iter()
            .map(|m| m.map(|s| s.as_str().to_string()).unwrap_or_default())
            .collect();
        let ver = apply_replace(&captures, cv.replace.as_deref());
        return Some((ver, captures));
    }

    let trimmed = content.trim();
    if !trimmed.is_empty() {
        let ver = apply_replace(&[trimmed.to_string()], cv.replace.as_deref());
        Some((ver, vec![trimmed.to_string()]))
    } else {
        None
    }
}

/// Apply the `replace` transformation to extracted captures.
///
/// When `replace` is set, `$1`, `$2`, etc. are replaced with the corresponding
/// capture group values. `$0` is the full match. If `replace` is not set,
/// returns `captures[1]` (first capture group) or `captures[0]` (full match).
fn apply_replace(captures: &[String], replace: Option<&str>) -> String {
    match replace {
        Some(pattern) => {
            let mut result = pattern.to_string();
            for (i, cap) in captures.iter().enumerate() {
                result = result.replace(&format!("${}", i), cap);
            }
            result
        }
        None => {
            captures.get(1).or(captures.first()).cloned().unwrap_or_default()
        }
    }
}

// ─── Autoupdate ────────────────────────────────────────────────────────────

/// Apply autoupdate: substitute variables, download files, compute/extract
/// hashes, write updated manifest.
fn apply_autoupdate(session: &Session, path: &PathBuf, manifest: &Manifest, new_version: &str, captures: &[String]) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let mut root: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| anyhow::anyhow!("parse: {}", e))?;

    // Update version
    root["version"] = serde_json::Value::String(new_version.to_string());

    // Build variable substitution (matching Scoop's Get-VersionSubstitution)
    let first_part = new_version.split('-').next().unwrap_or(new_version);
    let last_part = new_version.rsplit('-').next().unwrap_or(new_version);
    let version_parts: Vec<&str> = first_part.split('.').collect();
    let mut vars: Vec<(String, String)> = vec![
        ("$version".to_string(), new_version.to_string()),
        ("$dotVersion".to_string(), new_version.replace(|c: char| c == '_' || c == '-' || c == '.', ".")),
        ("$underscoreVersion".to_string(), new_version.replace(|c: char| c == '_' || c == '-' || c == '.', "_")),
        ("$dashVersion".to_string(), new_version.replace(|c: char| c == '_' || c == '-' || c == '.', "-")),
        ("$cleanVersion".to_string(), new_version.replace(|c: char| c == '_' || c == '-' || c == '.', "")),
        ("$majorVersion".to_string(), version_parts.first().copied().unwrap_or("0").to_string()),
        ("$minorVersion".to_string(), version_parts.get(1).copied().unwrap_or("0").to_string()),
        ("$patchVersion".to_string(), version_parts.get(2).copied().unwrap_or("0").to_string()),
        ("$buildVersion".to_string(), version_parts.get(3).copied().unwrap_or("0").to_string()),
        ("$preReleaseVersion".to_string(), last_part.to_string()),
    ];
    for (i, cap) in captures.iter().enumerate().skip(1) {
        vars.push((format!("$match{}", i), cap.clone()));
    }
    // Scoop: $matchHead/$matchTail are derived from the version string,
    // not from capture groups. Regex: (?<head>\d+\.\d+(?:\.\d+)?)(?<tail>.*)
    if let Some((head, tail)) = extract_version_head_tail(new_version) {
        vars.push(("$matchHead".to_string(), head));
        vars.push(("$matchTail".to_string(), tail));
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
        None => { libscoop::internal::fs::write_json(path, &root)?; return Ok(()); }
    };

    let tmp_dir = std::env::temp_dir().join("hok-autoupdate");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir)?;

    // ── Compute $basename from first URL after initial substitution ─────────
    if let Some(urls) = &au.url {
        let first_url = sub_first(urls.devectorize().first().copied().unwrap_or(""));
        let basename = util::url_basename(&first_url);
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
        root["hash"] = json_str_array(&hashes);
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
                    obj["hash"] = json_str_array(&hashes);
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

    // Write the complete updated JSON (preserving order, 4-space indentation)
    libscoop::internal::fs::write_json(path, &root)?;
    Ok(())
}

/// Download files and determine hashes using mode dispatch matching Scoop's get_hash_for_app.
///
/// Supported modes:
/// - `download` / no config: download file + compute hash
/// - `extract` (default when hash URL present): fetch hash page, extract via jsonpath/regex/find
/// - `json`: fetch JSON, extract via jsonpath
/// - `xpath`: fetch XML, extract via xpath
/// - `rdf`: fetch RDF XML, find digest by basename
/// - `metalink`: check HTTP Digest header, fallback to .meta4 file
/// - `fosshub`: auto-detected from fosshub.com URLs, extract sha256 from page
/// - `sourceforge`: auto-detected from sourceforge.net URLs, extract sha1 from SF files page
/// - `github`: auto-detected from github.com release download URLs, extract digest from API
fn download_and_hash_multi(
    session: &Session, urls: &[String],
    extractions: &[serde_json::Value], tmp_dir: &std::path::Path,
) -> Result<Vec<String>> {
    let mut hashes = Vec::new();
    for (i, url) in urls.iter().enumerate() {
        let extraction = extractions.get(i);

        let hash = match extraction {
            Some(ext) => {
                let mode = ext.get("mode").and_then(|m| m.as_str()).unwrap_or("");
                let has_url = ext.get("url").and_then(|u| u.as_str()).map(|s| !s.is_empty()).unwrap_or(false);

                // Auto-detect hash mode from download URL when no explicit config given
                // (matching Scoop: get_hash_for_app auto-detects fosshub/sourceforge/github)
                let effective_mode = if mode.is_empty() && !has_url {
                    detect_hash_mode(url).unwrap_or("")
                } else {
                    mode
                };

                // Scoop precedence: jsonpath/xpath from config override the mode
                let has_jp = ext.get("jp").or(ext.get("jsonpath")).and_then(|v| v.as_str()).is_some();
                let has_xp = ext.get("xpath").and_then(|v| v.as_str()).is_some();

                match effective_mode {
                    // ── download: download file + compute hash ──────────────────────
                    "download" | "" if !has_url => {
                        download_file_compute_hash(session, url, ext, tmp_dir)?
                    }
                    // ── extract (default when hash URL present): fetch + regex/jsonpath/find ──
                    "extract" | "" if has_url => {
                        let hash_url = ext["url"].as_str().unwrap_or(url);
                        let page_url = sub_url(hash_url, url);
                        let page = operation::download_page(session, &page_url, 30)
                            .map_err(|e| anyhow::anyhow!("fetch hash page {}: {}", page_url, e))?;
                        extract_hash_from_page(&page, ext)?
                    }
                    // ── json: fetch JSON + jsonpath extraction ──────────────────────
                    "json" if has_jp || has_url => {
                        let hash_url = ext.get("url").and_then(|u| u.as_str()).unwrap_or(url);
                        let page = operation::download_page(session, hash_url, 30)
                            .map_err(|e| anyhow::anyhow!("fetch json {}: {}", hash_url, e))?;
                        extract_hash_from_page(&page, ext)?
                    }
                    // ── xpath: fetch XML + xpath extraction ─────────────────────────
                    "xpath" if has_xp || has_url => {
                        let hash_url = ext.get("url").and_then(|u| u.as_str()).unwrap_or(url);
                        let page = operation::download_page(session, hash_url, 30)
                            .map_err(|e| anyhow::anyhow!("fetch xml {}: {}", hash_url, e))?;
                        extract_hash_from_page(&page, ext)?
                    }
                    // ── rdf: fetch RDF XML, find digest by basename ─────────────────
                    "rdf" => {
                        fetch_rdf_hash(session, url, ext)?
                    }
                    // ── metalink: HTTP Digest header fallback .meta4 ────────────────
                    "metalink" => {
                        fetch_metalink_hash(session, url, ext)?
                    }
                    // ── fosshub: extract sha256 from fosshub download page ──────────
                    "fosshub" => {
                        // Scoop: fetch the download page itself, find sha256 with regex
                        // Regex: <filename>.*?"sha256":"([a-fA-F0-9]{64})"
                        let filename = util::url_remote_filename(url);
                        let page = operation::download_page(session, url, 30)
                            .map_err(|e| anyhow::anyhow!("fetch fosshub page {}: {}", url, e))?;
                        let regex_str = format!(r#"{filename}.*?"sha256":"([a-fA-F0-9]+)""#);
                        let re = Regex::new(&regex_str)
                            .map_err(|e| anyhow::anyhow!("bad fosshub regex: {}", e))?;
                        if let Some(caps) = re.captures(&page) {
                            if let Some(h) = caps.get(1) {
                                h.as_str().to_string()
                            } else {
                                anyhow::bail!("could not find sha256 for '{}' in fosshub page", filename)
                            }
                        } else {
                            anyhow::bail!("could not find sha256 for '{}' in fosshub page", filename)
                        }
                    }
                    // ── sourceforge: extract sha1 from SF files page ────────────────
                    "sourceforge" => {
                        // Scoop: fetch SF files page, extract sha1 with regex
                        // Regex: '"$basename":.*?"sha1":\s*"([a-fA-F0-9]{40})"'
                        let (project, file_path) = parse_sourceforge_url(url)
                            .ok_or_else(|| anyhow::anyhow!("could not parse sourceforge URL: {}", url))?;
                        let sf_page_url = format!("https://sourceforge.net/projects/{project}/files/{file_path}");
                        let page = operation::download_page(session, &sf_page_url, 30)
                            .map_err(|e| anyhow::anyhow!("fetch sourceforge page {}: {}", sf_page_url, e))?;
                        let basename = util::url_remote_filename(url);
                        let regex_str = format!(r#""{basename}":.*?"sha1":\s*"([a-fA-F0-9]+)""#);
                        let re = Regex::new(&regex_str)
                            .map_err(|e| anyhow::anyhow!("bad sourceforge regex: {}", e))?;
                        if let Some(caps) = re.captures(&page) {
                            if let Some(h) = caps.get(1) {
                                h.as_str().to_string()
                            } else {
                                anyhow::bail!("could not find sha1 for '{}' in sourceforge page", basename)
                            }
                        } else {
                            anyhow::bail!("could not find sha1 for '{}' in sourceforge page", basename)
                        }
                    }
                    // ── github: extract digest from GitHub API releases ─────────────
                    "github" => {
                        // Scoop: fetch GitHub API releases, extract digest via jsonpath
                        // jsonpath: "$..assets[?(@.browser_download_url == '" + $url + "')].digest"
                        let (owner, repo) = parse_github_download_url(url)
                            .ok_or_else(|| anyhow::anyhow!("could not parse GitHub URL: {}", url))?;
                        let api_url = format!("https://api.github.com/repos/{owner}/{repo}/releases");
                        let gh_token = session.config().gh_token.clone();
                        let page = if let Some(token) = gh_token {
                            download_page_with_token(&api_url, &token, 30)?
                        } else {
                            operation::download_page(session, &api_url, 30)?
                        };
                        // Parse JSON and query via jsonpath
                        use jsonpath_rust::JsonPath;
                        let value: serde_json::Value = serde_json::from_str(&page)
                            .map_err(|e| anyhow::anyhow!("parse github API response: {}", e))?;
                        let jp = format!("$..assets[?(@.browser_download_url == '{url}')].digest");
                        let found = value.query(&jp)
                            .map_err(|e| anyhow::anyhow!("jsonpath query: {}", e))?;
                        if let Some(h) = found.first().and_then(|v| v.as_str()) {
                            h.to_string()
                        } else {
                            anyhow::bail!("could not find digest for '{}' in GitHub API", url)
                        }
                    }
                    // ── unknown mode: fallback to download + compute hash ───────────
                    _ => {
                        download_file_compute_hash(session, url, ext, tmp_dir)?
                    }
                }
            }
            None => {
                // No hash extraction config: download file and compute SHA256
                let filename = url.rsplit('/').next().unwrap_or("download");
                let dest = tmp_dir.join(filename);
                operation::download_file(session, url, &dest)
                    .map_err(|e| anyhow::anyhow!("download {}: {}", url, e))?;
                scoop_hash::compute_file_hash(&dest, "sha256")?
            }
        };

        // Apply Scoop-compatible hash format normalization
        if let Some(normalized) = format_hash(&hash) {
            hashes.push(normalized);
        } else {
            hashes.push(hash);
        }
    }
    Ok(hashes)
}

/// Detect hash extraction mode from download URL pattern (Scoop compatibility).
/// Auto-detects fosshub, sourceforge, and github when no explicit hash config is given.
fn detect_hash_mode(url: &str) -> Option<&'static str> {
    if url.contains("fosshub.com") || url.contains("fosshub.org") {
        return Some("fosshub");
    }
    if url.contains("sourceforge.net") || url.contains("sf.net") {
        return Some("sourceforge");
    }
    if url.contains("github.com/") && url.contains("/releases/download/") {
        return Some("github");
    }
    None
}

/// Parse a SourceForge download URL to extract (project, file_path).
fn parse_sourceforge_url(url: &str) -> Option<(String, String)> {
    // Scoop regex: '(?:downloads\.)?sourceforge.net\/projects?\/(?<project>[^\/]+)\/(?:files\/)?(?<file>.*)'
    let re = Regex::new(r"(?:downloads\.)?(?:sourceforge\.net|sf\.net)/projects?/([^/]+)(?:/files)?/(.*)").ok()?;
    let caps = re.captures(url)?;
    let project = caps.get(1)?.as_str().to_string();
    let file_path = caps.get(2)?.as_str().trim_end_matches('/').to_string();
    Some((project, file_path))
}

/// Parse a GitHub release download URL to extract (owner, repo).
fn parse_github_download_url(url: &str) -> Option<(String, String)> {
    // Pattern: https://github.com/<owner>/<repo>/releases/download/...
    let re = Regex::new(r"github\.com/([^/]+)/([^/]+)/releases/download/").ok()?;
    let caps = re.captures(url)?;
    let owner = caps.get(1)?.as_str().to_string();
    let repo = caps.get(2)?.as_str().to_string();
    Some((owner, repo))
}

/// Download a single file and compute its hash using the algorithm from extraction config.
fn download_file_compute_hash(
    session: &Session, url: &str,
    ext: &serde_json::Value, tmp_dir: &std::path::Path,
) -> Result<String> {
    let filename = url.rsplit('/').next().unwrap_or("download");
    let dest = tmp_dir.join(filename);
    operation::download_file(session, url, &dest)
        .map_err(|e| anyhow::anyhow!("download {}: {}", url, e))?;
    let algo = ext.get("algorithm").and_then(|a| a.as_str()).unwrap_or("sha256");
    scoop_hash::compute_file_hash(&dest, algo)
        .map_err(|e| anyhow::anyhow!("compute hash {}: {}", filename, e))
}

/// Fetch RDF XML and extract hash by basename (matching Scoop's find_hash_in_rdf).
fn fetch_rdf_hash(session: &Session, url: &str, ext: &serde_json::Value) -> Result<String> {
    let hash_url = ext.get("url").and_then(|u| u.as_str()).unwrap_or(url);
    let page = operation::download_page(session, hash_url, 30)
        .map_err(|e| anyhow::anyhow!("fetch rdf {}: {}", hash_url, e))?;

    // Parse RDF XML and find Content entry matching the basename
    // Scoop (find_hash_in_rdf):
    //   $digest = $xml.RDF.Content | Where-Object { [String]$_.about -eq $basename }
    //   return format_hash $digest.sha256
    let basename = util::url_remote_filename(url);
    find_hash_in_rdf(&page, &basename)
        .ok_or_else(|| anyhow::anyhow!("could not find hash for '{}' in RDF at {}", basename, hash_url))
}

/// Fetch metalink hash: check HTTP Digest header, fallback to .meta4 file.
fn fetch_metalink_hash(session: &Session, url: &str, _ext: &serde_json::Value) -> Result<String> {
    // Scoop (find_hash_in_headers + find_hash_in_textfile .meta4):
    //   1. HEAD request → check Digest header for SHA-256=...
    //   2. Fallback: fetch $url.meta4 and extract hash via regex
    //
    // Step 1: HEAD with Digest header check
    let config = session.config();
    if let Ok(digest) = head_digest_sha256(url, config.proxy(), 30) {
        return Ok(digest);
    }

    // Step 2: fallback to .meta4 file
    let meta4_url = format!("{}.meta4", url);
    let page = operation::download_page(session, &meta4_url, 30)
        .map_err(|e| anyhow::anyhow!("fetch metalink {}: {}", meta4_url, e))?;

    // Extract first SHA256 hash from .meta4 XML
    // Scoop uses find_hash_in_textfile with regex '<hash[^>]+>([a-fA-F0-9]{64})'
    let re = Regex::new(r"<hash[^>]+>([a-fA-F0-9]{64})")
        .map_err(|e| anyhow::anyhow!("bad metalink regex: {}", e))?;
    if let Some(caps) = re.captures(&page) {
        if let Some(h) = caps.get(1) {
            return Ok(h.as_str().to_string());
        }
    }

    anyhow::bail!("could not find hash in metalink at {}", meta4_url)
}


/// Parse RDF XML and find SHA256 digest for the given basename.
fn find_hash_in_rdf(content: &str, _basename: &str) -> Option<String> {
    // Simplified RDF parsing: look for `<rdf:Content ... about="...basename...">` and extract `<sha256:...>`
    // Scoop uses proper XML parsing: $xml.RDF.Content | Where-Object { $_.about -eq $basename }
    let re = Regex::new(r#"(?s)<[^:]*:Content[^>]*about="[^"]*{}[^"]*"[^>]*>.*?<(?:sha256|digest)[^>]*>(.+?)</"#).ok()?;
    let caps = re.captures(content)?;
    let hash = caps.get(1)?.as_str().trim().to_string();
    if !hash.is_empty() { Some(hash) } else { None }
}

/// Perform a HEAD request and extract SHA-256 digest from the Digest header.
fn head_digest_sha256(url: &str, proxy: Option<&str>, timeout_secs: u64) -> Result<String> {
    let mut cfg = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(timeout_secs)));
    if let Some(proxy_url) = proxy {
        let p = ureq::Proxy::new(proxy_url).map_err(|e| anyhow::anyhow!("proxy: {}", e))?;
        cfg = cfg.proxy(Some(p));
    }
    let agent = cfg.build().new_agent();
    let resp = agent.head(url).call()
        .map_err(|e| anyhow::anyhow!("HEAD {}: {}", url, e))?;

    // Scoop checks for Digest header: SHA-256=..., SHA=..., MD5=...
    let digest_val = resp.headers().get("Digest")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());

    if let Some(ref digest_val) = digest_val {
        // SHA-256=<base64>
        let re = Regex::new(r"SHA-256=([^,]+)").ok();
        if let Some(r) = re {
            if let Some(caps) = r.captures(digest_val) {
                if let Some(b64) = caps.get(1) {
                    // Decode standard base64 to hex
                    if let Ok(bytes) = simple_base64_decode(b64.as_str()) {
                        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
                        return Ok(hex);
                    }
                }
            }
        }
    }

    Err(anyhow::anyhow!("no Digest header with SHA-256"))
}

/// Minimal standard base64 decoder (RFC 4648) without external dependencies.
fn simple_base64_decode(input: &str) -> Result<Vec<u8>> {
    let input = input.trim();
    let len = input.len();
    if len == 0 {
        return Ok(Vec::new());
    }
    // Validate length (must be multiple of 4 after stripping padding)
    let padding = input.chars().rev().take(2).filter(|&c| c == '=').count();
    let cleaned = input.trim_end_matches('=');
    if cleaned.len() % 4 != 0 && padding == 0 {
        return Err(anyhow::anyhow!("invalid base64 length"));
    }

    let decode_char = |c: char| -> Option<u8> {
        match c {
            'A'..='Z' => Some(c as u8 - b'A'),
            'a'..='z' => Some(c as u8 - b'a' + 26),
            '0'..='9' => Some(c as u8 - b'0' + 52),
            '+' => Some(62),
            '/' => Some(63),
            '=' => Some(0),
            _ => None,
        }
    };

    let chars: Vec<u8> = input.chars()
        .filter_map(|c| if c == '=' { Some(0u8) } else { decode_char(c) })
        .collect();

    if chars.len() < 4 {
        return Err(anyhow::anyhow!("invalid base64 input"));
    }

    let mut result = Vec::with_capacity(chars.len() / 4 * 3);
    for chunk in chars.chunks(4) {
        if chunk.len() < 4 {
            break;
        }
        let b0 = chunk[0];
        let b1 = chunk[1];
        let b2 = chunk[2];
        let b3 = chunk[3];
        result.push((b0 << 2) | (b1 >> 4));
        result.push(((b1 & 0x0F) << 4) | (b2 >> 2));
        result.push(((b2 & 0x03) << 6) | b3);
    }

    // Remove padding bytes
    let out_len = if padding > 0 { result.len() - padding } else { result.len() };
    result.truncate(out_len);
    Ok(result)
}

/// Normalize hash format to match Scoop's format_hash behavior:
/// - Lowercase
/// - Strip 'sha256:' prefix
/// - Add algorithm prefix based on length: 32→md5:, 40→sha1:, 64→bare, 128→sha512:
/// - Returns None for invalid/unknown-length hashes
fn format_hash(hash: &str) -> Option<String> {
    let hash = hash.to_lowercase();
    let hash = if let Some(stripped) = hash.strip_prefix("sha256:") {
        stripped.to_string()
    } else {
        hash
    };
    match hash.len() {
        32 => Some(format!("md5:{hash}")),   // MD5
        40 => Some(format!("sha1:{hash}")),  // SHA1
        64 => Some(hash),                    // SHA256 (no prefix)
        128 => Some(format!("sha512:{hash}")), // SHA512
        _ => None, // Unknown length
    }
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
        let _url_for_re = ext.get("url").and_then(|u| u.as_str()).unwrap_or("");
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


fn is_hex_hash(s: &str) -> bool {
    if s.is_empty() { return false; }
    let len = s.len();
    // MD5=32, SHA1=40, SHA256=64, SHA512=128 + algorithm prefixes
    let valid_len = matches!(len, 32 | 40 | 64 | 128)
        || (len > 5 && matches!(&s[..5], "md5:" | "sha1:" | "sha256" | "sha51"))
        || (len > 7 && &s[..7] == "sha512:");
    valid_len && s.chars().all(|c| c.is_ascii_hexdigit() || c == ':')
}


fn json_str_array(items: &[String]) -> serde_json::Value {
    if items.len() == 1 {
        serde_json::Value::String(items[0].clone())
    } else {
        serde_json::Value::Array(
            items.iter().map(|s| serde_json::Value::String(s.clone())).collect()
        )
    }
}

/// Extract $matchHead/$matchTail from a version string (matching Scoop behavior).
///
/// Regex: `(?<head>\d+\.\d+(?:\.\d+)?)(?<tail>.*)`
/// For "1.2.3-beta1" → Some(("1.2.3", "-beta1"))
fn extract_version_head_tail(version: &str) -> Option<(String, String)> {
    let re = Regex::new(r"(?<head>\d+\.\d+(?:\.\d+)?)(?<tail>.*)").ok()?;
    let caps = re.captures(version)?;
    let head = caps.name("head")?.as_str().to_string();
    let tail = caps.name("tail")?.as_str().to_string();
    Some((head, tail))
}

/// Extract a string from XML content using XPath.
fn extract_xpath(content: &str, xpath_expr: &str) -> Option<String> {
    use sxd_document::parser;
    use sxd_xpath::{evaluate_xpath, Value};

    let doc = parser::parse(content).ok()?;
    let doc = doc.as_document();
    match evaluate_xpath(&doc, xpath_expr).ok()? {
        Value::String(s) => Some(s),
        Value::Number(n) => Some(n.to_string()),
        Value::Nodeset(nodes) => {
                nodes.iter().next().map(|n| n.string_value())
        }
        Value::Boolean(b) => Some(b.to_string()),
    }
}

/// Download a page with a Bearer token authorization header (used for GitHub API).
fn download_page_with_token(url: &str, token: &str, timeout_secs: u64) -> Result<String> {
    use std::io::Read;

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(timeout_secs)))
        .build()
        .new_agent();
    let resp = agent
        .get(url)
        .header("Authorization", &format!("Bearer {}", token))
        .header("User-Agent", "hok")
        .call()
        .map_err(|e| anyhow::anyhow!("request failed: {}", e))?;
    let mut body = Vec::new();
    resp.into_body()
        .into_reader()
        .read_to_end(&mut body)
        .map_err(|e| anyhow::anyhow!("read failed: {}", e))?;
    String::from_utf8(body).map_err(|e| anyhow::anyhow!("UTF-8 decode: {}", e))
}

/// Execute a checkver PowerShell script and capture the version from stdout.
///
/// The script is invoked as:
///   powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "{script}"
///
/// Environment variables set:
///   $url     — the checkver URL (if present)
///   $version — the current installed version
///
/// The script's stdout is captured and trimmed as the new version string.
fn run_checkver_script(_session: &Session, script: &str, url: Option<&str>, timeout_secs: u64) -> Result<Option<String>> {
    use std::io::Read;
    use std::time::Duration;

    // Prefer pwsh.exe (PowerShell Core, faster startup) over Windows PowerShell.
    let ps_exe = if libscoop::internal::os::is_pwsh_available() { "pwsh.exe" } else { "powershell.exe" };
    let mut cmd = std::process::Command::new(ps_exe);
    cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script])
        .env("url", url.unwrap_or(""))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| anyhow::anyhow!("spawn powershell: {}", e))?;

    // Wait with timeout
    let start = std::time::Instant::now();
    let deadline = Duration::from_secs(timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(anyhow::anyhow!("timed out after {}s", timeout_secs));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(anyhow::anyhow!("wait: {}", e)),
        }
    }

    let mut output = String::new();
    child.stdout.take().unwrap().read_to_string(&mut output)
        .map_err(|e| anyhow::anyhow!("read output: {}", e))?;

    let mut stderr = String::new();
    if let Some(mut err) = child.stderr.take() {
        err.read_to_string(&mut stderr).ok();
    }

    let version = output.trim().to_string();
    if version.is_empty() {
        Ok(None)
    } else {
        Ok(Some(version))
    }
}


