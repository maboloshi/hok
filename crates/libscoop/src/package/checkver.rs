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
//!   features (ThrowError, etc.). UA / Referer / PRIVATE_HOSTS header
//!   injection is provided by `network::download_page`.

use crate::internal::github;
use crate::{
    package::{manifest, manifest_walker},
    Manifest, Session,
};
use regex::Regex;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::error::Fallible as Result;

use checkver_hash::download_and_hash_multi;

#[path = "checkver_hash.rs"]
mod checkver_hash;
#[path = "checkver_url.rs"]
mod checkver_url;

// ---------------------------------------------------------------------------
// TODO(scoop-alignment): Unimplemented Scoop checkver features
// ---------------------------------------------------------------------------
// 1. ThrowError (--throw) — When set, errors are thrown as exceptions instead
//    of just printed to stderr. Currently all errors use output::err().
//    Scoop ref: bin/checkver.ps1 (param $ThrowError, used at line 412-418)
//
// ✅ Resolved gaps (network-layer unification):
//   - User-Agent — network::download_page() now injects UA from session
//     (previously build_agent() in download.rs ignored its `_user_agent` param).
//   - Referer — network::download_page() now sets Referer via strip_filename.
//   - PRIVATE_HOSTS — network::download_page() now applies extra headers
//     matched by host regex (used by checkurls).
//   Scoop refs: bin/checkver.ps1 (lines 117-121, 246, 240-244)
// ---------------------------------------------------------------------------

/// Check manifest for a newer version
#[derive(Debug, Clone)]
pub struct Args {
    /// Bucket directory to scan for manifests
    pub dir: PathBuf,

    /// Specific app(s) to check (supports wildcards, default: all)
    pub app: Vec<String>,

    /// Update manifest with new version and trigger autoupdate
    pub update: bool,

    /// Force update even when version is unchanged (useful for hash updates)
    pub force_update: bool,

    /// Skip manifests that are already up-to-date
    pub skip_updated: bool,

    /// Update manifest to specific version (skip version detection)
    pub version: Option<String>,

    /// Request timeout in seconds
    pub timeout: u64,
}

// ─── Execute ────────────────────────────────────────────────────────────────

/// Severity of a per-manifest checkver message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportSeverity {
    /// Normal status (no message; render the version change directly).
    Ok,
    /// Warning (e.g. script produced no version).
    Warn,
    /// Error (e.g. download or extraction failure).
    Error,
}

/// Result of checking a single manifest.
#[derive(Debug, Clone)]
pub struct CheckverReport {
    /// Package stem (manifest file name without `.json`).
    pub stem: String,
    /// Current version in the manifest.
    pub current: String,
    /// New version found upstream (Some even when unchanged).
    pub new_version: Option<String>,
    /// Whether the manifest was rewritten via autoupdate.
    pub updated: bool,
    /// Whether an autoupdate is available (semantic upgrade over current).
    pub autoupdate_available: bool,
    /// Optional message for the CLI to render (with `severity`).
    pub message: Option<String>,
    /// Message severity (ignored when `message` is None).
    pub severity: ReportSeverity,
}

impl CheckverReport {
    fn new(stem: String, current: String) -> Self {
        CheckverReport {
            stem,
            current,
            new_version: None,
            updated: false,
            autoupdate_available: false,
            message: None,
            severity: ReportSeverity::Ok,
        }
    }
}

pub fn execute(args: Args, session: &Session) -> Result<Vec<CheckverReport>> {
    let dir = &args.dir;

    // Don't use --version with wildcard app pattern
    if args.version.is_some() && args.app[0] == "*" {
        return Err(crate::Error::Custom(
            "checkver: --version cannot be combined with wildcard app pattern".to_string(),
        ));
    }

    // Extract session data needed for concurrent downloads (Session is !Sync)
    let proxy = session.config().proxy().map(|s| s.to_string());
    let gh_token = session.config().gh_token.clone();
    let private_hosts = session.config().private_hosts().map(|hosts| {
        hosts
            .iter()
            .map(|h| (h.match_pattern().to_string(), h.headers().to_string()))
            .collect::<Vec<_>>()
    });
    let user_agent = session
        .user_agent()
        .unwrap_or("Scoop/1.0 (+http://scoop.sh/)")
        .to_string();
    let timeout = args.timeout;

    /// A manifest that needs version checking.
    struct PendingItem {
        stem: String,
        path: PathBuf,
        manifest: Manifest,
        /// Current version string (pre-extracted for comparison)
        current: String,
        /// URL to download (fully resolved with homepage fallback, GitHub API transform, etc.)
        url: String,
        /// Whether this is a GitHub checkver (for v/V prefix stripping)
        github_mode: bool,
        /// JSONPath override for version extraction (e.g. "$.tag_name" for GitHub API)
        effective_jsonpath: Option<String>,
        /// Regex override for version extraction (e.g. sourceforge default)
        effective_regex: Option<String>,
        /// The checkver configuration (cloned for extraction)
        cv: crate::Checkver,
        /// Script text to execute instead of downloading (script output overrides page content)
        script_text: Option<String>,
    }

    let mut pending: Vec<PendingItem> = Vec::new();
    let mut reports: Vec<CheckverReport> = Vec::new();

    for (path, stem) in manifest_walker::discover_matching(dir, &args.app)? {
        let manifest = match Manifest::parse(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let cv = match manifest.checkver() {
            Some(c) => c.clone(),
            None => continue,
        };

        let current = manifest.version().to_string();

        // When --version is specified, skip all detection and use that version directly
        if let Some(ref ver_override) = args.version {
            let mut report = CheckverReport::new(stem, current);
            report.new_version = Some(ver_override.clone());
            if args.skip_updated && ver_override == &report.current {
                continue;
            }
            if args.update || args.force_update {
                let captures = vec![ver_override.clone()];
                match apply_autoupdate(
                    session,
                    &path,
                    &manifest,
                    ver_override,
                    &captures,
                    &HashMap::new(),
                ) {
                    Ok(()) => report.updated = true,
                    Err(e) => {
                        report.message =
                            Some(rust_i18n::t!("cmd.checkver_update_failed", e = e).to_string());
                        report.severity = ReportSeverity::Error;
                    }
                }
            }
            reports.push(report);
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
            let extracted = github::extract_sourceforge_project(manifest.homepage());
            let project = sf.project.as_deref().or(extracted.as_deref());
            match project {
                Some(proj) => {
                    if cv.regex.is_none() && cv.jsonpath.is_none() {
                        effective_regex = Some(r"/([\d.]+)/".to_string());
                    }
                    format!(
                        "https://sourceforge.net/projects/{}/rss?path=/{}",
                        proj, sf.path
                    )
                }
                None => {
                    let mut report = CheckverReport::new(stem, current);
                    report.message =
                        Some(rust_i18n::t!("cmd.checkver_sourceforge_err").to_string());
                    report.severity = ReportSeverity::Error;
                    reports.push(report);
                    continue;
                }
            }
        } else if github_mode {
            // No cv.url set but github detected: extract repo from homepage
            match github::github_api_url(manifest.homepage()) {
                Some(api_url) => api_url,
                None => {
                    let mut report = CheckverReport::new(stem, current);
                    report.message = Some(rust_i18n::t!("cmd.checkver_github_err").to_string());
                    report.severity = ReportSeverity::Error;
                    reports.push(report);
                    continue;
                }
            }
        } else {
            // Homepage fallback when checkver.url is absent (Scoop L125-129)
            let hp = manifest.homepage().to_string();
            if hp.is_empty() {
                let mut report = CheckverReport::new(stem, current);
                report.message = Some(rust_i18n::t!("cmd.checkver_no_url").to_string());
                report.severity = ReportSeverity::Error;
                reports.push(report);
                continue;
            }
            hp
        };

        // Transform github.com releases URLs to API URLs (Scoop's useGithubAPI behavior)
        if github_mode && url.contains("github.com/") && !url.contains("api.github.com") {
            if let Some(api_url) = github::github_api_url(&url) {
                url = api_url;
            }
        }

        // Automatically add `$.tag_name` JSONPath for GitHub API responses
        let mut effective_jsonpath = cv.jsonpath.clone();
        if effective_jsonpath.is_none() && url.contains("api.github.com") {
            effective_jsonpath = Some("$.tag_name".to_string());
        }

        let script_text = cv.script.as_ref().map(|s| s.devectorize().join("\r\n"));

        pending.push(PendingItem {
            stem,
            path,
            manifest,
            current,
            url,
            github_mode,
            effective_jsonpath,
            effective_regex,
            cv,
            script_text,
        });
    }

    // ── Phase 2: Download all URLs concurrently ──────────────────────────
    // (matching Scoop's async downloads in checkver.ps1 lines 110-248)
    // Script-based items are skipped here (their output is produced below).
    // Session data was extracted before the scope because Session is !Sync.
    let proxy_ref: Option<&str> = proxy.as_deref();
    let ua_ref: &str = &user_agent;
    let hosts_ref = &private_hosts;
    let download_results: Vec<Option<std::result::Result<String, String>>> =
        std::thread::scope(|s| {
            let mut handles = Vec::with_capacity(pending.len());
            for item in &pending {
                if item.script_text.is_some() {
                    handles.push(None);
                    continue;
                }
                let url = item.url.clone();
                let token = gh_token.clone();
                handles.push(Some(s.spawn(move || {
                    // PRIVATE_HOSTS entries matched against the URL here
                    // (computed inside the closure so the HashMap is owned by the thread).
                    let extra = hosts_ref.as_ref().and_then(|v| {
                        crate::internal::network::match_private_hosts(
                            v.iter().map(|(p, h)| (p.as_str(), h.as_str())),
                            &url,
                        )
                    });
                    let extra_ref = extra.as_ref();
                    let referer = crate::internal::network::strip_filename(&url);
                    let dl_opts = crate::internal::network::RequestOptions {
                        proxy: proxy_ref,
                        timeout_secs: timeout,
                        user_agent: Some(ua_ref),
                        referer: Some(&referer),
                        cookies: None,
                        token: token.as_deref(),
                        extra_headers: extra_ref,
                    };
                    crate::internal::network::download(&url, &dl_opts)
                        .and_then(|data| String::from_utf8(data).map_err(|e| e.to_string()))
                })));
            }
            handles
                .into_iter()
                .map(|h| {
                    h.map(|h| {
                        h.join()
                            .unwrap_or_else(|_| Err("thread panicked".to_string()))
                    })
                })
                .collect()
        });

    // ── Phase 3: Process each result sequentially ────────────────────────
    // (matching Scoop's event loop in checkver.ps1 lines 257-419)
    for (item, dl_result) in pending.into_iter().zip(download_results) {
        let PendingItem {
            stem,
            path,
            manifest,
            current,
            url: _,
            github_mode,
            effective_jsonpath,
            effective_regex,
            cv,
            script_text,
        } = item;

        let mut report = CheckverReport::new(stem, current);

        // Get the downloaded page content (or error out, matching Scoop's error handling)
        let mut raw = match dl_result {
            Some(Ok(content)) => content,
            Some(Err(e)) => {
                report.message = Some(rust_i18n::t!("cmd.err_download", e = e).to_string());
                report.severity = ReportSeverity::Error;
                reports.push(report);
                continue;
            }
            None => String::new(),
        };

        // Script output overrides the downloaded page
        // (matching Scoop checkver.ps1 lines 298-301)
        if let Some(ref script) = script_text {
            match run_checkver_script(session, script, cv.url.as_deref(), timeout) {
                Ok(Some(page)) => {
                    raw = page;
                }
                Ok(None) => {
                    report.message = Some(rust_i18n::t!("cmd.checkver_no_version").to_string());
                    report.severity = ReportSeverity::Warn;
                    reports.push(report);
                    continue;
                }
                Err(e) => {
                    report.message =
                        Some(rust_i18n::t!("cmd.checkver_script_err", e = e).to_string());
                    report.severity = ReportSeverity::Error;
                    reports.push(report);
                    continue;
                }
            }
        }

        // Extract version
        let extract_result = extract_version(
            &raw,
            &cv,
            effective_jsonpath.as_deref(),
            effective_regex.as_deref(),
        );

        // Auto-strip leading v/V prefix only for GitHub API JSONPath results
        // (matching Scoop checkver.ps1 lines 219-224 — targeted, not global)
        let (mut ver, captures, named_captures) = match extract_result {
            Some((ref ver, ref caps, ref named)) => (ver.clone(), caps.clone(), named.clone()),
            None => {
                report.message = Some(rust_i18n::t!("cmd.checkver_no_extract").to_string());
                report.severity = ReportSeverity::Error;
                reports.push(report);
                continue;
            }
        };

        if github_mode && cv.jsonpath.is_none() && cv.replace.is_none() {
            ver = ver.trim_start_matches(['v', 'V']).to_string();
        }

        report.new_version = Some(ver.clone());
        report.autoupdate_available = manifest.autoupdate().is_some()
            && ver != report.current
            && crate::compare_versions(&ver, &report.current) == std::cmp::Ordering::Greater;

        // ForceUpdate implies Update (matching Scoop behavior)
        let do_update = args.update || args.force_update;

        if ver == report.current {
            // SkipUpdated: skip display (and update) for up-to-date manifests
            if args.skip_updated && !args.force_update {
                continue;
            }
            if args.force_update {
                match apply_autoupdate(session, &path, &manifest, &ver, &captures, &named_captures)
                {
                    Ok(()) => report.updated = true,
                    Err(e) => {
                        report.message =
                            Some(rust_i18n::t!("cmd.checkver_update_failed", e = e).to_string());
                        report.severity = ReportSeverity::Error;
                    }
                }
            }
        } else if do_update {
            match apply_autoupdate(session, &path, &manifest, &ver, &captures, &named_captures) {
                Ok(()) => report.updated = true,
                Err(e) => {
                    report.message =
                        Some(rust_i18n::t!("cmd.checkver_update_failed", e = e).to_string());
                    report.severity = ReportSeverity::Error;
                }
            }
        }
        reports.push(report);
    }

    Ok(reports)
}

// ─── URL helpers ────────────────────────────────────────────────────────────

fn is_github_checkver(cv: &crate::Checkver) -> bool {
    // Check by regex pattern (set by deserializer for checkver: "github" / checkver.github: "owner/repo")
    if cv
        .regex
        .as_deref()
        .is_some_and(|r| r.contains("/releases/tag/"))
    {
        return true;
    }
    // Check by URL pattern (for cases where cv.url is set to a github releases URL)
    if cv
        .url
        .as_deref()
        .is_some_and(|u| u.contains("github.com/") && u.contains("/releases/"))
    {
        return true;
    }
    false
}

// ─── Version extraction ─────────────────────────────────────────────────────

/// Extract version + capture groups from page content.
///
/// Returns `(version, numbered_captures, named_captures)` where `named_captures`
/// maps named group names to their values (Scoop `$matchesHashtable`, checkver.ps1
/// lines 361-362).
fn extract_version(
    content: &str,
    cv: &crate::Checkver,
    jsonpath_override: Option<&str>,
    regex_override: Option<&str>,
) -> Option<(String, Vec<String>, HashMap<String, String>)> {
    // JSONPath: use override first (for GitHub API), then cv.jsonpath
    if let Some(jp) = jsonpath_override.or(cv.jsonpath.as_deref()) {
        use jsonpath_rust::JsonPath;
        let value: serde_json::Value = serde_json::from_str(content).ok()?;
        let found = value.query(jp).ok()?;
        let ver = found.first()?.as_str()?;
        if !ver.is_empty() {
            let caps = vec![ver.to_string()];
            let v = apply_replace(&caps, cv.replace.as_deref());
            return Some((v, caps, HashMap::new()));
        }
    }

    // XPath: evaluate XPath expression on XML content
    if let Some(xp) = &cv.xpath {
        if let Some(ver) = extract_xpath(content, xp) {
            let caps = vec![ver.clone()];
            let v = apply_replace(&caps, cv.replace.as_deref());
            return Some((v, caps, HashMap::new()));
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
            let captures: Vec<String> = caps
                .iter()
                .map(|m| m.map(|s| s.as_str().to_string()).unwrap_or_default())
                .collect();
            let named = extract_named_captures(&re, caps);
            let ver = apply_replace(&captures, cv.replace.as_deref());
            return Some((ver, captures, named));
        }

        let caps = re.captures(content)?;
        let captures: Vec<String> = caps
            .iter()
            .map(|m| m.map(|s| s.as_str().to_string()).unwrap_or_default())
            .collect();
        let named = extract_named_captures(&re, &caps);
        let ver = apply_replace(&captures, cv.replace.as_deref());
        return Some((ver, captures, named));
    }

    let trimmed = content.trim();
    if !trimmed.is_empty() {
        let ver = apply_replace(&[trimmed.to_string()], cv.replace.as_deref());
        Some((ver, vec![trimmed.to_string()], HashMap::new()))
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
        None => captures
            .get(1)
            .or(captures.first())
            .cloned()
            .unwrap_or_default(),
    }
}

/// Extract named capture groups from a regex match.
///
/// Returns a map of group_name → value for all named groups (excluding numbered
/// groups). This matches Scoop's `$matchesHashtable` behavior (checkver.ps1
/// lines 361-362).
fn extract_named_captures(re: &Regex, caps: &regex::Captures<'_>) -> HashMap<String, String> {
    let mut named = HashMap::new();
    for name in re.capture_names().flatten() {
        if let Some(m) = caps.name(name) {
            named.insert(name.to_string(), m.as_str().to_string());
        }
    }
    named
}

// ─── Autoupdate ────────────────────────────────────────────────────────────

/// Apply autoupdate: substitute variables, download files, compute/extract
/// hashes, write updated manifest.
fn apply_autoupdate(
    session: &Session,
    path: &PathBuf,
    manifest: &Manifest,
    new_version: &str,
    captures: &[String],
    named_captures: &HashMap<String, String>,
) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let mut root: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| crate::Error::Custom(format!("parse: {}", e)))?;

    // Update version
    root["version"] = serde_json::Value::String(new_version.to_string());

    // Build variable substitution (matching Scoop's Get-VersionSubstitution)
    let first_part = new_version.split('-').next().unwrap_or(new_version);
    let last_part = new_version.rsplit('-').next().unwrap_or(new_version);
    let version_parts: Vec<&str> = first_part.split('.').collect();
    let mut vars: Vec<(String, String)> = vec![
        ("$version".to_string(), new_version.to_string()),
        (
            "$dotVersion".to_string(),
            new_version.replace(['_', '-', '.'], "."),
        ),
        (
            "$underscoreVersion".to_string(),
            new_version.replace(['_', '-', '.'], "_"),
        ),
        (
            "$dashVersion".to_string(),
            new_version.replace(['_', '-', '.'], "-"),
        ),
        (
            "$cleanVersion".to_string(),
            new_version.replace(['_', '-', '.'], ""),
        ),
        (
            "$majorVersion".to_string(),
            version_parts.first().copied().unwrap_or("0").to_string(),
        ),
        (
            "$minorVersion".to_string(),
            version_parts.get(1).copied().unwrap_or("0").to_string(),
        ),
        (
            "$patchVersion".to_string(),
            version_parts.get(2).copied().unwrap_or("0").to_string(),
        ),
        (
            "$buildVersion".to_string(),
            version_parts.get(3).copied().unwrap_or("0").to_string(),
        ),
        ("$preReleaseVersion".to_string(), last_part.to_string()),
    ];
    for (i, cap) in captures.iter().enumerate().skip(1) {
        vars.push((format!("$match{}", i), cap.clone()));
    }
    // Named capture groups (Scoop $matchesHashtable, checkver.ps1 lines 361-362)
    // Named groups like `(?<version>...)` create `$version` variables
    for (name, val) in named_captures {
        vars.push((format!("${}", name), val.clone()));
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
        None => {
            crate::internal::fs::write_json(path, &root)?;
            return Ok(());
        }
    };

    let tmp_dir = std::env::temp_dir().join("hok-autoupdate");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir)?;

    // ── Compute $basename from first URL after initial substitution ─────────
    if let Some(urls) = &au.url {
        let first_url = sub_first(urls.devectorize().first().copied().unwrap_or(""));
        let basename = crate::internal::url::basename(&first_url);
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
        .get("autoupdate")
        .and_then(|au| au.get("hash"))
        .and_then(|h| h.as_array())
        .cloned()
        .unwrap_or_default();
    let arch_hash_extractions: [(&str, Vec<serde_json::Value>); 3] = [
        (
            "32bit",
            root.pointer("/architecture/32bit/autoupdate/hash")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default(),
        ),
        (
            "64bit",
            root.pointer("/architecture/64bit/autoupdate/hash")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default(),
        ),
        (
            "arm64",
            root.pointer("/architecture/arm64/autoupdate/hash")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default(),
        ),
    ];

    // ── Top-level URLs ─────────────────────────────────────────────────────
    if let Some(urls) = &au.url {
        let substituted: Vec<String> = urls.devectorize().iter().map(|u| sub(u)).collect();
        let hashes =
            download_and_hash_multi(session, &substituted, &top_hash_extractions, &tmp_dir)?;

        root["url"] = manifest::json_str_array(&substituted);
        root["hash"] = manifest::json_str_array(&hashes);
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
                let hashes =
                    download_and_hash_multi(session, &substituted, arch_extractions, &tmp_dir)?;

                let ptr = format!("/architecture/{}", arch_name);

                if let Some(obj) = root.pointer_mut(&ptr) {
                    obj["url"] = manifest::json_str_array(&substituted);
                    obj["hash"] = manifest::json_str_array(&hashes);
                }
            }
        }
    }

    // ── extract_dir ────────────────────────────────────────────────────────
    if let Some(dirs) = &au.extract_dir {
        let substituted: Vec<String> = dirs.devectorize().iter().map(|d| sub(d)).collect();
        root["extract_dir"] = manifest::json_str_array(&substituted);
    }

    let _ = std::fs::remove_dir_all(&tmp_dir);

    // Write the complete updated JSON (preserving order, 4-space indentation)
    crate::internal::fs::write_json(path, &root)?;
    Ok(())
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
        Value::Nodeset(nodes) => nodes.iter().next().map(|n| n.string_value()),
        Value::Boolean(b) => Some(b.to_string()),
    }
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
fn run_checkver_script(
    _session: &Session,
    script: &str,
    url: Option<&str>,
    timeout_secs: u64,
) -> Result<Option<String>> {
    use std::io::Read;
    use std::time::Duration;

    let mut cmd = crate::internal::os::ps_command();
    cmd.arg("-Command")
        .arg(script)
        .env("url", url.unwrap_or(""))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| crate::Error::Custom(format!("spawn powershell: {}", e)))?;

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
                    return Err(crate::Error::Custom(format!(
                        "timed out after {}s",
                        timeout_secs
                    )));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(crate::Error::Custom(format!("wait: {}", e))),
        }
    }

    let mut output = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .map_err(|e| crate::Error::Custom(format!("read output: {}", e)))?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Checkver;

    fn checkver_default() -> Checkver {
        Checkver {
            regex: None,
            url: None,
            jsonpath: None,
            xpath: None,
            reverse: None,
            replace: None,
            useragent: None,
            script: None,
            sourceforge: None,
        }
    }

    // ── apply_replace ────────────────────────────────────────────────────────

    #[test]
    fn apply_replace_no_pattern_returns_first_capture() {
        let caps = vec!["full_match".to_string(), "1.2.3".to_string()];
        assert_eq!(apply_replace(&caps, None), "1.2.3");
    }

    #[test]
    fn apply_replace_no_pattern_falls_back_to_zeroth_when_no_groups() {
        let caps = vec!["1.2.3".to_string()];
        assert_eq!(apply_replace(&caps, None), "1.2.3");
    }

    #[test]
    fn apply_replace_empty_captures_returns_empty() {
        let caps: Vec<String> = vec![];
        assert_eq!(apply_replace(&caps, None), "");
    }

    #[test]
    fn apply_replace_pattern_substitutes_dollar_index() {
        let caps = vec!["full".to_string(), "1".to_string(), "2".to_string()];
        assert_eq!(apply_replace(&caps, Some("$1.$2")), "1.2");
    }

    #[test]
    fn apply_replace_pattern_substitutes_zero() {
        let caps = vec!["fullmatch".to_string()];
        assert_eq!(apply_replace(&caps, Some("v$0")), "vfullmatch");
    }

    // ── is_github_checkver ───────────────────────────────────────────────────

    #[test]
    fn is_github_regex_match() {
        let cv = Checkver {
            regex: Some(r"/releases/tag/v?([\d.]+)".to_string()),
            ..checkver_default()
        };
        assert!(is_github_checkver(&cv));
    }

    #[test]
    fn is_github_url_match() {
        let cv = Checkver {
            url: Some("https://github.com/owner/repo/releases/latest".to_string()),
            ..checkver_default()
        };
        assert!(is_github_checkver(&cv));
    }

    #[test]
    fn is_github_false_when_unrelated() {
        let cv = Checkver {
            url: Some("https://example.com/latest.txt".to_string()),
            ..checkver_default()
        };
        assert!(!is_github_checkver(&cv));
    }

    #[test]
    fn is_github_false_when_empty() {
        assert!(!is_github_checkver(&checkver_default()));
    }

    // ── extract_version (regex) ──────────────────────────────────────────────

    #[test]
    fn extract_version_regex_basic() {
        let cv = Checkver {
            regex: Some(r"version=([\d.]+)".to_string()),
            ..checkver_default()
        };
        let content = "some text version=1.2.3 end";
        let result = extract_version(content, &cv, None, None);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "1.2.3");
    }

    #[test]
    fn extract_version_regex_no_match_returns_none_if_content_empty() {
        let cv = Checkver {
            regex: Some(r"version=([\d.]+)".to_string()),
            ..checkver_default()
        };
        let result = extract_version("", &cv, None, None);
        assert!(result.is_none());
    }

    #[test]
    fn extract_version_regex_override_takes_precedence() {
        let cv = Checkver {
            regex: Some(r"v(\d+)".to_string()),
            ..checkver_default()
        };
        // regex_override overrides cv.regex
        let result = extract_version("1.9.9 release", &cv, None, Some(r"(\d+\.\d+\.\d+)"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "1.9.9");
    }

    // ── extract_version (jsonpath) ────────────────────────────────────────────

    #[test]
    fn extract_version_jsonpath_extracts_tag_name() {
        let cv = checkver_default();
        let json = r#"{"tag_name": "v1.5.0", "name": "Release 1.5.0"}"#;
        let result = extract_version(json, &cv, Some("$.tag_name"), None);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "v1.5.0");
    }

    #[test]
    fn extract_version_jsonpath_no_field_returns_none() {
        let cv = checkver_default();
        let json = r#"{"other_key": "value"}"#;
        let result = extract_version(json, &cv, Some("$.tag_name"), None);
        assert!(result.is_none());
    }

    // ── extract_version (fallback trim) ──────────────────────────────────────

    #[test]
    fn extract_version_fallback_returns_trimmed_content() {
        let cv = checkver_default(); // no regex, no jsonpath, no xpath
        let result = extract_version("  1.0.0  ", &cv, None, None);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "1.0.0");
    }

    #[test]
    fn extract_version_empty_content_fallback_returns_none() {
        let cv = checkver_default();
        let result = extract_version("   ", &cv, None, None);
        assert!(result.is_none());
    }

    // ── extract_version_head_tail ────────────────────────────────────────────

    #[test]
    fn head_tail_extracts_semver() {
        let result = extract_version_head_tail("1.2.3-beta1");
        assert!(result.is_some());
        let (head, tail) = result.unwrap();
        assert_eq!(head, "1.2.3");
        assert_eq!(tail, "-beta1");
    }

    #[test]
    fn head_tail_simple_version() {
        let result = extract_version_head_tail("2.0.0");
        assert!(result.is_some());
        let (head, tail) = result.unwrap();
        assert_eq!(head, "2.0.0");
        assert_eq!(tail, "");
    }
}
