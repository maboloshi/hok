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
//! - **Request headers**: UA / Referer / PRIVATE_HOSTS / Authorization are all
//!   injected by the shared `ScoopRequest` entry point under
//!   `HeaderProfile::Checkver`; checkver never assembles headers itself.

use crate::internal::url;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::{
    Manifest, Session,
    network::{HeaderProfile, ScoopRequest},
    package::{manifest, manifest_walker},
};
use regex::Regex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Serial number for the per-run `checkver.script` scratch directory.
static CHECKVER_JSON_SEQ: AtomicU64 = AtomicU64::new(0);

use crate::error::{Error, Fallible};

use checkver_hash::download_and_hash_multi;

#[path = "checkver_hash.rs"]
mod checkver_hash;

/// Options for a checkver run, filled by the CLI layer from its clap options.
///
/// Deliberately *not* a clap struct: the CLI owns argument parsing, this is
/// the plain domain options object (clap additions must be mapped here
/// explicitly, giving compile-time protection against the two drifting
/// apart).
#[derive(Debug, Clone)]
pub struct CheckverOptions {
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

    /// Throw on the first failure instead of reporting it and continuing
    /// (upstream `-ThrowError`).
    pub throw_error: bool,
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

/// Outcome of fetching the check page for a single manifest.
enum CheckverFetch {
    /// Page content ready for version extraction (the downloaded body, or the
    /// output of `checkver.script` when one is configured).
    Page(String),
    /// The download failed but `checkver.script` is configured, so upstream
    /// warns and runs the script anyway (`bin/checkver.ps1:307-321`). The page
    /// is the script's output and `warning` is reported before the version.
    ScriptFallback { warning: String, page: String },
    /// Download failed and no script can supply a page (`$err.message` upstream).
    DownloadError(String),
    /// The checkver script failed. Upstream lets that escape and kill the whole
    /// run; hok reports it per manifest and continues.
    ScriptError(String),
}

/// Upstream's default GitHub regex in HTML mode (`bin/checkver.ps1:190`).
const GITHUB_HTML_DEFAULT_REGEX: &str = "/releases/tag/(?:v|V)?([\\d.-]+)";

/// Upstream's default GitHub regex in API mode (`bin/checkver.ps1:186`).
const GITHUB_API_DEFAULT_REGEX: &str = "(?:v|V)?([\\d.-]+)";

/// Upstream's default GitHub jsonpath in API mode (`bin/checkver.ps1:183`); it
/// is **not** injected when `checkver.script` supplies the page
/// (`bin/checkver.ps1:182`).
const GITHUB_API_DEFAULT_JSONPATH: &str = "$.tag_name";

/// `$githubUrlPattern` (`bin/checkver.ps1:156`) — the `homepage` /
/// `checkver.github` values the GitHub shortcut accepts. A mismatch skips the
/// manifest instead of downloading anything (`bin/checkver.ps1:165-168`).
static GITHUB_URL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https://((www\.)?github\.com/[\w.-]+/[\w.-]+/?|api\.github\.com/repos/[\w.-]+/[\w.-]+/.+)$",
    )
    .unwrap()
});

/// Upstream's default SourceForge regex (`bin/checkver.ps1:169`).
const SOURCEFORGE_DEFAULT_REGEX: &str = "(?!\\.)([\\d.]+)(?<=\\d)";

/// `//(www\.)?github\.com/`, rewritten to `//api.github.com/repos/` when the
/// GitHub API is used with a token (`bin/checkver.ps1:222`).
static GITHUB_WEB_HOST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"//(www\.)?github\.com/").unwrap());

/// SourceForge homepage/project forms (`bin/checkver.ps1:172`).
static SOURCEFORGE_HOMEPAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"//(sourceforge|sf)\.net/projects/(?<project>[^/]+)(/files/(?<path>[^/]+))?|//(?<project2>[^.]+)\.(sourceforge\.(net|io)|sf\.net)",
    )
    .unwrap()
});

pub async fn execute(
    options: CheckverOptions,
    session: &Session,
    on_report: impl FnMut(&CheckverReport),
) -> Fallible<()> {
    let mut on_report = on_report;
    let dir = &options.dir;

    // Don't use --version with wildcard app pattern
    if options.version.is_some() && options.app[0] == "*" {
        return Err(Error::Custom(
            "checkver: --version cannot be combined with wildcard app pattern".to_string(),
        ));
    }

    // Extract session data needed for concurrent downloads (Session is !Sync)
    let proxy = session.config().proxy().map(|s| s.to_string());
    let timeout = options.timeout;

    // Scratch directory for the `$json` handed to `checkver.script`; unique per
    // call (not merely per process, so two concurrent runs cannot delete each
    // other's files) and removed on the way out.
    let json_dir = std::env::temp_dir().join(format!(
        "hok-checkver-{}-{}",
        std::process::id(),
        CHECKVER_JSON_SEQ.fetch_add(1, Ordering::Relaxed)
    ));

    /// A manifest that needs version checking.
    struct PendingItem {
        stem: String,
        path: PathBuf,
        manifest: Manifest,
        /// Current version string (pre-extracted for comparison)
        current: String,
        /// Fully resolved URL (homepage fallback, shortcut override, API rewrite).
        url: String,
        /// The regex the body is matched against (user regex, shortcut default,
        /// or the bare `checkver` string); `None` means "no regex in play".
        regex: Option<String>,
        jsonpath: Option<String>,
        /// The checkver configuration (cloned for extraction).
        cv: crate::Checkver,
        /// Script text to run after the download (its stdout replaces the body).
        script_text: Option<String>,
        /// The manifest as JSON, handed to `checkver.script` as `$json`.
        manifest_json_path: Option<PathBuf>,
    }

    let mut pending: Vec<PendingItem> = Vec::new();

    for (path, stem) in manifest_walker::discover_matching(dir, &options.app)? {
        let manifest = match manifest::parse_file(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let cv = match manifest.checkver() {
            Some(c) => c.clone(),
            None => continue,
        };

        let current = manifest.version().to_string();

        // When --version is specified, skip all detection and use that version directly
        if let Some(ref ver_override) = options.version {
            let mut report = CheckverReport::new(stem, current);
            report.new_version = Some(ver_override.clone());
            if options.skip_updated && ver_override == &report.current {
                continue;
            }
            if options.update || options.force_update {
                let captures = vec![ver_override.clone()];
                match apply_autoupdate(
                    session,
                    &path,
                    &manifest,
                    ver_override,
                    &captures,
                    &HashMap::new(),
                )
                .await
                {
                    Ok(()) => report.updated = true,
                    Err(e) => report.message = Some(update_failed_message(&e)),
                }
            }
            let failed = report.message.is_some() && !report.updated;
            if failed && options.throw_error {
                return Err(Error::Custom(report.message.clone().unwrap_or_default()));
            }
            on_report(&report);
            continue;
        }

        let resolved = resolve_checkver_source(
            &cv,
            &manifest,
            &stem,
            crate::network::github_token(session).is_some(),
        );

        // Upstream `error ...; return`: an invalid `homepage`/`checkver.github`
        // for a GitHub shortcut skips the manifest before any download.
        if let Some(message) = resolved.skip_reason {
            let mut report = CheckverReport::new(stem, current);
            report.message = Some(message);
            report.severity = ReportSeverity::Error;
            on_report(&report);
            continue;
        }

        if resolved.url.is_empty() {
            let mut report = CheckverReport::new(stem, current);
            report.message = Some(hok_i18n::t!("cmd.checkver_no_url").to_string());
            report.severity = ReportSeverity::Error;
            on_report(&report);
            continue;
        }

        // Fall back to the raw file when the parsed manifest cannot be
        // re-serialised (the script's `$json` must always be present).
        let manifest_json = manifest
            .to_pretty_json()
            .or_else(|| std::fs::read_to_string(&path).ok())
            .unwrap_or_default();

        let script_text = cv.script.as_ref().map(|s| s.devectorize().join("\r\n"));

        // `checkver.script` runs in a child PowerShell, where `$json` has to be
        // reconstructed. It is loaded from a file rather than embedded in the
        // command line: Windows caps a command line at ~32k characters and a
        // manifest plus a script can exceed that.
        let manifest_json_path = match &script_text {
            None => None,
            Some(_) => {
                crate::internal::fs::ensure_dir(&json_dir)?;
                let file = json_dir.join(format!("{stem}.json"));
                std::fs::write(&file, &manifest_json)?;
                Some(file)
            }
        };

        pending.push(PendingItem {
            stem,
            path,
            manifest,
            current,
            url: resolved.url,
            regex: resolved.regex,
            jsonpath: resolved.jsonpath,
            cv,
            script_text,
            manifest_json_path,
        });
    }

    // ── Phase 2: fetch all URLs (and run scripts) concurrently ──────────
    // Upstream downloads every URL up front and then waits on the completed
    // events (bin/checkver.ps1:110-248, 255-419).
    let net = session.network();
    let cancel = session.cancellation_token();
    let fetch_results: Vec<(usize, CheckverFetch)> =
        futures::stream::iter(pending.iter().enumerate())
            .map(|(i, item)| {
                let url = item.url.clone();
                let script = item.script_text.clone();
                let manifest_user_agent = checkver_user_agent(&item.cv, &item.current);
                let manifest_json_path = item.manifest_json_path.clone();
                let app = item.stem.clone();
                let expected_ver = item.current.clone();
                let net = net.clone();
                let cancel = cancel.clone();
                let proxy_ref = proxy.as_deref();
                async move {
                    // Scoop's checkver header contract (`bin/checkver.ps1:118-246`)
                    // through the shared entry point: User-Agent (the manifest's
                    // `checkver.useragent` wins), Referer, PRIVATE_HOSTS and the
                    // GitHub token only for `api.github.com` URLs.
                    let request = ScoopRequest::for_url(session, &url, HeaderProfile::Checkver)
                        .with_user_agent(manifest_user_agent.as_deref());
                    // The script in its final form: upstream's `$json`, `$app`
                    // and `$expected_ver` are already in scope there, so hok
                    // reconstructs them as a preamble.
                    let script = script.map(|script| {
                        let preamble = manifest_json_path
                            .as_deref()
                            .map(|p| script_preamble(&app, &expected_ver, p))
                            .unwrap_or_default();
                        format!("{preamble}{script}")
                    });

                    // Upstream downloads first even when `checkver.script` is
                    // set, but a failure only skips the manifest when there is
                    // no script to fall back on (`bin/checkver.ps1:307-321`).
                    let body = match net.download(&url, &request.options(timeout)).await {
                        Ok(bytes) => crate::internal::network::decode_body(&bytes),
                        Err(e) => match script.as_deref() {
                            None => return (i, CheckverFetch::DownloadError(e)),
                            // `Write-Host "... Falling back to checkver.script ..."`
                            // then the page decode is skipped and the script runs.
                            Some(script) => {
                                let warning =
                                    hok_i18n::t!("cmd.checkver_script_fallback", e = e).to_string();
                                return match run_checkver_script(
                                    script,
                                    Some(&url),
                                    proxy_ref,
                                    timeout,
                                    &cancel,
                                )
                                .await
                                {
                                    Ok(page) => (
                                        i,
                                        CheckverFetch::ScriptFallback {
                                            warning,
                                            page: page.unwrap_or_default(),
                                        },
                                    ),
                                    Err(e) => (i, CheckverFetch::ScriptError(e.to_string())),
                                };
                            }
                        },
                    };

                    match script {
                        None => (i, CheckverFetch::Page(body)),
                        Some(script) => match run_checkver_script(
                            &script,
                            Some(&url),
                            proxy_ref,
                            timeout,
                            &cancel,
                        )
                        .await
                        {
                            Ok(Some(page)) => (i, CheckverFetch::Page(page)),
                            // An empty script output is a page with nothing
                            // to extract, exactly like an empty body.
                            Ok(None) => (i, CheckverFetch::Page(String::new())),
                            Err(e) => (i, CheckverFetch::ScriptError(e.to_string())),
                        },
                    }
                }
            })
            // The concurrency cap must be at least 1. `buffer_unordered(0)` never
            // polls its source stream — the `in_progress_queue.len() < max` fill
            // loop is skipped and the queue is empty — so the stream is never
            // drained, `is_done()` stays false and the `collect` below parks
            // forever without registering a waker: a silent, permanent hang.
            // An empty `pending` (no manifest declares `checkver`, or `--version`
            // handled every manifest inline) then collects an empty vector and
            // Phase 3 is a no-op.
            .buffer_unordered(pending.len().max(1))
            .collect()
            .await;

    // ── Phase 3: process each result as it completes ────────────────
    for (idx, fetch) in fetch_results {
        let PendingItem {
            stem,
            path,
            manifest,
            current,
            url: _,
            regex,
            jsonpath,
            cv,
            script_text: _,
            manifest_json_path: _,
        } = &pending[idx];

        let mut report = CheckverReport::new(stem.clone(), current.clone());

        let (raw, fallback_warning) = match fetch {
            CheckverFetch::Page(content) => (content, None),
            // `Write-Host "... Falling back to checkver.script ..."`: a warning,
            // not a failure — the script's output is the page.
            CheckverFetch::ScriptFallback { warning, page } => (page, Some(warning)),
            CheckverFetch::DownloadError(e) => {
                report.message = Some(hok_i18n::t!("cmd.err_download", e = e).to_string());
                report.severity = ReportSeverity::Error;
                if options.throw_error {
                    return Err(Error::Custom(report.message.clone().unwrap_or_default()));
                }
                on_report(&report);
                continue;
            }
            CheckverFetch::ScriptError(e) => {
                report.message = Some(hok_i18n::t!("cmd.checkver_script_err", e = e).to_string());
                report.severity = ReportSeverity::Error;
                if options.throw_error {
                    return Err(Error::Custom(report.message.clone().unwrap_or_default()));
                }
                on_report(&report);
                continue;
            }
        };

        if let Some(message) = fallback_warning {
            let mut notice = CheckverReport::new(stem.clone(), current.clone());
            notice.message = Some(message);
            notice.severity = ReportSeverity::Warn;
            on_report(&notice);
        }

        // Extract version
        let subs = version_substitutions(current, &[], &HashMap::new());
        let extract_result =
            extract_version(&raw, cv, jsonpath.as_deref(), regex.as_deref(), &subs);

        let (ver, captures, named_captures) = match extract_result {
            Some((ver, caps, named)) => (ver, caps, named),
            None => {
                report.message = Some(hok_i18n::t!("cmd.checkver_no_extract").to_string());
                report.severity = ReportSeverity::Error;
                if options.throw_error {
                    return Err(Error::Custom(report.message.clone().unwrap_or_default()));
                }
                on_report(&report);
                continue;
            }
        };

        report.new_version = Some(ver.clone());
        report.autoupdate_available = manifest.autoupdate().is_some()
            && ver != report.current
            && crate::compare_versions(&ver, &report.current) == std::cmp::Ordering::Greater;

        // ForceUpdate implies Update (matching Scoop behavior)
        let do_update = options.update || options.force_update;

        if ver == report.current {
            // SkipUpdated: skip display (and update) for up-to-date manifests
            if options.skip_updated && !options.force_update {
                continue;
            }
            if options.force_update {
                match apply_autoupdate(session, path, manifest, &ver, &captures, &named_captures)
                    .await
                {
                    Ok(()) => report.updated = true,
                    Err(e) => {
                        report.message = Some(update_failed_message(&e));
                        report.severity = ReportSeverity::Error;
                    }
                }
            }
        } else if do_update {
            match apply_autoupdate(session, path, manifest, &ver, &captures, &named_captures).await
            {
                Ok(()) => report.updated = true,
                Err(e) => {
                    report.message = Some(update_failed_message(&e));
                    report.severity = ReportSeverity::Error;
                }
            }
        }

        // `catch { if ($ThrowError) { throw $_ } else { error ... } }`
        if options.throw_error
            && let Some(message) = &report.message
        {
            return Err(Error::Custom(message.clone()));
        }
        on_report(&report);
    }

    // Drop the `$json` scratch files created for `checkver.script`.
    let _ = std::fs::remove_dir_all(&json_dir);

    Ok(())
}

fn update_failed_message(e: &dyn std::fmt::Display) -> String {
    hok_i18n::t!("cmd.checkver_update_failed", e = e).to_string()
}

// ─── Source resolution (`bin/checkver.ps1:114-247`) ─────────────────────────

#[derive(Debug)]
struct ResolvedSource {
    url: String,
    regex: Option<String>,
    /// The jsonpath in play: `checkver.jsonpath` (which upstream applies
    /// *after* the GitHub block) or the GitHub API-mode default.
    jsonpath: Option<String>,
    /// `error "$name checkver expects $fieldUsed to be a valid GitHub repository URL"`:
    /// upstream then `return`s, skipping the manifest entirely
    /// (`bin/checkver.ps1:165-168`).
    skip_reason: Option<String>,
}

/// Resolve the URL, regex and jsonpath for one manifest.
///
/// This is upstream's pre-download block (`bin/checkver.ps1:124-244`), in
/// order:
///
/// 1. `$url = $json.checkver.url` else `$json.homepage`;
/// 2. the GitHub shortcut — the string form (`checkver: "github"`, which reads
///    `homepage`) or the map form (`checkver.github`) — validates its input
///    against `$githubUrlPattern` and **skips the manifest** when it does not
///    match, appends `/releases/latest` only when the URL is not already
///    `api.github.com`, rewrites a web URL to the API whenever a token is
///    configured, and then applies the mode-dependent
///    jsonpath/regex defaults;
/// 3. the SourceForge shortcuts;
/// 4. an explicit `checkver.jsonpath` wins over the GitHub default, because
///    upstream applies `jp`/`jsonpath` after the shortcut blocks.
///
/// The `$useGithubAPI` gate upstream carried through 0.5.3 is gone
/// (`327040cf`/`bdf63444`): the token rewrite now depends only on a token being
/// configured, and the API/HTML split is decided by the final URL.
///
/// Everything else about the request — User-Agent (`checkver.useragent`),
/// Referer, PRIVATE_HOSTS and the `Authorization` header — is resolved by the
/// shared [`ScoopRequest`] entry point. The URL rewrite is a *URL* decision the
/// shared entry point cannot make, because the default-jsonpath choice depends
/// on it; `has_github_token` is its only extra input.
fn resolve_checkver_source(
    cv: &crate::Checkver,
    manifest: &Manifest,
    stem: &str,
    has_github_token: bool,
) -> ResolvedSource {
    let homepage = manifest.homepage();

    let mut url = cv.url.clone().unwrap_or_else(|| homepage.to_string());
    let mut regex = cv.regex.clone();
    let mut jsonpath = cv.jsonpath.clone();

    // `if (($json.checkver -eq 'github') -or $json.checkver.github)`
    // (`bin/checkver.ps1:155`).
    if cv.github_homepage || cv.github.is_some() {
        // `$inputGithubUrl` / `$fieldUsed`: `checkver.github` wins over
        // `homepage` (`bin/checkver.ps1:158-163`).
        let (input, field) = match cv.github.as_deref() {
            Some(repo) => (repo, "checkver.github"),
            None => (homepage, "homepage"),
        };

        if !GITHUB_URL_PATTERN.is_match(input) {
            return ResolvedSource {
                url: String::new(),
                regex: None,
                jsonpath: None,
                skip_reason: Some(
                    hok_i18n::t!("cmd.checkver_github_err", name = stem, field = field).to_string(),
                ),
            };
        }

        let mut github_url = input.trim_end_matches('/').to_string();
        // `if ($url -notlike 'https://api.github.com*') { $url += '/releases/latest' }`
        // (`bin/checkver.ps1:171-173`).
        if !is_github_api_mode(&github_url) {
            github_url.push_str("/releases/latest");
        }
        // `if ($GitHubToken) { $url = $url -replace ... }` (`bin/checkver.ps1:175-178`)
        // — not gated on `$useGithubAPI` any more.
        if has_github_token {
            github_url = GITHUB_WEB_HOST
                .replace_all(&github_url, "//api.github.com/repos/")
                .into_owned();
        }
        url = github_url;

        if is_github_api_mode(&url) {
            // Default jsonpath/regex in GitHub API mode
            // (`bin/checkver.ps1:180-187`). The jsonpath default is skipped
            // when the page comes from `checkver.script`.
            if cv.script.is_none() && jsonpath.is_none() {
                jsonpath = Some(GITHUB_API_DEFAULT_JSONPATH.to_string());
            }
            if regex.is_none() {
                regex = Some(GITHUB_API_DEFAULT_REGEX.to_string());
            }
        } else if regex.is_none() {
            // Default regex in GitHub HTML mode (`bin/checkver.ps1:188-191`).
            regex = Some(GITHUB_HTML_DEFAULT_REGEX.to_string());
        }
    }

    let sourceforge_regex = regex
        .clone()
        .unwrap_or_else(|| SOURCEFORGE_DEFAULT_REGEX.to_string());
    if cv.sourceforge_homepage {
        let (project, path) = sourceforge_project_from_homepage(homepage)
            .unwrap_or_else(|| (crate::internal::url::strip_ext(stem), None));
        url = sourceforge_rss(&project, path.as_deref());
        regex = Some(sourceforge_cdata_regex(path.as_deref(), &sourceforge_regex));
    }
    if let Some(sf) = &cv.sourceforge {
        url = sourceforge_rss(sf.project.as_deref().unwrap_or(""), sf.path.as_deref());
        regex = Some(sourceforge_cdata_regex(
            sf.path.as_deref(),
            &sourceforge_regex,
        ));
    }

    ResolvedSource {
        url,
        regex,
        jsonpath,
        skip_reason: None,
    }
}

/// `$url -like 'https://api.github.com*'`, and the `-notlike` that guards the
/// `/releases/latest` suffix (`bin/checkver.ps1:171-180`). PowerShell's `-like`
/// is case-insensitive, so this comparison is too.
fn is_github_api_mode(url: &str) -> bool {
    url.to_ascii_lowercase()
        .starts_with("https://api.github.com")
}

/// `substitute $json.checkver.useragent $substitutions` (`bin/checkver.ps1:118`).
///
/// The manifest's User-Agent is version-substituted before it reaches the
/// request layer, which applies it via `ScoopRequest::with_user_agent`. hok
/// leaves the session default in place when the manifest sets no User-Agent.
fn checkver_user_agent(cv: &crate::Checkver, version: &str) -> Option<String> {
    cv.useragent
        .as_deref()
        .map(|ua| version_substitutions(version, &[], &HashMap::new()).apply(ua))
}

/// `if ($json.homepage -match '//(sourceforge|sf)\.net/projects/...')`
/// (bin/checkver.ps1:172).
fn sourceforge_project_from_homepage(homepage: &str) -> Option<(String, Option<String>)> {
    let caps = SOURCEFORGE_HOMEPAGE.captures(homepage)?;
    let project = caps
        .name("project")
        .or_else(|| caps.name("project2"))?
        .as_str()
        .to_string();
    let path = caps.name("path").map(|m| m.as_str().to_string());
    Some((project, path))
}

fn sourceforge_rss(project: &str, path: Option<&str>) -> String {
    let mut url = format!("https://sourceforge.net/projects/{project}/rss");
    if let Some(path) = path.filter(|p| !p.is_empty()) {
        url.push_str(&format!("?path=/{}", path.trim_start_matches('/')));
    }
    url
}

/// `"CDATA\[/$path/.*?$regex.*?\]".Replace('//', '/')` (bin/checkver.ps1:182).
fn sourceforge_cdata_regex(path: Option<&str>, regex: &str) -> String {
    let path = path.unwrap_or("");
    format!(r"CDATA\[/{path}/.*?{regex}.*?\]").replace("//", "/")
}

/// `$script -join "`r`n"` runs in-process upstream, where `$json`, `$app` and
/// `$expected_ver` are already in scope. hok runs the script in a child
/// PowerShell, so those variables are reconstructed as a preamble; `$json` is
/// read from `manifest_json_path` so a large manifest cannot overflow the
/// command line.
fn script_preamble(app: &str, expected_ver: &str, manifest_json_path: &std::path::Path) -> String {
    let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    format!(
        "$json = Get-Content -LiteralPath {} -Raw | ConvertFrom-Json\r\n$app = {}\r\n$expected_ver = {}\r\n",
        quote(&manifest_json_path.display().to_string()),
        quote(app),
        quote(expected_ver)
    )
}

// ─── Version extraction ─────────────────────────────────────────────────────

/// Extract version + capture groups from page content.
///
/// Returns `(version, numbered_captures, named_captures)` where `named_captures`
/// maps named group names to their values (Scoop `$matchesHashtable`, checkver.ps1
/// lines 361-362).
///
/// `subs` carries the version substitution table, which upstream applies to the
/// jsonpath (`json_path`, lib/json.ps1:98) and to the legacy jsonpath walk.
fn extract_version(
    content: &str,
    cv: &crate::Checkver,
    jsonpath: Option<&str>,
    regex: Option<&str>,
    subs: &Substitution,
) -> Option<(String, Vec<String>, HashMap<String, String>)> {
    let replace = cv.replace.as_deref();
    let reverse = cv.reverse.unwrap_or(false);
    // `$noregex = [String]::IsNullOrEmpty($regexp)`
    let noregex = regex.is_none();

    // JSONPath, with upstream's legacy fallback (bin/checkver.ps1:303-318).
    if let Some(jsonpath) = jsonpath {
        // `json_path $page $jsonpath $null ($reverse -and $noregex) $noregex`
        let mut ver =
            json_path(content, jsonpath, reverse && noregex, noregex, subs).unwrap_or_default();
        if ver.is_empty() {
            ver = json_path_legacy(content, jsonpath, subs).unwrap_or_default();
        }
        if ver.is_empty() {
            return None;
        }
        return finish_extraction(&ver, regex, reverse, replace);
    }

    // XPath (`bin/checkver.ps1:320-340`).
    if let Some(xpath) = &cv.xpath {
        let ver = extract_xpath(content, xpath)?;
        if ver.is_empty() {
            return None;
        }
        return finish_extraction(&ver, regex, reverse, replace);
    }

    if let Some(regex) = regex {
        return match_regex(content, regex, reverse, replace);
    }

    // No selector at all: hok's long-standing fallback is the trimmed body.
    // Upstream would try to compile the checkver object as a regex and abort.
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    let ver = apply_replace(&[trimmed.to_string()], &HashMap::new(), replace);
    Some((ver, vec![trimmed.to_string()], HashMap::new()))
}

/// The tail shared by the jsonpath/xpath branches: when a regex is also in
/// play the selected value becomes the page and the regex runs on it
/// (`if ($jsonpath -and $regexp) { $page = $ver; $ver = '' }`). A regex that
/// does not match then fails the whole extraction, exactly like upstream's
/// `next "couldn't match ..."`.
fn finish_extraction(
    value: &str,
    regex: Option<&str>,
    reverse: bool,
    replace: Option<&str>,
) -> Option<(String, Vec<String>, HashMap<String, String>)> {
    if let Some(regex) = regex {
        return match_regex(value, regex, reverse, replace);
    }
    let captures = vec![value.to_string()];
    let ver = apply_replace(&captures, &HashMap::new(), replace);
    Some((ver, captures, HashMap::new()))
}

/// `json_path` (lib/json.ps1:95-127) with the substitutions the caller passes.
///
/// `single` is upstream's `$noregex`: with a regex in play several tokens are
/// serialised as a JSON array for the regex to run against; otherwise the
/// first token is stringified.
fn json_path(
    content: &str,
    jsonpath: &str,
    reverse: bool,
    single: bool,
    subs: &Substitution,
) -> Option<String> {
    use jsonpath_rust::JsonPath;

    // `substitute $jsonpath $substitutions ($jsonpath -like "*=~*")`
    let jsonpath = if jsonpath.contains("=~") {
        subs.apply_escaped(jsonpath)
    } else {
        subs.apply(jsonpath)
    };
    let value: serde_json::Value = serde_json::from_str(content).ok()?;
    // jsonpath-rust cannot parse unquoted field names with special characters
    // (e.g. `$.dist-tags.latest`); normalize to bracket form.
    let jsonpath = crate::internal::string::normalize_jsonpath(&jsonpath);
    let found = value.query(&jsonpath).ok()?;
    let mut items: Vec<serde_json::Value> = found.iter().map(|v| (*v).clone()).collect();
    if reverse {
        items.reverse();
    }
    if single || items.len() == 1 {
        // `if ([Count] -eq 1 -or $single) { First().ToString() }`; an empty
        // selection throws upstream and yields `$null`.
        items.first().map(json_value_text)
    } else if items.is_empty() {
        None
    } else {
        Some(serde_json::Value::Array(items).to_string())
    }
}

/// `json_path_legacy` (lib/json.ps1:129-159): a dotted property walk, used when
/// the real JSONPath query returns nothing. Real buckets rely on it — 40
/// manifests use a bare `assets.browser_download_url` / `tag_name`.
fn json_path_legacy(content: &str, jsonpath: &str, subs: &Substitution) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(content).ok()?;
    let is_jsonpath = jsonpath.starts_with('$');
    let mut current = value;
    for segment in jsonpath.split('.') {
        let segment = subs.apply(segment);
        // `if ($el -eq '$' -and $isJsonPath) { return }`
        if segment == "$" && is_jsonpath {
            continue;
        }
        // `if ($el -match '^(?<property>\w+)?\[(?<index>\d+)\]$')`
        if let Some(caps) = LEGACY_INDEX.captures(&segment) {
            let index: usize = caps.name("index")?.as_str().parse().ok()?;
            current = match caps.name("property") {
                Some(prop) => current.get(prop.as_str())?.as_array()?.get(index)?.clone(),
                None => current.as_array()?.get(index)?.clone(),
            };
            continue;
        }
        current = current.get(segment.as_str())?.clone();
    }
    Some(json_value_text(&current))
}

/// `^(?<property>\w+)?\[(?<index>\d+)\]$` from the legacy jsonpath walk.
static LEGACY_INDEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?<property>\w+)?\[(?<index>\d+)\]$").unwrap());

/// PowerShell's `.ToString()` on a JSON token.
fn json_value_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Match `regex` against `content` (honoring `reverse`), returning
/// `(version, numbered_captures, named_captures)`. Shared by the regex branch
/// and the jsonpath→regex chaining.
fn match_regex(
    content: &str,
    regex: &str,
    reverse: bool,
    replace: Option<&str>,
) -> Option<(String, Vec<String>, HashMap<String, String>)> {
    let re = Regex::new(regex).ok()?;

    // If reverse is enabled, take the last match
    let caps = if reverse {
        re.captures_iter(content).last()?
    } else {
        re.captures(content)?
    };
    let captures: Vec<String> = caps
        .iter()
        .map(|m| m.map(|s| s.as_str().to_string()).unwrap_or_default())
        .collect();
    let named = extract_named_captures(&re, &caps);

    // Scoop group selection (checkver.ps1 L362-369): group name "1" (the
    // first unnamed group) -> named group `version` -> failure. An empty
    // replace result falls back to `version` the same way (official `if (!$ver)`).
    let ver = match replace {
        Some(_) => {
            let v = apply_replace(&captures, &named, replace);
            if v.is_empty() {
                official_capture_version(&re, &caps)?
            } else {
                v
            }
        }
        None => official_capture_version(&re, &caps)?,
    };
    Some((ver, captures, named))
}

/// Select the version from a regex match following Scoop's group-name
/// semantics (checkver.ps1 lines 362-369).
///
/// Scoop reads `$matchesHashtable['1']` — the first *unnamed* capture group:
/// in .NET, named groups occupy negative numbers while unnamed groups are
/// numbered 1, 2, ... (a `(?<hash>...)` group is therefore NOT group "1").
/// If that is empty it falls back to the named group `version`; if both are
/// empty Scoop reports failure (and so do we — no further fallback).
fn official_capture_version(re: &Regex, caps: &regex::Captures<'_>) -> Option<String> {
    // First unnamed capture group (Scoop's group name "1")
    if let Some(i) = re
        .capture_names()
        .enumerate()
        .skip(1) // skip group 0 (full match)
        .find(|(_, name)| name.is_none())
        .map(|(i, _)| i)
        && let Some(v) = caps.get(i)
    {
        let s = v.as_str();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    // Named group `version`
    if let Some(v) = caps.name("version") {
        let s = v.as_str();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    None
}

/// Apply the `replace` transformation to extracted captures.
///
/// When `replace` is set, `$1`, `$2`, etc. are replaced with the corresponding
/// capture group values, and `${name}` (the .NET `Regex.Replace` named-group
/// syntax) with the matching named capture. `$0` is the full match. If
/// `replace` is not set, returns `captures[1]` (first capture group) or
/// `captures[0]` (full match).
fn apply_replace(
    captures: &[String],
    named: &HashMap<String, String>,
    replace: Option<&str>,
) -> String {
    match replace {
        Some(pattern) => {
            let mut result = pattern.to_string();
            for (i, cap) in captures.iter().enumerate() {
                result = result.replace(&format!("${}", i), cap);
            }
            for (name, cap) in named {
                result = result.replace(&format!("${{{name}}}"), cap);
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
///
/// `captures`/`named_captures` are the version-extraction groups from a
/// checkver run; pass empty slices for a plain `version` substitution, which
/// matches upstream `generate_user_manifest` calling `Invoke-AutoUpdate`
/// with an empty `$CustomMatches`.
pub(crate) async fn apply_autoupdate(
    session: &Session,
    path: &PathBuf,
    manifest: &Manifest,
    new_version: &str,
    captures: &[String],
    named_captures: &HashMap<String, String>,
) -> Fallible<()> {
    let content = std::fs::read_to_string(path)?;
    // Tolerant read, matching the manifest parse that produced `manifest`:
    // the rewrite must not fail on a manifest that installed fine (upstream
    // `Invoke-AutoUpdate` works on the already-parsed `$manifest`).
    let mut root: serde_json::Value = crate::internal::json::parse_str(&content)
        .map_err(|e| Error::Custom(format!("parse: {}", e)))?;

    // Update version
    root["version"] = serde_json::Value::String(new_version.to_string());

    // Build the upstream version-substitution table
    // (`Get-VersionSubstitution`, lib/autoupdate.ps1:429-463).
    let subs = version_substitutions(new_version, captures, named_captures);

    // Official `Invoke-AutoUpdate` only runs when the manifest has
    // `autoupdate`; the CLI never calls us otherwise.
    if manifest.autoupdate().is_none() {
        crate::internal::fs::write_json(path, &root)?;
        return Ok(());
    }

    // Upstream caches the hash fallback download in Scoop's download cache
    // (`Invoke-CachedDownload`, lib/autoupdate.ps1:319 → `cache_path $app $version $url`),
    // so a second run — or a later download/install — reuses it. hok used to
    // drop its own `%TEMP%\hok-autoupdate` copy after every run.
    let cache_dir = session.config().cache_path().to_path_buf();
    crate::internal::fs::ensure_dir(&cache_dir)?;

    let app_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    // ── Properties to update, driven by the autoupdate object itself ────────
    // `Invoke-AutoUpdate` (lib/autoupdate.ps1:483-490) collects the autoupdate
    // object's own property names (minus `architecture`, plus every
    // architecture-level property name) and appends `hash` whenever `url` is
    // present. Driving that list from the raw JSON — instead of a hardcoded
    // set — is exactly what gives upstream its coverage: `changelog`,
    // `extract_to`, `pre_install`, `homepage`, ... are substituted too.
    let au_obj = root
        .get("autoupdate")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let mut props: Vec<String> = Vec::new();
    if let Some(map) = au_obj.as_object() {
        for key in map.keys() {
            if key != "architecture" {
                push_unique(&mut props, key);
            }
        }
    }
    if let Some(arch) = au_obj.get("architecture").and_then(|a| a.as_object()) {
        for spec in arch.values() {
            if let Some(map) = spec.as_object() {
                for key in map.keys() {
                    push_unique(&mut props, key);
                }
            }
        }
    }
    if props.iter().any(|p| p == "url") {
        push_unique(&mut props, "hash");
    }

    for prop in &props {
        if prop == "hash" {
            // `hash` is the one property with a bespoke pipeline
            // (`Update-ManifestProperty` lib/autoupdate.ps1:374-391).
            update_hash_property(
                session,
                &mut root,
                &au_obj,
                &subs,
                &cache_dir,
                &app_name,
                new_version,
            )
            .await?;
        } else {
            update_manifest_property(&mut root, &au_obj, prop, &subs);
        }
    }

    // Write the complete updated JSON (preserving order, 4-space indentation)
    crate::internal::fs::write_json(path, &root)?;
    Ok(())
}

// ─── Version substitution (lib/autoupdate.ps1:429-463 + :208-219) ───────────

/// Append `name` to `props` unless it is already present (upstream's
/// `Select-Object -Unique`, kept in first-seen order).
fn push_unique(props: &mut Vec<String>, name: &str) {
    if !props.iter().any(|p| p == name) {
        props.push(name.to_string());
    }
}

/// `$match<Name>` token lookup — PowerShell's hashtables and its `-match`
/// operator are case-insensitive, so any casing of the token resolves.
static RE_MATCH_VAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\$match([A-Za-z0-9_]+)").unwrap());

/// The version-variable table plus the per-URL additions that
/// `get_hash_for_app` layers on (lib/autoupdate.ps1:208-219).
///
/// Two kinds of token live here:
/// * **exact keys** (`$version`, `$baseurl`, ...), applied longest-key-first
///   so a shorter key can never eat a longer one — upstream's `Hashtable`
///   enumeration order is unspecified, which is why it lets `$url` eat
///   `$urlNoExt` on some manifests;
/// * **`$match<Name>` keys**, resolved case-insensitively (see
///   [`RE_MATCH_VAR`]).
#[derive(Debug, Clone, Default)]
pub(crate) struct Substitution {
    vars: Vec<(String, String)>,
    matches: Vec<(String, String)>,
}

impl Substitution {
    /// Plain substitution — upstream `substitute` with `$regexEscape = $false`
    /// (URLs, jsonpath, xpath, manifest property values).
    pub(crate) fn apply(&self, input: &str) -> String {
        self.apply_with(input, false)
    }

    /// Regex-escaped substitution — upstream `substitute` with
    /// `$regexEscape = $true` (the hash-file regex, so a substituted value
    /// cannot change the pattern's meaning).
    pub(crate) fn apply_escaped(&self, input: &str) -> String {
        self.apply_with(input, true)
    }

    fn apply_with(&self, input: &str, escape: bool) -> String {
        let mut out = input.to_string();
        let mut keys: Vec<&(String, String)> = self.vars.iter().collect();
        keys.sort_by_key(|e| std::cmp::Reverse(e.0.len()));
        for (key, value) in keys {
            if !out.contains(key.as_str()) {
                continue;
            }
            let replacement = if escape {
                regex::escape(value)
            } else {
                value.clone()
            };
            out = out.replace(key.as_str(), &replacement);
        }
        if !self.matches.is_empty() {
            out = RE_MATCH_VAR
                .replace_all(&out, |caps: &regex::Captures<'_>| {
                    let Some(name) = caps.get(1) else {
                        return caps
                            .get(0)
                            .map(|m| m.as_str().to_string())
                            .unwrap_or_default();
                    };
                    match self.lookup_match(name.as_str()) {
                        Some(value) => {
                            if escape {
                                regex::escape(value)
                            } else {
                                value.to_string()
                            }
                        }
                        None => caps
                            .get(0)
                            .map(|m| m.as_str().to_string())
                            .unwrap_or_default(),
                    }
                })
                .into_owned();
        }
        out
    }

    fn lookup_match(&self, name: &str) -> Option<&str> {
        self.matches
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Layer the per-download-URL variables on top of the version table
    /// (`get_hash_for_app`, lib/autoupdate.ps1:212-219).
    ///
    /// Truth values (recorded from upstream, 2026-09-13):
    /// `https://ex.com/dl/app-1.2.3.zip?download=1` →
    /// `$url` = fragment-stripped URL, `$baseurl` = `https://ex.com/dl`,
    /// `$basename` = `app-1.2.3.zip` (URL-decoded), `$urlNoExt` =
    /// `https://ex.com/dl/app-1.2.3`, `$basenameNoExt` = `app-1.2.3`.
    pub(crate) fn with_url(&self, url: &str) -> Substitution {
        let stripped = url::strip_url_fragment(url).to_string();
        let basename = url::decoded(&url::url_remote_filename(&stripped));
        let mut out = self.clone();
        out.vars.push((
            "$baseurl".to_string(),
            url::strip_filename(&stripped)
                .trim_end_matches('/')
                .to_string(),
        ));
        out.vars
            .push(("$urlNoExt".to_string(), url::strip_ext(&stripped)));
        out.vars
            .push(("$basenameNoExt".to_string(), url::strip_ext(&basename)));
        out.vars.push(("$url".to_string(), stripped));
        out.vars.push(("$basename".to_string(), basename));
        out
    }
}

/// Build the upstream `Get-VersionSubstitution` table
/// (lib/autoupdate.ps1:429-463): the ten version variables, `$matchHead`/
/// `$matchTail` derived from the version string itself, then one
/// `$match<Name>` per capture group (`$match1`, `$match2`, ... for numbered
/// groups; `$match<ToTitleCase(name)>` for named ones).
fn version_substitutions(
    version: &str,
    captures: &[String],
    named_captures: &HashMap<String, String>,
) -> Substitution {
    let first_part = version.split('-').next().unwrap_or(version);
    let last_part = version.rsplit('-').next().unwrap_or(version);
    let parts: Vec<&str> = first_part.split('.').collect();
    let vars: Vec<(String, String)> = vec![
        ("$version".to_string(), version.to_string()),
        (
            "$dotVersion".to_string(),
            version.replace(['_', '-', '.'], "."),
        ),
        (
            "$underscoreVersion".to_string(),
            version.replace(['_', '-', '.'], "_"),
        ),
        (
            "$dashVersion".to_string(),
            version.replace(['_', '-', '.'], "-"),
        ),
        (
            "$cleanVersion".to_string(),
            version.replace(['_', '-', '.'], ""),
        ),
        (
            "$majorVersion".to_string(),
            parts.first().copied().unwrap_or("0").to_string(),
        ),
        (
            "$minorVersion".to_string(),
            parts.get(1).copied().unwrap_or("0").to_string(),
        ),
        (
            "$patchVersion".to_string(),
            parts.get(2).copied().unwrap_or("0").to_string(),
        ),
        (
            "$buildVersion".to_string(),
            parts.get(3).copied().unwrap_or("0").to_string(),
        ),
        ("$preReleaseVersion".to_string(), last_part.to_string()),
    ];

    let mut matches: Vec<(String, String)> = Vec::new();
    if let Some((head, tail)) = extract_version_head_tail(version) {
        matches.push(("Head".to_string(), head));
        matches.push(("Tail".to_string(), tail));
    }
    // Numbered groups: upstream's `$matchesHashtable` carries "0", "1", ...
    // and `Get-VersionSubstitution` skips only group "0".
    for (i, capture) in captures.iter().enumerate().skip(1) {
        matches.push((i.to_string(), capture.clone()));
    }
    // Named groups: `'$match' + ToTitleCase($_.Name)`.
    for (name, value) in named_captures {
        matches.push((to_title_case(name), value.clone()));
    }

    Substitution { vars, matches }
}

/// `.NET TextInfo.ToTitleCase` for an identifier: the first letter of every
/// word upper-cased and the rest lower-cased, **except** words that are
/// already entirely upper-case, which are returned unchanged.
///
/// Verified against PowerShell 5.1 on 2026-09-13:
/// `gccversion`→`Gccversion`, `GccVersion`→`Gccversion`, `myGroup`→`Mygroup`,
/// `tag`→`Tag`, `AB`→`AB`.
fn to_title_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut word = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            word.push(c);
        } else {
            out.push_str(&title_case_word(&word));
            word.clear();
            out.push(c);
        }
    }
    out.push_str(&title_case_word(&word));
    out
}

fn title_case_word(word: &str) -> String {
    if word.is_empty() {
        return String::new();
    }
    let has_lower = word.chars().any(|c| c.is_lowercase());
    let has_upper = word.chars().any(|c| c.is_uppercase());
    if has_upper && !has_lower {
        return word.to_string();
    }
    let mut out = String::with_capacity(word.len());
    let mut chars = word.chars();
    if let Some(first) = chars.next() {
        out.extend(first.to_uppercase());
    }
    out.extend(chars.flat_map(|c| c.to_lowercase()));
    out
}

// ─── Property replacement (Update-ManifestProperty, lib/autoupdate.ps1:332-427)

/// PowerShell truthiness, as `Update-ManifestProperty` relies on it
/// (`if ($Manifest.$p -and $Manifest.autoupdate.$p)`).
fn ps_truthy(value: Option<&serde_json::Value>) -> bool {
    match value {
        None | Some(serde_json::Value::Null) => false,
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(serde_json::Value::String(s)) => !s.is_empty(),
        Some(serde_json::Value::Array(a)) => !a.is_empty(),
        Some(serde_json::Value::Object(o)) => !o.is_empty(),
    }
}

/// `@($value).Length` — PowerShell's array coercion (`$null` → 0, scalar → 1).
fn ps_array_len(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Null => 0,
        serde_json::Value::Array(items) => items.len(),
        _ => 1,
    }
}

/// `@($value)` — PowerShell's array coercion.
fn ps_as_array(value: &serde_json::Value) -> Vec<serde_json::Value> {
    match value {
        serde_json::Value::Null => Vec::new(),
        serde_json::Value::Array(items) => items.clone(),
        other => vec![other.clone()],
    }
}

/// Upstream `PropertyHelper` (lib/autoupdate.ps1:527-579): update a manifest
/// property line by line, keeping any existing entries the replacement does
/// not cover. Returns `(newValue, changed)`.
fn property_helper(
    existing: &serde_json::Value,
    value: &serde_json::Value,
) -> (serde_json::Value, bool) {
    if ps_array_len(existing) < ps_array_len(value) {
        return (value.clone(), true);
    }
    match existing {
        serde_json::Value::String(_) => match value {
            serde_json::Value::String(s) => (serde_json::Value::String(s.clone()), true),
            // `$Value -as [String]` on null yields null → no change.
            serde_json::Value::Null => (existing.clone(), false),
            serde_json::Value::Number(n) => (serde_json::Value::String(n.to_string()), true),
            serde_json::Value::Bool(b) => (
                serde_json::Value::String(if *b { "True" } else { "False" }.to_string()),
                true,
            ),
            // Arrays/objects: upstream would stringify to their .NET type
            // name; hok leaves the existing value alone instead of writing a
            // bogus type name into the manifest.
            _ => (existing.clone(), false),
        },
        serde_json::Value::Array(items) => {
            let values = ps_as_array(value);
            let mut out = items.clone();
            let mut changed = false;
            for (i, v) in values.iter().enumerate() {
                if i >= out.len() {
                    break;
                }
                let (new_item, item_changed) = property_helper(&out[i], v);
                out[i] = new_item;
                changed |= item_changed;
            }
            (serde_json::Value::Array(out), changed)
        }
        serde_json::Value::Object(map) => {
            let serde_json::Value::Object(values) = value else {
                return (existing.clone(), false);
            };
            let mut out = map.clone();
            let mut changed = false;
            for (name, current) in map {
                if !ps_truthy(values.get(name)) {
                    continue;
                }
                let (new_value, value_changed) = property_helper(current, &values[name]);
                out.insert(name.clone(), new_value);
                changed |= value_changed;
            }
            (serde_json::Value::Object(out), changed)
        }
        _ => {
            if value.is_null() {
                (existing.clone(), false)
            } else {
                (value.clone(), true)
            }
        }
    }
}

/// Upstream `arch_specific` (lib/manifest.ps1:148-155): the architecture-level
/// value when it is truthy, otherwise the generic one.
fn arch_specific(
    autoupdate: &serde_json::Value,
    arch: &str,
    prop: &str,
) -> Option<serde_json::Value> {
    if let Some(value) = autoupdate
        .get("architecture")
        .and_then(|a| a.get(arch))
        .and_then(|spec| spec.get(prop))
        && ps_truthy(Some(value))
    {
        return Some(value.clone());
    }
    autoupdate.get(prop).filter(|v| ps_truthy(Some(v))).cloned()
}

/// Recursively substitute every string in a JSON value
/// (upstream `substitute`, lib/core.ps1:1203-1225).
fn sub_json_value(v: &serde_json::Value, sub: &dyn Fn(&str) -> String) -> serde_json::Value {
    match v {
        serde_json::Value::String(s) => serde_json::Value::String(sub(s)),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(|i| sub_json_value(i, sub)).collect())
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, vv)| (k.clone(), sub_json_value(vv, sub)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Substitute an autoupdate URL value (string or array) into a `Vec<String>`.
fn substituted_urls(value: Option<&serde_json::Value>, subs: &Substitution) -> Vec<String> {
    match value {
        Some(serde_json::Value::String(s)) if !s.is_empty() => vec![subs.apply(s)],
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| subs.apply(s))
            .collect(),
        _ => Vec::new(),
    }
}

/// Shape `HashHelper`'s result the way PowerShell hands it back
/// (lib/autoupdate.ps1:616-628): a one-element array is unrolled to a bare
/// string, an empty one to `$null`. `PropertyHelper` then compares shapes, so
/// getting this wrong makes a single-URL hash never overwrite the string the
/// manifest already holds.
fn hash_json_value(hashes: Vec<String>) -> serde_json::Value {
    match hashes.len() {
        0 => serde_json::Value::Null,
        1 => serde_json::Value::String(hashes.into_iter().next().unwrap_or_default()),
        _ => serde_json::Value::Array(hashes.into_iter().map(serde_json::Value::String).collect()),
    }
}

/// The names of the architectures the manifest declares
/// (`$Manifest.architecture | Get-Member -MemberType NoteProperty`).
fn manifest_arch_names(root: &serde_json::Value) -> Vec<String> {
    root.get("architecture")
        .and_then(|a| a.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

/// One non-`hash` property of `Update-ManifestProperty`
/// (lib/autoupdate.ps1:392-417).
///
/// Official shape:
/// ```text
/// if ($Manifest.$p -and $Manifest.autoupdate.$p)      -> global update
/// elseif ($Manifest.architecture)                      -> per-architecture update
/// ```
fn update_manifest_property(
    root: &mut serde_json::Value,
    autoupdate: &serde_json::Value,
    prop: &str,
    subs: &Substitution,
) {
    if ps_truthy(root.get(prop)) && ps_truthy(autoupdate.get(prop)) {
        let Some(source) = autoupdate.get(prop) else {
            return;
        };
        let value = sub_json_value(source, &|s| subs.apply(s));
        if let Some(slot) = root.get_mut(prop) {
            *slot = property_helper(slot, &value).0;
        }
        return;
    }

    for arch in manifest_arch_names(root) {
        let ptr = format!("/architecture/{}", arch);
        let existing = root.pointer(&ptr).and_then(|a| a.get(prop)).cloned();
        if !ps_truthy(existing.as_ref()) {
            continue;
        }
        let Some(spec) = arch_specific(autoupdate, &arch, prop) else {
            continue;
        };
        // Upstream wraps the arch value in `@(...)` and then re-wraps it, but
        // the observable result (recorded 2026-09-13) keeps the value's shape:
        // a scalar stays a scalar, an array stays an array.
        let value = sub_json_value(&spec, &|s| subs.apply(s));
        if let Some(slot) = root.pointer_mut(&ptr).and_then(|a| a.get_mut(prop)) {
            *slot = property_helper(slot, &value).0;
        }
    }
}

/// The `hash` property of `Update-ManifestProperty`
/// (lib/autoupdate.ps1:374-391).
///
/// Upstream only computes a hash where the manifest already declares one:
/// `$Manifest.hash` (global) or, failing that, for each entry of
/// `$Manifest.architecture`. A manifest with neither is left untouched — hok
/// used to synthesise a top-level `hash` from `autoupdate.url` regardless.
#[allow(clippy::too_many_arguments)]
async fn update_hash_property(
    session: &Session,
    root: &mut serde_json::Value,
    autoupdate: &serde_json::Value,
    subs: &Substitution,
    cache_dir: &std::path::Path,
    app_name: &str,
    version: &str,
) -> Fallible<()> {
    if ps_truthy(root.get("hash")) {
        let urls = substituted_urls(autoupdate.get("url"), subs);
        if urls.is_empty() {
            // Upstream substitutes `$null` and hands `HashHelper` an empty URL
            // list, which blanks the existing hash. hok leaves it alone rather
            // than destroying a hash it cannot recompute.
            return Ok(());
        }
        let extractions = autoupdate.get("hash").map(ps_as_array).unwrap_or_default();
        let hashes = download_and_hash_multi(
            session,
            app_name,
            version,
            &urls,
            &extractions,
            subs,
            cache_dir,
        )
        .await?;
        let value = hash_json_value(hashes);
        if let Some(slot) = root.get_mut("hash") {
            *slot = property_helper(slot, &value).0;
        }
        return Ok(());
    }

    for arch in manifest_arch_names(root) {
        // Upstream assigns `$Manifest.architecture.$arch.hash`, which throws
        // when the entry has no `hash` property — so an architecture without a
        // hash does not gain one (verified 2026-09-13). Skip it, and skip the
        // `HashHelper` download that would have been thrown away.
        if root
            .pointer(&format!("/architecture/{}", arch))
            .and_then(|a| a.get("hash"))
            .is_none()
        {
            continue;
        }
        let Some(url_value) = arch_specific(autoupdate, &arch, "url") else {
            // Upstream would substitute `$null` and hand an empty URL list to
            // `HashHelper`, which blanks the hash. hok leaves such an entry
            // alone rather than writing an empty hash.
            continue;
        };
        let urls = substituted_urls(Some(&url_value), subs);
        if urls.is_empty() {
            continue;
        }
        let extractions = arch_specific(autoupdate, &arch, "hash")
            .map(|v| ps_as_array(&v))
            .unwrap_or_default();
        let hashes = download_and_hash_multi(
            session,
            app_name,
            version,
            &urls,
            &extractions,
            subs,
            cache_dir,
        )
        .await?;
        let value = hash_json_value(hashes);
        let ptr = format!("/architecture/{}", arch);
        if let Some(obj) = root.pointer_mut(&ptr) {
            let existing = obj.get("hash").cloned().unwrap_or(serde_json::Value::Null);
            obj["hash"] = property_helper(&existing, &value).0;
        }
    }
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
    use sxd_xpath::{Value, evaluate_xpath};

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
///   $url       — the checkver URL (if present)
///   $version   — the current installed version
///   HTTP_PROXY / HTTPS_PROXY — the Scoop proxy config, so downloads inside
///               the script (Invoke-WebRequest, curl, git, ...) honor it
///
/// The script's stdout is captured and trimmed as the new version string.
///
/// This runs entirely off the `Session` (pure `ps_command()` invocation), so
/// it can be executed from a worker thread alongside the downloads.
async fn run_checkver_script(
    script: &str,
    url: Option<&str>,
    proxy: Option<&str>,
    timeout_secs: u64,
    token: &CancellationToken,
) -> Fallible<Option<String>> {
    use std::io::Read;
    use std::time::Duration;

    // Cooperative cancellation: bail before spawning when already cancelled.
    if token.is_cancelled() {
        return Err(Error::Custom("checkver script cancelled".to_owned()));
    }

    let mut cmd = crate::internal::os::ps_command();
    cmd.arg("-Command")
        .arg(script)
        .env("url", url.unwrap_or(""))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // Expose the proxy to the script via environment variables. The value
    // always comes from the Scoop config's `proxy` key (config.rs: "" or
    // "none" clears it) — hok never reads HTTP_PROXY/HTTPS_PROXY itself.
    // Set only when configured: on None the child inherits whatever
    // HTTP_PROXY/HTTPS_PROXY our own environment already has, so explicit
    // config always wins over inherited system/user proxy settings.
    if let Some(p) = proxy {
        cmd.env("HTTP_PROXY", p).env("HTTPS_PROXY", p);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Custom(format!("spawn powershell: {}", e)))?;

    // Wait with a timeout, racing cooperative Ctrl-C cancellation. Both
    // timeouts and cancellation kill the child so no stray powershell lingers.
    let start = std::time::Instant::now();
    let deadline = Duration::from_secs(timeout_secs);
    loop {
        if token.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Custom("checkver script cancelled".to_owned()));
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error::Custom(format!("timed out after {}s", timeout_secs)));
                }
                // Async sleep so the runtime is not blocked; race it against
                // cancellation so a Ctrl-C is noticed promptly.
                tokio::select! {
                    _ = token.cancelled() => {}
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                }
            }
            Err(e) => return Err(Error::Custom(format!("wait: {}", e))),
        }
    }

    let mut output = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .map_err(|e| Error::Custom(format!("read output: {}", e)))?;

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
        Checkver::default()
    }

    /// Test-local wrapper: `extract_version` takes the resolved regex and the
    /// substitution table, which these unit tests take from the config and
    /// leave empty respectively.
    fn extract_version(
        content: &str,
        cv: &Checkver,
        jsonpath: Option<&str>,
        regex: Option<&str>,
    ) -> Option<(String, Vec<String>, HashMap<String, String>)> {
        super::extract_version(
            content,
            cv,
            jsonpath.or(cv.jsonpath.as_deref()),
            regex.or(cv.regex.as_deref()),
            &Substitution::default(),
        )
    }

    // ── apply_replace ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn apply_replace_no_pattern_returns_first_capture() {
        let caps = vec!["full_match".to_string(), "1.2.3".to_string()];
        assert_eq!(apply_replace(&caps, &HashMap::new(), None), "1.2.3");
    }

    #[tokio::test]
    async fn run_checkver_script_pre_cancelled_bails_without_spawning() {
        let token = CancellationToken::new();
        token.cancel();
        // A pre-cancelled token must abort the script check immediately (the
        // early-cancellation guard fires before the child is ever spawned).
        let result = run_checkver_script("Start-Sleep -Seconds 60", None, None, 120, &token).await;
        let err = result.expect_err("pre-cancelled script check should fail");
        assert!(
            err.to_string().contains("cancelled"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn apply_replace_no_pattern_falls_back_to_zeroth_when_no_groups() {
        let caps = vec!["1.2.3".to_string()];
        assert_eq!(apply_replace(&caps, &HashMap::new(), None), "1.2.3");
    }

    #[tokio::test]
    async fn apply_replace_empty_captures_returns_empty() {
        let caps: Vec<String> = vec![];
        assert_eq!(apply_replace(&caps, &HashMap::new(), None), "");
    }

    #[tokio::test]
    async fn apply_replace_pattern_substitutes_dollar_index() {
        let caps = vec!["full".to_string(), "1".to_string(), "2".to_string()];
        assert_eq!(apply_replace(&caps, &HashMap::new(), Some("$1.$2")), "1.2");
    }

    #[tokio::test]
    async fn apply_replace_pattern_substitutes_zero() {
        let caps = vec!["fullmatch".to_string()];
        assert_eq!(
            apply_replace(&caps, &HashMap::new(), Some("v$0")),
            "vfullmatch"
        );
    }

    #[tokio::test]
    async fn apply_replace_pattern_substitutes_named_groups() {
        // tim: .NET named-group syntax `${main}.${patch}`
        let caps: Vec<String> = Vec::new();
        let named = HashMap::from([
            ("main".to_string(), "3.5.0".to_string()),
            ("patch".to_string(), "22143".to_string()),
        ]);
        assert_eq!(
            apply_replace(&caps, &named, Some("${main}.${patch}")),
            "3.5.0.22143"
        );
    }

    // ── resolve_checkver_source (upstream's shortcut blocks) ────────────────

    /// A minimal manifest parsed from `manifest_json`, fed to
    /// `resolve_checkver_source` with a per-test temp dir.
    fn resolve_with(
        label: &str,
        cv: &Checkver,
        manifest_json: &str,
        stem: &str,
        has_token: bool,
    ) -> ResolvedSource {
        let root = crate::test_utils::tmpdir(label);
        let path = root.join("app.json");
        std::fs::write(&path, manifest_json).unwrap();
        let manifest = manifest::parse_file(&path).unwrap();
        resolve_checkver_source(cv, &manifest, stem, has_token)
    }

    #[test]
    fn github_string_shortcut_derives_url_from_homepage() {
        let mut cv = checkver_default();
        cv.github_homepage = true;
        let resolved = resolve_with(
            "sv_gh_home",
            &cv,
            r#"{"version":"1.0.0","homepage":"https://github.com/o/r"}"#,
            "app",
            false,
        );
        assert_eq!(resolved.url, "https://github.com/o/r/releases/latest");
        // GitHub HTML mode: upstream's `/releases/tag/...` regex, no jsonpath.
        assert_eq!(
            resolved.regex.as_deref(),
            Some(GITHUB_HTML_DEFAULT_REGEX),
            "HTML mode must not use the API regex"
        );
        assert!(resolved.jsonpath.is_none());
        assert!(resolved.skip_reason.is_none());
    }

    #[test]
    fn github_string_shortcut_accepts_a_www_homepage() {
        // `(www\.)?` in `$githubUrlPattern` (`bin/checkver.ps1:156`).
        let mut cv = checkver_default();
        cv.github_homepage = true;
        let resolved = resolve_with(
            "sv_gh_www",
            &cv,
            r#"{"version":"1.0.0","homepage":"https://www.github.com/o/r"}"#,
            "app",
            false,
        );
        assert_eq!(resolved.url, "https://www.github.com/o/r/releases/latest");
    }

    #[test]
    fn github_string_shortcut_with_a_token_rewrites_to_the_api() {
        let mut cv = checkver_default();
        cv.github_homepage = true;
        let resolved = resolve_with(
            "sv_gh_home_tok",
            &cv,
            r#"{"version":"1.0.0","homepage":"https://github.com/o/r/"}"#,
            "app",
            true,
        );
        // The trailing slash is trimmed *before* the suffix is appended.
        assert_eq!(
            resolved.url,
            "https://api.github.com/repos/o/r/releases/latest"
        );
        // GitHub API mode: `$.tag_name` and the API regex (`-` allowed).
        assert_eq!(
            resolved.jsonpath.as_deref(),
            Some(GITHUB_API_DEFAULT_JSONPATH)
        );
        assert_eq!(resolved.regex.as_deref(), Some(GITHUB_API_DEFAULT_REGEX));
    }

    #[test]
    fn github_string_shortcut_without_a_token_keeps_the_html_page() {
        // The rewrite still needs a token, so an unauthenticated run does not
        // burn the anonymous API quota; the `$useGithubAPI` gate around it is
        // gone (`35334422`).
        let mut cv = checkver_default();
        cv.github_homepage = true;
        let resolved = resolve_with(
            "sv_gh_home_notok",
            &cv,
            r#"{"version":"1.0.0","homepage":"https://github.com/o/r/"}"#,
            "app",
            false,
        );
        assert_eq!(resolved.url, "https://github.com/o/r/releases/latest");
    }

    #[test]
    fn github_string_shortcut_skips_a_non_github_homepage() {
        // `error "... expects $fieldUsed to be a valid GitHub repository URL";
        // return` (`bin/checkver.ps1:165-168`) — the manifest is skipped.
        let mut cv = checkver_default();
        cv.github_homepage = true;
        let resolved = resolve_with(
            "sv_gh_home_bad",
            &cv,
            r#"{"version":"1.0.0","homepage":"https://www.7-zip.org"}"#,
            "7zip",
            true,
        );
        assert!(resolved.skip_reason.is_some(), "got {resolved:?}");
        assert!(resolved.url.is_empty(), "nothing must be downloaded");
        let message = resolved.skip_reason.unwrap();
        assert!(message.contains("homepage"), "got {message}");
    }

    #[test]
    fn github_map_with_another_key_still_rewrites_with_a_token() {
        // `bdf63444` removed `PSObject.Properties.Count -eq 1`, so a map with
        // extra keys uses the API whenever a token is configured. The user
        // regex is kept; the API jsonpath default is injected.
        let mut cv = checkver_default();
        cv.github = Some("https://github.com/JFLarvoire/the_silver_searcher".to_string());
        cv.regex = Some(r"([\d.]+)-Windows".to_string());
        let resolved = resolve_with(
            "sv_gh_map_re",
            &cv,
            r#"{"version":"1.0.0","homepage":"https://github.com/JFLarvoire/the_silver_searcher"}"#,
            "ag",
            true,
        );
        assert_eq!(
            resolved.url,
            "https://api.github.com/repos/JFLarvoire/the_silver_searcher/releases/latest"
        );
        assert_eq!(resolved.regex.as_deref(), Some(r"([\d.]+)-Windows"));
        assert_eq!(
            resolved.jsonpath.as_deref(),
            Some(GITHUB_API_DEFAULT_JSONPATH)
        );
    }

    #[test]
    fn github_map_skips_an_invalid_value_naming_the_field() {
        let mut cv = checkver_default();
        cv.github = Some("https://github.com/o".to_string());
        let resolved = resolve_with(
            "sv_gh_bad_map",
            &cv,
            r#"{"version":"1.0.0","homepage":"https://github.com/o/r"}"#,
            "app",
            false,
        );
        let message = resolved.skip_reason.expect("the map value is invalid");
        assert!(message.contains("checkver.github"), "got {message}");
    }

    #[test]
    fn github_map_uses_the_api_with_a_token() {
        let mut cv = checkver_default();
        cv.github = Some("https://github.com/ip7z/7zip".to_string());
        let resolved = resolve_with(
            "sv_gh_solo",
            &cv,
            r#"{"version":"1.0.0","homepage":"https://www.7-zip.org"}"#,
            "7zip",
            true,
        );
        assert_eq!(
            resolved.url,
            "https://api.github.com/repos/ip7z/7zip/releases/latest"
        );
    }

    #[test]
    fn github_map_without_a_token_keeps_the_html_page() {
        let mut cv = checkver_default();
        cv.github = Some("https://github.com/ip7z/7zip".to_string());
        let resolved = resolve_with(
            "sv_gh_solo_notok",
            &cv,
            r#"{"version":"1.0.0","homepage":"https://www.7-zip.org"}"#,
            "7zip",
            false,
        );
        assert_eq!(resolved.url, "https://github.com/ip7z/7zip/releases/latest");
    }

    /// `checkver.github` may already point at the API; the suffix is then not
    /// appended (`if ($url -notlike 'https://api.github.com*')`,
    /// `bin/checkver.ps1:171-173`), so `arc.json`'s
    /// `.../repos/mholt/archiver/releases` shape keeps working.
    #[test]
    fn github_map_keeps_an_already_complete_api_prefix() {
        let mut cv = checkver_default();
        cv.github = Some("https://api.github.com/repos/aptos-labs/aptos-core/releases".to_string());
        cv.regex = Some(r"(?i)download/aptos-cli-v([\d.]+)/aptos-cli".to_string());
        let resolved = resolve_with(
            "sv_gh_api_prefix",
            &cv,
            r#"{"version":"1.0.0","homepage":"https://aptosnetwork.com"}"#,
            "aptos-cli",
            true,
        );
        assert_eq!(
            resolved.url, "https://api.github.com/repos/aptos-labs/aptos-core/releases",
            "an api.github.com value never gets the suffix"
        );
        // API mode with a user regex: the jsonpath default is still injected,
        // because only `checkver.script` suppresses it.
        assert_eq!(
            resolved.jsonpath.as_deref(),
            Some(GITHUB_API_DEFAULT_JSONPATH)
        );
    }

    #[test]
    fn github_map_keeps_an_already_complete_latest_suffix() {
        let mut cv = checkver_default();
        cv.github =
            Some("https://api.github.com/repos/bflattened/bflat/releases/latest/".to_string());
        let resolved = resolve_with(
            "sv_gh_latest_suffix",
            &cv,
            r#"{"version":"1.0.0","homepage":"https://github.com/bflattened/bflat"}"#,
            "bflat",
            false,
        );
        assert_eq!(
            resolved.url,
            "https://api.github.com/repos/bflattened/bflat/releases/latest"
        );
        assert_eq!(resolved.regex.as_deref(), Some(GITHUB_API_DEFAULT_REGEX));
    }

    /// `d452e8f6`: with `checkver.script` the API-mode jsonpath default must not
    /// be injected — the script's output is the page.
    #[test]
    fn github_api_mode_with_a_script_skips_the_jsonpath_default() {
        let mut cv = checkver_default();
        cv.github = Some("https://api.github.com/repos/o/r/releases/latest".to_string());
        cv.script = Some(manifest::Vectorized(vec![
            "Write-Output 'v1.2.3'".to_string(),
        ]));
        let resolved = resolve_with(
            "sv_gh_script",
            &cv,
            r#"{"version":"1.0.0","homepage":"https://www.7-zip.org"}"#,
            "app",
            false,
        );
        assert!(resolved.jsonpath.is_none(), "got {resolved:?}");
        // The regex default still applies: the script's output has to be parsed.
        assert_eq!(resolved.regex.as_deref(), Some(GITHUB_API_DEFAULT_REGEX));
    }

    /// An explicit `checkver.jsonpath` wins over the API default, because
    /// upstream applies `jp`/`jsonpath` *after* the GitHub block
    /// (`bin/checkver.ps1:231-233`).
    #[test]
    fn explicit_jsonpath_wins_over_the_github_api_default() {
        let mut cv = checkver_default();
        cv.github = Some("https://api.github.com/repos/o/r/releases/latest".to_string());
        cv.jsonpath = Some("$.name".to_string());
        let resolved = resolve_with(
            "sv_gh_explicit_jp",
            &cv,
            r#"{"version":"1.0.0","homepage":"https://www.7-zip.org"}"#,
            "app",
            false,
        );
        assert_eq!(resolved.jsonpath.as_deref(), Some("$.name"));
    }

    /// The default regexes now admit `-` in the version (`cbd7c64b`).
    #[test]
    fn default_regexes_admit_hyphenated_versions() {
        let html = Regex::new(GITHUB_HTML_DEFAULT_REGEX).unwrap();
        assert_eq!(
            &html
                .captures("https://github.com/o/r/releases/tag/v1.2.3-4")
                .expect("HTML default must match a release tag")[1],
            "1.2.3-4"
        );
        let api = Regex::new(GITHUB_API_DEFAULT_REGEX).unwrap();
        assert_eq!(
            &api.captures("v1.2.3-4").expect("API default must match")[1],
            "1.2.3-4"
        );
    }

    #[test]
    fn sourceforge_string_shortcut_uses_the_homepage_project_and_a_cdata_regex() {
        let mut cv = checkver_default();
        cv.sourceforge_homepage = true;
        let resolved = resolve_with(
            "sv_sf_home",
            &cv,
            r#"{"version":"1.0.0","homepage":"https://sourceforge.net/projects/sevenzip/files/7-Zip/"}"#,
            "7zip",
            false,
        );
        assert_eq!(
            resolved.url, "https://sourceforge.net/projects/sevenzip/rss?path=/7-Zip",
            "the homepage path capture is a single segment"
        );
        assert_eq!(
            resolved.regex.as_deref(),
            Some(r"CDATA\[/7-Zip/.*?(?!\.)([\d.]+)(?<=\d).*?\]"),
            "the CDATA anchor wraps the default sourceforge regex"
        );
    }

    #[test]
    fn sourceforge_string_shortcut_falls_back_to_strip_ext_of_the_name() {
        let mut cv = checkver_default();
        cv.sourceforge_homepage = true;
        let resolved = resolve_with(
            "sv_sf_fallback",
            &cv,
            r#"{"version":"1.0.0","homepage":"https://example.com/"}"#,
            "7zip-24.json",
            false,
        );
        assert_eq!(resolved.url, "https://sourceforge.net/projects/7zip-24/rss");
        assert_eq!(
            resolved.regex.as_deref(),
            Some(r"CDATA\[/.*?(?!\.)([\d.]+)(?<=\d).*?\]")
        );
    }

    #[test]
    fn sourceforge_map_form_uses_its_own_project_and_path() {
        let mut cv = checkver_default();
        cv.sourceforge = Some(crate::package::manifest::Sourceforge {
            project: Some("foo".to_string()),
            path: Some("/bar".to_string()),
        });
        let resolved = resolve_with(
            "sv_sf_map",
            &cv,
            r#"{"version":"1.0.0","homepage":"https://example.com"}"#,
            "app",
            false,
        );
        assert_eq!(
            resolved.url,
            "https://sourceforge.net/projects/foo/rss?path=/bar"
        );
    }

    #[test]
    fn plain_source_uses_the_manifest_url_then_the_homepage() {
        let mut cv = checkver_default();
        cv.url = Some("https://example.com/latest.txt".to_string());
        let resolved = resolve_with(
            "sv_plain_url",
            &cv,
            r#"{"version":"1.0.0","homepage":"https://example.com"}"#,
            "app",
            false,
        );
        assert_eq!(resolved.url, "https://example.com/latest.txt");
        assert!(resolved.regex.is_none());

        let resolved = resolve_with(
            "sv_plain_home",
            &checkver_default(),
            r#"{"version":"1.0.0","homepage":"https://example.com"}"#,
            "app",
            false,
        );
        assert_eq!(resolved.url, "https://example.com");
        assert!(resolved.regex.is_none());
    }

    #[test]
    fn checkver_user_agent_substitutes_the_version_variables() {
        // `substitute $json.checkver.useragent $substitutions`
        // (bin/checkver.ps1:118) — 6 manifests in the local buckets rely on it.
        let mut cv = checkver_default();
        cv.useragent = Some("Fiddler/$version".to_string());
        assert_eq!(
            checkver_user_agent(&cv, "2.5.1").as_deref(),
            Some("Fiddler/2.5.1")
        );
        // A variable the substitution table does not know stays literal, exactly
        // as upstream: `$app` is in scope only for `checkver.script`.
        cv.useragent = Some("$app/$version".to_string());
        assert_eq!(checkver_user_agent(&cv, "2.0").as_deref(), Some("$app/2.0"));
        // No manifest value means "keep the session default".
        assert_eq!(checkver_user_agent(&checkver_default(), "1.0"), None);
    }

    // ── json_path_legacy ────────────────────────────────────────────────────

    #[test]
    fn json_path_legacy_walks_dotted_properties() {
        let json = r#"{"assets":{"browser_download_url":"https://x/app.zip"}}"#;
        assert_eq!(
            json_path_legacy(
                json,
                "assets.browser_download_url",
                &Substitution::default()
            ),
            Some("https://x/app.zip".to_string())
        );
    }

    #[test]
    fn json_path_legacy_skips_a_leading_dollar() {
        let json = r#"{"tag_name":"v1.2.3"}"#;
        assert_eq!(
            json_path_legacy(json, "$.tag_name", &Substitution::default()),
            Some("v1.2.3".to_string())
        );
    }

    #[test]
    fn json_path_legacy_indexes_arrays() {
        let json = r#"{"asset":[{"name":"a"},{"name":"b"}]}"#;
        assert_eq!(
            json_path_legacy(json, "$.asset[1].name", &Substitution::default()),
            Some("b".to_string())
        );
    }

    #[test]
    fn json_path_legacy_missing_property_is_none() {
        assert_eq!(
            json_path_legacy(r#"{"a":1}"#, "b.c", &Substitution::default()),
            None
        );
    }

    // ── json_path (single/multi result shape) ──────────────────────────────

    #[test]
    fn json_path_serialises_multiple_tokens_for_a_regex() {
        // With a regex in play (`$single = $false`) several tokens are handed
        // over as a JSON array.
        let json = r#"{"a":["x","y"]}"#;
        let text = json_path(json, "$.a[*]", false, false, &Substitution::default()).unwrap();
        assert!(text.starts_with('['), "got {text}");
    }

    #[test]
    fn json_path_takes_the_first_token_without_a_regex() {
        let json = r#"{"a":["x","y"]}"#;
        assert_eq!(
            json_path(json, "$.a[*]", false, true, &Substitution::default()).as_deref(),
            Some("x")
        );
    }

    #[test]
    fn json_path_empty_selection_is_none() {
        assert_eq!(
            json_path(r#"{"a":1}"#, "$.b", false, false, &Substitution::default()),
            None
        );
    }

    // ── extract_version (regex) ──────────────────────────────────────────────

    #[tokio::test]
    async fn extract_version_regex_basic() {
        let cv = Checkver {
            regex: Some(r"version=([\d.]+)".to_string()),
            ..checkver_default()
        };
        let content = "some text version=1.2.3 end";
        let result = extract_version(content, &cv, None, None);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "1.2.3");
    }

    #[tokio::test]
    async fn extract_version_regex_no_match_returns_none_if_content_empty() {
        let cv = Checkver {
            regex: Some(r"version=([\d.]+)".to_string()),
            ..checkver_default()
        };
        let result = extract_version("", &cv, None, None);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn extract_version_regex_override_takes_precedence() {
        let cv = Checkver {
            regex: Some(r"v(\d+)".to_string()),
            ..checkver_default()
        };
        // regex_override overrides cv.regex
        let result = extract_version("1.9.9 release", &cv, None, Some(r"(\d+\.\d+\.\d+)"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "1.9.9");
    }

    // ── extract_version (Scoop group-name semantics) ─────────────────────────

    #[tokio::test]
    async fn extract_version_unnamed_group_beats_named_hash() {
        // we-meet: `(?<hash>...)` precedes the unnamed version group. Scoop
        // reads group name "1" = the first *unnamed* group (in .NET named
        // groups get negative numbers), so the version is extracted, not the
        // md5. The named hash group stays available for autoupdate.
        let cv = Checkver {
            regex: Some(r#""md5":"(?<hash>[a-f0-9]+)".+"version":"([\d\.]+)""#.to_string()),
            ..checkver_default()
        };
        let json = r#"{"md5":"a2bf9c01f76b1df44383ab2f529bec13","version":"3.44.10.457"}"#;
        let result = extract_version(json, &cv, None, None);
        let (ver, _, named) = result.expect("extraction");
        assert_eq!(ver, "3.44.10.457");
        assert_eq!(
            named.get("hash").map(String::as_str),
            Some("a2bf9c01f76b1df44383ab2f529bec13")
        );
    }

    #[test]
    fn extract_version_named_version_fallback() {
        // Only a named `version` group: Scoop falls back to it (group "1"
        // does not exist)
        let cv = Checkver {
            regex: Some(r"v(?<version>[\d.]+)".to_string()),
            ..checkver_default()
        };
        let result = extract_version("latest v1.2.3", &cv, None, None);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "1.2.3");
    }

    #[test]
    fn extract_version_named_only_without_version_returns_none() {
        // Only a named group that is not `version`: both Scoop fallbacks fail,
        // so extraction reports failure (no further fallback)
        let cv = Checkver {
            regex: Some(r"(?<hash>[a-f0-9]{32})".to_string()),
            ..checkver_default()
        };
        let result = extract_version("a2bf9c01f76b1df44383ab2f529bec13", &cv, None, None);
        assert!(result.is_none());
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

    // ── extract_version (jsonpath + custom regex chaining) ───────────────────

    #[test]
    fn extract_version_jsonpath_chains_custom_regex() {
        // monitorian: auto `$.tag_name` yields "4.15.2-Installer", then the
        // custom regex chains onto that value (Scoop checkver.ps1 L342-345)
        let cv = Checkver {
            regex: Some(r"([\d.]+)-Installer".to_string()),
            ..checkver_default()
        };
        let json = r#"{"tag_name": "4.15.2-Installer", "name": "4.15.2-Installer"}"#;
        let result = extract_version(json, &cv, Some("$.tag_name"), None);
        let (ver, caps, _) = result.expect("chained extraction");
        assert_eq!(ver, "4.15.2");
        assert_eq!(caps, vec!["4.15.2-Installer", "4.15.2"]);
    }

    #[test]
    fn extract_version_jsonpath_chaining_fails_when_the_regex_does_not_match() {
        // With a jsonpath *and* a regex the jsonpath value becomes the page,
        // and a regex that cannot match it fails the whole extraction
        // (upstream's `next "couldn't match ..."`). The default GitHub regex is
        // a plain regex in this model — hok no longer special-cases it.
        let cv = Checkver {
            regex: Some(r"/releases/tag/(?:v|V)?([\d.]+)".to_string()),
            ..checkver_default()
        };
        let json = r#"{"tag_name": "v1.5.0", "name": "Release 1.5.0"}"#;
        let result = extract_version(json, &cv, Some("$.tag_name"), None);
        assert!(result.is_none());
    }

    #[test]
    fn extract_version_jsonpath_chaining_runs_the_regex_on_the_tag() {
        // The real-world `{github, jsonpath: "$.tag_name", regex: "v([\\d.]+)"}`
        // shape (e.g. bflat.json): the tag feeds the user regex.
        let cv = Checkver {
            regex: Some(r"v([\d.]+)".to_string()),
            ..checkver_default()
        };
        let json = r#"{"tag_name": "v1.5.0", "name": "Release 1.5.0"}"#;
        let result = extract_version(json, &cv, Some("$.tag_name"), None);
        assert_eq!(result.expect("chained").0, "1.5.0");
    }

    // ── extract_version (github + custom regex) ──────────────────────────────

    #[test]
    fn extract_version_github_custom_regex_applies_to_content() {
        // Without a jsonpath (e.g. a plain regex checkver), the custom regex
        // runs against the raw content and captures the version.
        let cv = Checkver {
            regex: Some(r"([\d.]+)-Installer".to_string()),
            ..checkver_default()
        };
        let json = r#"{"tag_name": "4.15.2-Installer", "name": "4.15.2-Installer"}"#;
        let result = extract_version(json, &cv, None, None);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "4.15.2");
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

    // ── to_title_case (Scoop `$match<ToTitleCase(name)>` autoupdate vars) ───

    #[test]
    fn to_title_case_matches_dotnet_textinfo() {
        // Truth values recorded from PowerShell 5.1 / .NET `TextInfo.ToTitleCase`
        // (2026-09-13): a mixed-case word is folded to first-upper + rest-lower,
        // but an all-uppercase word is returned unchanged.
        assert_eq!(to_title_case("hash"), "Hash");
        assert_eq!(to_title_case("version"), "Version");
        assert_eq!(to_title_case("tag"), "Tag");
        assert_eq!(to_title_case("gccversion"), "Gccversion");
        assert_eq!(to_title_case("GccVersion"), "Gccversion");
        assert_eq!(to_title_case("myGroup"), "Mygroup");
        assert_eq!(to_title_case("AB"), "AB");
        assert_eq!(to_title_case(""), "");
    }

    #[test]
    fn to_title_case_splits_words_on_non_alphanumerics() {
        assert_eq!(to_title_case("a-b"), "A-B");
        assert_eq!(to_title_case("a b"), "A B");
    }

    // ── Substitution (Get-VersionSubstitution + get_hash_for_app) ──────────

    fn subs_for(version: &str) -> Substitution {
        // `captures` mirrors upstream's `$matchesHashtable`: index 0 is the
        // full match, index 1 the first group.
        version_substitutions(
            version,
            &[version.to_string(), version.to_string()],
            &HashMap::new(),
        )
    }

    #[test]
    fn substitution_replaces_version_variables() {
        let subs = subs_for("1.2.3");
        assert_eq!(subs.apply("v$version"), "v1.2.3");
        assert_eq!(subs.apply("$majorVersion.$minorVersion"), "1.2");
        assert_eq!(subs.apply("$cleanVersion"), "123");
        assert_eq!(subs.apply("$match1"), "1.2.3");
    }

    #[test]
    fn substitution_resolves_match_vars_case_insensitively() {
        // PowerShell hashtables and `-match` are case-insensitive, so any
        // casing of `$match<Name>` must resolve.
        let named = HashMap::from([("tag".to_string(), "v9".to_string())]);
        let subs = version_substitutions("1.0.0", &[], &named);
        for token in ["$matchTag", "$matchtag", "$MATCHTAG", "$MatchTag"] {
            assert_eq!(subs.apply(token), "v9", "token {token}");
        }
    }

    #[test]
    fn substitution_keeps_unknown_match_vars() {
        let subs = subs_for("1.0.0");
        assert_eq!(subs.apply("$matchNope"), "$matchNope");
    }

    #[test]
    fn substitution_url_variables_match_upstream() {
        // Truth values recorded from upstream `get_hash_for_app`
        // (lib/autoupdate.ps1:212-219) / `strip_ext`, 2026-09-13. Note
        // `strip_ext`'s `\.[^\.]*$` swallows a trailing `?query` too, so
        // `$urlNoExt` drops `.zip?download=1` — an upstream quirk we keep.
        let subs = subs_for("1.2.3").with_url("https://ex.com/dl/app-1.2.3.zip?download=1#/dl.7z");
        assert_eq!(
            subs.apply("$url"),
            "https://ex.com/dl/app-1.2.3.zip?download=1"
        );
        assert_eq!(
            subs.apply("$baseurl/checksums.txt"),
            "https://ex.com/dl/checksums.txt"
        );
        assert_eq!(subs.apply("$basename"), "app-1.2.3.zip");
        assert_eq!(subs.apply("$urlNoExt"), "https://ex.com/dl/app-1.2.3");
        assert_eq!(subs.apply("$basenameNoExt"), "app-1.2.3");
    }

    #[test]
    fn substitution_url_variables_without_query() {
        let subs = subs_for("1.2.3").with_url("https://ex.com/dl/app-1.2.3.zip");
        assert_eq!(subs.apply("$url"), "https://ex.com/dl/app-1.2.3.zip");
        assert_eq!(subs.apply("$baseurl"), "https://ex.com/dl");
        assert_eq!(subs.apply("$basename"), "app-1.2.3.zip");
        assert_eq!(subs.apply("$urlNoExt"), "https://ex.com/dl/app-1.2.3");
        assert_eq!(subs.apply("$basenameNoExt"), "app-1.2.3");
    }

    #[test]
    fn substitution_basename_is_url_decoded() {
        // Upstream wraps `url_remote_filename` in `UrlDecode`.
        let subs = subs_for("1.0").with_url("https://ex.com/a%20b/app%201.0.zip");
        assert_eq!(subs.apply("$basename"), "app 1.0.zip");
    }

    #[test]
    fn substitution_applies_longest_key_first() {
        // `$url` must not eat the `$urlNoExt` prefix (upstream's Hashtable
        // enumeration order is unspecified and does exactly that).
        let subs = subs_for("1.0.0").with_url("https://ex.com/p/app.zip");
        assert_eq!(subs.apply("$urlNoExt"), "https://ex.com/p/app");
        assert_eq!(subs.apply("$basenameNoExt"), "app");
    }

    #[test]
    fn substitution_escapes_values_for_regex_use() {
        let subs = subs_for("1.0.0").with_url("https://ex.com/p/app(1).zip");
        assert_eq!(subs.apply_escaped("$basename"), r"app\(1\)\.zip");
        // The plain pass keeps the value verbatim.
        assert_eq!(subs.apply("$basename"), "app(1).zip");
    }

    // ── PropertyHelper (lib/autoupdate.ps1:527-579) ────────────────────────

    #[test]
    fn property_helper_replaces_scalar() {
        let (value, changed) =
            property_helper(&serde_json::json!("old"), &serde_json::json!("new"));
        assert_eq!(value, serde_json::json!("new"));
        assert!(changed);
    }

    #[test]
    fn property_helper_keeps_extra_existing_array_items() {
        // Upstream updates the array element by element, so entries the
        // replacement does not cover survive.
        let (value, changed) = property_helper(
            &serde_json::json!(["a", "b", "c"]),
            &serde_json::json!(["A", "B"]),
        );
        assert_eq!(value, serde_json::json!(["A", "B", "c"]));
        assert!(changed);
    }

    #[test]
    fn property_helper_replaces_when_value_is_longer() {
        let (value, changed) =
            property_helper(&serde_json::json!(["a"]), &serde_json::json!(["A", "B"]));
        assert_eq!(value, serde_json::json!(["A", "B"]));
        assert!(changed);
    }

    #[test]
    fn property_helper_walks_objects() {
        let (value, changed) = property_helper(
            &serde_json::json!({"a": "x", "b": "y"}),
            &serde_json::json!({"a": "X"}),
        );
        assert_eq!(value, serde_json::json!({"a": "X", "b": "y"}));
        assert!(changed);
    }

    #[test]
    fn property_helper_null_value_leaves_string_alone() {
        let (value, changed) =
            property_helper(&serde_json::json!("keep"), &serde_json::Value::Null);
        assert_eq!(value, serde_json::json!("keep"));
        assert!(!changed);
    }

    // ── property scope + hash write condition ─────────────────────────────

    fn autoupdate_fixture(root: &serde_json::Value) -> serde_json::Value {
        root.get("autoupdate").cloned().unwrap()
    }

    #[test]
    fn property_scope_is_driven_by_autoupdate_keys() {
        // A key hok has no typed field for (`changelog`) must still be
        // substituted, exactly like upstream's PSObject walk.
        let mut root = serde_json::json!({
            "version": "1.0.0",
            "changelog": "https://ex.com/old",
            "extract_to": "old",
            "autoupdate": {
                "changelog": "https://ex.com/$version",
                "extract_to": "$version"
            }
        });
        let au = autoupdate_fixture(&root);
        let subs = subs_for("2.0.0");
        update_manifest_property(&mut root, &au, "changelog", &subs);
        update_manifest_property(&mut root, &au, "extract_to", &subs);
        assert_eq!(root["changelog"], serde_json::json!("https://ex.com/2.0.0"));
        assert_eq!(root["extract_to"], serde_json::json!("2.0.0"));
        assert_eq!(root["version"], serde_json::json!("1.0.0"));
    }

    #[test]
    fn property_only_written_when_manifest_declares_it() {
        let mut root = serde_json::json!({
            "version": "1.0.0",
            "autoupdate": { "changelog": "https://ex.com/$version" }
        });
        let au = autoupdate_fixture(&root);
        update_manifest_property(&mut root, &au, "changelog", &subs_for("2.0.0"));
        assert!(
            root.get("changelog").is_none(),
            "upstream only updates properties the manifest already declares"
        );
    }

    // ── ps_truthy / ps_array_len ───────────────────────────────────────────

    #[test]
    fn truthiness_matches_powershell() {
        assert!(!ps_truthy(None));
        assert!(!ps_truthy(Some(&serde_json::Value::Null)));
        assert!(!ps_truthy(Some(&serde_json::json!(""))));
        assert!(!ps_truthy(Some(&serde_json::json!([]))));
        assert!(ps_truthy(Some(&serde_json::json!("x"))));
        assert!(ps_truthy(Some(&serde_json::json!(["x"]))));
        assert!(!ps_truthy(Some(&serde_json::json!(0))));
        assert!(ps_truthy(Some(&serde_json::json!(1))));
    }

    #[test]
    fn array_len_matches_powershell_coercion() {
        assert_eq!(ps_array_len(&serde_json::Value::Null), 0);
        assert_eq!(ps_array_len(&serde_json::json!("a")), 1);
        assert_eq!(ps_array_len(&serde_json::json!(["a", "b"])), 2);
    }

    // ── arch_specific (lib/manifest.ps1:148-155) ───────────────────────────

    #[test]
    fn arch_specific_prefers_arch_then_falls_back() {
        let au = serde_json::json!({
            "url": "https://generic/$version",
            "architecture": { "64bit": { "url": "https://64/$version" } }
        });
        assert_eq!(
            arch_specific(&au, "64bit", "url").unwrap(),
            serde_json::json!("https://64/$version")
        );
        assert_eq!(
            arch_specific(&au, "32bit", "url").unwrap(),
            serde_json::json!("https://generic/$version")
        );
        assert!(arch_specific(&au, "arm64", "hash").is_none());
    }

    // ── end-to-end with an in-memory network fake ─────────────────────────

    #[tokio::test]
    async fn execute_uses_injected_network_fake() {
        use crate::internal::network::{Network, RequestOptions};
        use crate::test_utils::{FakeNetwork, test_session, tmpdir};
        use std::sync::Arc;

        let root = tmpdir("checkver_fake");
        let session = test_session(&root);

        // Serve a canned page for the checkver URL so no real network is hit.
        let url = "https://fake.example.com/latest.txt";
        let fake = Arc::new(FakeNetwork::new());
        fake.serve(url, b"version=2.0.0".to_vec());
        session.set_network(fake.clone()).unwrap();

        // sanity: the fake serves the canned page
        let body = fake
            .download(url, &RequestOptions::default())
            .await
            .unwrap();
        assert_eq!(String::from_utf8(body).unwrap(), "version=2.0.0");

        // A bucket dir with one manifest whose checkver points at the URL.
        let bucket_dir = root.join("bucket");
        std::fs::create_dir_all(&bucket_dir).unwrap();
        let manifest = format!(
            r#"{{"version": "1.0.0", "homepage": "https://example.com",
                "checkver": {{"url": "{url}", "regex": "version=([\\d.]+)"}}}}"#
        );
        std::fs::write(bucket_dir.join("app.json"), manifest).unwrap();

        let opts = CheckverOptions {
            dir: bucket_dir,
            app: vec!["*".to_string()],
            update: false,
            force_update: false,
            skip_updated: false,
            version: None,
            timeout: 10,
            throw_error: false,
        };

        let mut reports = Vec::new();
        execute(opts, &session, |r| reports.push(r.clone()))
            .await
            .unwrap();

        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].stem, "app");
        assert_eq!(reports[0].current, "1.0.0");
        assert_eq!(
            reports[0].new_version.as_deref(),
            Some("2.0.0"),
            "report: {:?}",
            reports[0]
        );
    }

    // ── apply_autoupdate end-to-end (S9-2 variable table + write scope) ────

    /// Write `manifest` to `<root>/app.json`, parse it, run
    /// `apply_autoupdate` against an in-memory network fake serving `canned`,
    /// and return the rewritten JSON.
    async fn run_autoupdate(
        label: &str,
        manifest_json: &str,
        version: &str,
        canned: &[(&str, &[u8])],
    ) -> serde_json::Value {
        use crate::test_utils::{FakeNetwork, test_session, tmpdir};
        use std::sync::Arc;

        let root = tmpdir(label);
        let session = test_session(&root);
        let fake = Arc::new(FakeNetwork::new());
        for (url, body) in canned {
            fake.serve(url, body.to_vec());
        }
        session.set_network(fake).unwrap();

        let path = root.join("app.json");
        std::fs::write(&path, manifest_json).unwrap();
        let manifest = manifest::parse_file(&path).unwrap();
        apply_autoupdate(&session, &path, &manifest, version, &[], &HashMap::new())
            .await
            .unwrap();
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = crate::internal::hash::ChecksumBuilder::new()
            .sha256()
            .build();
        hasher.consume(bytes);
        hasher.finalize()
    }

    /// A real-bucket shape (`main`/`extras`): the hash lives in a
    /// `$baseurl/checksums.txt` file and the regex anchors on `$basename`.
    /// Before S9-2 hok fetched the literal string `$baseurl/checksums.txt`,
    /// which cannot even be parsed as a URL.
    #[tokio::test]
    async fn autoupdate_substitutes_hash_url_and_basename() {
        let hash = "a".repeat(64);
        let body = format!("{hash}  app-2.0.0.zip\n");
        let value = run_autoupdate(
            "autoupdate_subst",
            r#"{
                "version": "1.0.0",
                "url": "https://dl.example.com/app-1.0.0.zip",
                "hash": "0000000000000000000000000000000000000000000000000000000000000000",
                "autoupdate": {
                    "url": "https://dl.example.com/app-$version.zip",
                    "hash": {
                        "url": "$baseurl/checksums.txt",
                        "regex": "([a-fA-F0-9]{64})\\s+$basename"
                    }
                }
            }"#,
            "2.0.0",
            &[("https://dl.example.com/checksums.txt", body.as_bytes())],
        )
        .await;
        assert_eq!(value["version"], serde_json::json!("2.0.0"));
        assert_eq!(
            value["url"],
            serde_json::json!("https://dl.example.com/app-2.0.0.zip")
        );
        assert_eq!(
            value["hash"],
            serde_json::json!(hash),
            "hash file found via $baseurl and matched via $basename"
        );
    }

    /// Upstream only writes a hash where the manifest already declares one.
    /// hok used to synthesise a top-level `hash` from `autoupdate.url`.
    #[tokio::test]
    async fn autoupdate_does_not_add_hash_when_manifest_has_none() {
        let value = run_autoupdate(
            "autoupdate_no_hash",
            r#"{
                "version": "1.0.0",
                "url": "https://dl.example.com/app-1.0.0.zip",
                "autoupdate": { "url": "https://dl.example.com/app-$version.zip" }
            }"#,
            "2.0.0",
            &[],
        )
        .await;
        assert_eq!(value["version"], serde_json::json!("2.0.0"));
        assert_eq!(
            value["url"],
            serde_json::json!("https://dl.example.com/app-2.0.0.zip")
        );
        assert!(
            value.get("hash").is_none(),
            "no manifest hash and no architecture -> upstream writes no hash: {value}"
        );
    }

    /// With `architecture` but no top-level `hash`, upstream updates the
    /// per-architecture hash instead.
    #[tokio::test]
    async fn autoupdate_updates_arch_hash_when_no_top_level_hash() {
        let hash = "b".repeat(64);
        let body = format!("{hash}\n");
        let value = run_autoupdate(
            "autoupdate_arch_hash",
            r#"{
                "version": "1.0.0",
                "architecture": {
                    "64bit": {
                        "url": "https://dl.example.com/64/app-1.0.0.zip",
                        "hash": "0000000000000000000000000000000000000000000000000000000000000000"
                    }
                },
                "autoupdate": {
                    "architecture": {
                        "64bit": {
                            "url": "https://dl.example.com/64/app-$version.zip",
                            "hash": {
                                "url": "$baseurl/checksums.txt",
                                "regex": "([a-fA-F0-9]{64})"
                            }
                        }
                    }
                }
            }"#,
            "2.0.0",
            &[("https://dl.example.com/64/checksums.txt", body.as_bytes())],
        )
        .await;
        assert_eq!(
            value["architecture"]["64bit"]["url"],
            serde_json::json!("https://dl.example.com/64/app-2.0.0.zip")
        );
        assert_eq!(
            value["architecture"]["64bit"]["hash"],
            serde_json::json!(hash)
        );
        assert!(value.get("hash").is_none());
    }

    /// An architecture entry that has no `hash` does not gain one: upstream's
    /// `$Manifest.architecture.$arch.hash = ...` throws there (verified
    /// 2026-09-13), and the sibling entry that *does* have a hash is updated.
    #[tokio::test]
    async fn autoupdate_does_not_add_arch_hash_when_absent() {
        let hash = "c".repeat(64);
        let body = format!("{hash}\n");
        let value = run_autoupdate(
            "autoupdate_arch_no_hash",
            r#"{
                "version": "1.0.0",
                "architecture": {
                    "64bit": {
                        "url": "https://dl.example.com/64/app-1.0.0.zip",
                        "hash": "0000000000000000000000000000000000000000000000000000000000000000"
                    },
                    "32bit": { "url": "https://dl.example.com/32/app-1.0.0.zip" }
                },
                "autoupdate": {
                    "architecture": {
                        "64bit": {
                            "url": "https://dl.example.com/64/app-$version.zip",
                            "hash": { "url": "$baseurl/checksums.txt", "regex": "([a-fA-F0-9]{64})" }
                        },
                        "32bit": { "url": "https://dl.example.com/32/app-$version.zip" }
                    }
                }
            }"#,
            "2.0.0",
            &[("https://dl.example.com/64/checksums.txt", body.as_bytes())],
        )
        .await;
        assert_eq!(
            value["architecture"]["64bit"]["hash"],
            serde_json::json!(hash)
        );
        assert_eq!(
            value["architecture"]["32bit"]["url"],
            serde_json::json!("https://dl.example.com/32/app-2.0.0.zip")
        );
        assert!(
            value["architecture"]["32bit"].get("hash").is_none(),
            "an architecture without a hash must not gain one: {value}"
        );
    }

    /// Without any `autoupdate.hash` template the file is downloaded and
    /// hashed — and (S9-2) cached under Scoop's cache dir instead of a
    /// throwaway temp copy.
    #[tokio::test]
    async fn autoupdate_download_fallback_uses_the_cache_dir() {
        use crate::test_utils::{FakeNetwork, test_session, tmpdir};
        use std::sync::Arc;

        let payload = b"payload-bytes".to_vec();
        let fake = Arc::new(FakeNetwork::new());
        fake.serve("https://dl.example.com/app-2.0.0.zip", payload.clone());
        let root = tmpdir("autoupdate_cache");
        let session = test_session(&root);
        session.set_network(fake).unwrap();
        let path = root.join("app.json");
        std::fs::write(
            &path,
            r#"{
                "version": "1.0.0",
                "url": "https://dl.example.com/app-1.0.0.zip",
                "hash": "0000000000000000000000000000000000000000000000000000000000000000",
                "autoupdate": { "url": "https://dl.example.com/app-$version.zip" }
            }"#,
        )
        .unwrap();
        let manifest = manifest::parse_file(&path).unwrap();
        apply_autoupdate(&session, &path, &manifest, "2.0.0", &[], &HashMap::new())
            .await
            .unwrap();

        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["hash"], serde_json::json!(sha256_hex(&payload)));

        let cached = session
            .config()
            .cache_path()
            .join(crate::package::cache_file_name(
                "app",
                "2.0.0",
                "https://dl.example.com/app-2.0.0.zip",
            ));
        assert!(
            cached.is_file(),
            "hash fallback must populate the Scoop cache: {}",
            cached.display()
        );
        assert_eq!(std::fs::read(&cached).unwrap(), payload);
    }

    /// The real-bucket shape with **no** `regex`: a `checksums.txt` line. Only
    /// the default `^\s*([a-fA-F0-9]+)\s*$` plus the filename fallback can
    /// resolve this — hok used to abort with "could not extract hash".
    #[tokio::test]
    async fn autoupdate_uses_the_default_hash_file_regex() {
        let hash = "a".repeat(64);
        let body = format!("{hash}  app-2.0.0.zip\n");
        // Only the checksums file is served: a download fallback would fail.
        let value = run_autoupdate(
            "autoupdate_default_regex",
            r#"{
                "version": "1.0.0",
                "url": "https://dl.example.com/app-1.0.0.zip",
                "hash": "0000000000000000000000000000000000000000000000000000000000000000",
                "autoupdate": {
                    "url": "https://dl.example.com/app-$version.zip",
                    "hash": { "url": "$baseurl/checksums.txt" }
                }
            }"#,
            "2.0.0",
            &[("https://dl.example.com/checksums.txt", body.as_bytes())],
        )
        .await;
        assert_eq!(
            value["hash"],
            serde_json::json!(hash),
            "a bare hash line must be picked up without any configured regex"
        );
    }

    /// `find` is a *regex* whose first group is the hash
    /// (lib/autoupdate.ps1:246-251), and the `$sha256` template expands to its
    /// capture group. hok used to `content.find()` the literal string.
    #[tokio::test]
    async fn autoupdate_find_is_a_regex_with_template_expansion() {
        let hash = "b".repeat(64);
        let body = format!("{hash}  app-2.0.0.zip\n");
        let value = run_autoupdate(
            "autoupdate_find_regex",
            r#"{
                "version": "1.0.0",
                "url": "https://dl.example.com/app-1.0.0.zip",
                "hash": "0000000000000000000000000000000000000000000000000000000000000000",
                "autoupdate": {
                    "url": "https://dl.example.com/app-$version.zip",
                    "hash": {
                        "url": "$baseurl/SHA256SUMS",
                        "find": "$sha256\\s+$basename"
                    }
                }
            }"#,
            "2.0.0",
            &[("https://dl.example.com/SHA256SUMS", body.as_bytes())],
        )
        .await;
        assert_eq!(value["hash"], serde_json::json!(hash));
    }

    /// A hash file that yields nothing is not an error: upstream downloads the
    /// artefact and computes SHA-256 (lib/autoupdate.ps1:304-329).
    #[tokio::test]
    async fn autoupdate_falls_back_to_downloading_when_the_hash_file_yields_nothing() {
        let payload = b"payload-bytes".to_vec();
        let value = run_autoupdate(
            "autoupdate_hash_file_miss",
            r#"{
                "version": "1.0.0",
                "url": "https://dl.example.com/app-1.0.0.zip",
                "hash": "0000000000000000000000000000000000000000000000000000000000000000",
                "autoupdate": {
                    "url": "https://dl.example.com/app-$version.zip",
                    "hash": { "url": "$baseurl/checksums.txt" }
                }
            }"#,
            "2.0.0",
            &[
                ("https://dl.example.com/checksums.txt", b"no hash here"),
                ("https://dl.example.com/app-2.0.0.zip", &payload),
            ],
        )
        .await;
        assert_eq!(value["hash"], serde_json::json!(sha256_hex(&payload)));
    }

    /// Any autoupdate key is substituted, not just the typed ones — this is
    /// what upstream's PSObject walk buys (`changelog`, `extract_to`, ...).
    #[tokio::test]
    async fn autoupdate_substitutes_untyped_properties() {
        let payload = b"payload".to_vec();
        let value = run_autoupdate(
            "autoupdate_untyped",
            r#"{
                "version": "1.0.0",
                "url": "https://dl.example.com/app-1.0.0.zip",
                "hash": "0000000000000000000000000000000000000000000000000000000000000000",
                "changelog": "https://ex.com/old",
                "extract_to": "old",
                "autoupdate": {
                    "url": "https://dl.example.com/app-$version.zip",
                    "changelog": "https://ex.com/$version",
                    "extract_to": "$cleanVersion"
                }
            }"#,
            "2.0.0",
            &[("https://dl.example.com/app-2.0.0.zip", &payload)],
        )
        .await;
        assert_eq!(
            value["changelog"],
            serde_json::json!("https://ex.com/2.0.0")
        );
        assert_eq!(value["extract_to"], serde_json::json!("200"));
    }

    // ── execute end-to-end: github shortcut + checkver.script ──────────────

    /// Run `execute` over a single-manifest bucket dir with a network fake.
    async fn run_execute(
        label: &str,
        manifest_json: &str,
        canned: &[(&str, &[u8])],
    ) -> Vec<CheckverReport> {
        use crate::test_utils::{FakeNetwork, test_session, tmpdir};
        use std::sync::Arc;

        let root = tmpdir(label);
        let session = test_session(&root);
        let fake = Arc::new(FakeNetwork::new());
        for (url, body) in canned {
            fake.serve(url, body.to_vec());
        }
        session.set_network(fake).unwrap();

        let bucket_dir = root.join("bucket");
        std::fs::create_dir_all(&bucket_dir).unwrap();
        std::fs::write(bucket_dir.join("app.json"), manifest_json).unwrap();

        let opts = CheckverOptions {
            dir: bucket_dir,
            app: vec!["*".to_string()],
            update: false,
            force_update: false,
            skip_updated: false,
            version: None,
            timeout: 30,
            throw_error: false,
        };
        let mut reports = Vec::new();
        execute(opts, &session, |r| reports.push(r.clone()))
            .await
            .unwrap();
        reports
    }

    /// `{github: "https://github.com/o/r", regex: ...}` must fetch the HTML
    /// release page and run the user regex on it. The URL is only rewritten to
    /// `api.github.com` when a token is configured (`bin/checkver.ps1:175-178`);
    /// this session has none, so no anonymous API call is made. hok used to
    /// rewrite unconditionally and feed the regex a JSON tag, which no real
    /// manifest regex can match.
    #[tokio::test]
    async fn github_with_a_custom_regex_uses_the_html_page() {
        let html = br#"<a href="/o/r/releases/tag/v9.9.9">release</a>"#;
        let reports = run_execute(
            "cv_gh_regex",
            r#"{
                "version": "1.0.0",
                "homepage": "https://github.com/o/r",
                "checkver": {
                    "github": "https://github.com/o/r",
                    "regex": "releases/tag/v([\\d.]+)"
                }
            }"#,
            // Only the web page is served: an API rewrite would fail the test.
            &[("https://github.com/o/r/releases/latest", html)],
        )
        .await;
        assert_eq!(reports.len(), 1);
        assert_eq!(
            reports[0].new_version.as_deref(),
            Some("9.9.9"),
            "report: {:?}",
            reports[0]
        );
    }

    /// The `checkver: "github"` string shortcut without a token fetches the
    /// HTML page and applies upstream's default `/releases/tag/...` regex
    /// (no anonymous API call).
    #[tokio::test]
    async fn github_string_shortcut_without_a_token_uses_the_html_page() {
        let html = br#"<a href="/o/r/releases/tag/v9.9.9">release</a>"#;
        let reports = run_execute(
            "cv_gh_string",
            r#"{
                "version": "1.0.0",
                "homepage": "https://github.com/o/r",
                "checkver": "github"
            }"#,
            &[("https://github.com/o/r/releases/latest", html)],
        )
        .await;
        assert_eq!(reports[0].new_version.as_deref(), Some("9.9.9"));
    }

    /// `cbd7c64b`: an `api.github.com` URL (no token rewrite needed) gets the
    /// `$.tag_name` jsonpath **and** the API-mode default regex — the manifest
    /// declares neither. `run_execute`'s session has no token, so this exercises
    /// the "already the API" branch of the URL decision, not the rewrite.
    #[tokio::test]
    async fn github_api_mode_defaults_to_tag_name_and_the_api_regex() {
        let reports = run_execute(
            "cv_gh_api_default",
            r#"{
                "version": "1.0.0",
                "homepage": "https://www.7-zip.org",
                "checkver": {
                    "github": "https://api.github.com/repos/o/r/releases/latest"
                }
            }"#,
            &[(
                "https://api.github.com/repos/o/r/releases/latest",
                // `name` comes first on purpose: without the injected
                // `$.tag_name` jsonpath the default regex would run over the
                // whole body and report `9.9.9` instead.
                br#"{"name":"9.9.9","tag_name":"v1.2.3"}"#,
            )],
        )
        .await;
        assert_eq!(
            reports[0].new_version.as_deref(),
            Some("1.2.3"),
            "jsonpath `$.tag_name` extracts `v1.2.3`, then the API default regex \
             drops the `v`; report: {:?}",
            reports[0]
        );
    }

    /// `d452e8f6`: with `checkver.script`, the API-mode jsonpath default is not
    /// injected — the script's output is the page, and the regex parses it.
    #[tokio::test]
    async fn github_api_mode_with_a_script_skips_the_default_jsonpath() {
        let reports = run_execute(
            "cv_gh_api_script",
            r#"{
                "version": "1.0.0",
                "homepage": "https://www.7-zip.org",
                "checkver": {
                    "github": "https://api.github.com/repos/o/r/releases/latest",
                    "script": "Write-Output 'v9.9.9'"
                }
            }"#,
            &[("https://api.github.com/repos/o/r/releases/latest", b"{}")],
        )
        .await;
        assert_eq!(
            reports[0].new_version.as_deref(),
            Some("9.9.9"),
            "a `$.tag_name` jsonpath would fail on the script's plain-text page; \
             report: {:?}",
            reports[0]
        );
    }

    /// An invalid `checkver.github` skips the manifest before any download
    /// (`bin/checkver.ps1:165-168`): the only report is the error naming the
    /// field, and the unserved URL is never fetched.
    #[tokio::test]
    async fn github_map_with_an_invalid_url_skips_the_manifest() {
        let reports = run_execute(
            "cv_gh_invalid",
            r#"{
                "version": "1.0.0",
                "homepage": "https://www.7-zip.org",
                "checkver": {"github": "https://gitlab.com/o/r"}
            }"#,
            &[],
        )
        .await;
        assert_eq!(reports.len(), 1, "got {reports:?}");
        assert_eq!(reports[0].severity, ReportSeverity::Error);
        assert!(reports[0].new_version.is_none());
        let message = reports[0].message.as_deref().unwrap_or_default();
        assert!(message.contains("checkver.github"), "got {message}");
    }

    /// `20a178b1`: when the download fails but `checkver.script` exists,
    /// upstream warns and runs the script instead of skipping the manifest.
    #[tokio::test]
    async fn checkver_script_runs_when_the_download_fails() {
        // Nothing is served, so the URL fetch fails.
        let reports = run_execute(
            "cv_script_after_error",
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "checkver": {
                    "url": "https://example.com/missing.txt",
                    "regex": "v([\\d.]+)",
                    "script": "Write-Output 'v9.9.9'"
                }
            }"#,
            &[],
        )
        .await;
        assert!(
            reports.iter().any(|r| r.severity == ReportSeverity::Warn),
            "the fallback must warn; got {reports:?}"
        );
        let ver = reports.iter().find_map(|r| r.new_version.clone());
        assert_eq!(
            ver.as_deref(),
            Some("9.9.9"),
            "the script must still run after a failed fetch; got {reports:?}"
        );
    }

    /// Without a script a failed download is still a hard per-manifest error
    /// (upstream's `next "$($err.message)..."; continue`).
    #[tokio::test]
    async fn download_failure_without_a_script_is_an_error() {
        let reports = run_execute(
            "cv_no_script_error",
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "checkver": {
                    "url": "https://example.com/missing.txt",
                    "regex": "v([\\d.]+)"
                }
            }"#,
            &[],
        )
        .await;
        assert_eq!(reports.len(), 1, "got {reports:?}");
        assert_eq!(reports[0].severity, ReportSeverity::Error);
        assert!(reports[0].new_version.is_none());
    }

    /// `checkver.script` runs in a child PowerShell, so upstream's in-scope
    /// `$json`, `$app` and `$expected_ver` are reconstructed as a preamble.
    /// The download still happens first (the URL is served here).
    #[tokio::test]
    async fn checkver_script_receives_app_expected_ver_and_json() {
        let reports = run_execute(
            "cv_script",
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "checkver": {
                    "url": "https://example.com/latest.txt",
                    "script": "Write-Output ($app + '|' + $expected_ver + '|' + $json.version)"
                }
            }"#,
            &[("https://example.com/latest.txt", b"ignored body")],
        )
        .await;
        assert_eq!(
            reports[0].new_version.as_deref(),
            Some("app|1.0.0|1.0.0"),
            "the script must see $app, $expected_ver and $json ($json.version)"
        );
    }

    #[test]
    fn script_preamble_quotes_single_quotes() {
        let preamble = script_preamble("app's", "1.0.0", std::path::Path::new(r"C:\tmp\it's.json"));
        assert!(
            preamble.contains("$json = Get-Content -LiteralPath"),
            "got {preamble}"
        );
        assert!(preamble.contains(r"'C:\tmp\it''s.json'"), "got {preamble}");
        assert!(preamble.contains("$app = 'app''s'"));
        assert!(preamble.contains("$expected_ver = '1.0.0'"));
    }

    // ── completion with an empty `pending` list ───────────────────────────
    //
    // Every path that leaves `pending` empty used to park forever in the
    // `buffer_unordered(pending.len())` of Phase 2 (capacity 0 never polls its
    // source stream). These tests pin the four reachable triggers: an empty
    // bucket dir, a dir whose manifests all lack `checkver`, an app pattern
    // matching nothing, and `--version` (which handles every manifest inline).

    /// Generous on purpose: the assertion is "`execute` returns *at all*", not
    /// "it is fast". Without the capacity guard these futures never resolve, so
    /// a failure here is a timeout rather than a hung test suite.
    const COMPLETION_BUDGET: std::time::Duration = std::time::Duration::from_secs(10);

    /// A bucket dir holding `files` (file name → manifest JSON) plus a session
    /// whose network serves nothing: any request fails offline instead of
    /// reaching out.
    fn offline_bucket(label: &str, files: &[(&str, &str)]) -> (std::path::PathBuf, crate::Session) {
        use crate::test_utils::{FakeNetwork, test_session, tmpdir};
        use std::sync::Arc;

        let root = tmpdir(label);
        let session = test_session(&root);
        session.set_network(Arc::new(FakeNetwork::new())).unwrap();

        let bucket_dir = root.join("bucket");
        std::fs::create_dir_all(&bucket_dir).unwrap();
        for (name, json) in files {
            std::fs::write(bucket_dir.join(name), json).unwrap();
        }
        (bucket_dir, session)
    }

    /// Run `execute` under `COMPLETION_BUDGET` and return the reports.
    async fn execute_within_budget(
        options: CheckverOptions,
        session: &crate::Session,
    ) -> Vec<CheckverReport> {
        let mut reports = Vec::new();
        tokio::time::timeout(
            COMPLETION_BUDGET,
            execute(options, session, |r| reports.push(r.clone())),
        )
        .await
        .expect("checkver must terminate with an empty `pending` list")
        .unwrap();
        reports
    }

    fn options_for(
        dir: std::path::PathBuf,
        app: &[&str],
        version: Option<&str>,
        update: bool,
    ) -> CheckverOptions {
        CheckverOptions {
            dir,
            app: app.iter().map(|s| s.to_string()).collect(),
            update,
            force_update: false,
            skip_updated: false,
            version: version.map(str::to_string),
            timeout: 10,
            throw_error: false,
        }
    }

    /// A bucket dir with no manifest at all finishes without reporting anything.
    #[tokio::test]
    async fn empty_bucket_dir_completes() {
        let (dir, session) = offline_bucket("cv_empty", &[]);
        let reports = execute_within_budget(options_for(dir, &["*"], None, false), &session).await;
        assert!(reports.is_empty(), "got {reports:?}");
    }

    /// A dir whose manifests declare no `checkver` finishes and reports nothing.
    #[tokio::test]
    async fn bucket_without_any_checkver_completes() {
        let (dir, session) = offline_bucket(
            "cv_none",
            &[
                (
                    "app.json",
                    r#"{"version":"1.0.0","homepage":"https://example.com"}"#,
                ),
                (
                    "other.json",
                    r#"{"version":"2.0.0","homepage":"https://example.com"}"#,
                ),
            ],
        );
        let reports = execute_within_budget(options_for(dir, &["*"], None, false), &session).await;
        assert!(reports.is_empty(), "got {reports:?}");
    }

    /// An app pattern that matches no stem finishes (the walker yields nothing).
    #[tokio::test]
    async fn app_glob_matching_nothing_completes() {
        let (dir, session) = offline_bucket(
            "cv_nomatch",
            &[(
                "app.json",
                r#"{"version":"1.0.0","homepage":"https://example.com","checkver":"https://example.com/v([\\d.]+)"}"#,
            )],
        );
        let reports =
            execute_within_budget(options_for(dir, &["does-not-exist"], None, false), &session)
                .await;
        assert!(reports.is_empty(), "got {reports:?}");
    }

    /// `--version` handles every manifest inline, so `pending` is empty and the
    /// run must still terminate. Version detection is skipped entirely, hence
    /// no network is touched.
    #[tokio::test]
    async fn version_override_completes() {
        let (dir, session) = offline_bucket(
            "cv_version",
            &[(
                "app.json",
                r#"{"version":"1.0.0","homepage":"https://example.com","checkver":"https://example.com/v([\\d.]+)"}"#,
            )],
        );
        let reports = execute_within_budget(
            options_for(dir.clone(), &["app"], Some("2.0.0"), false),
            &session,
        )
        .await;

        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].new_version.as_deref(), Some("2.0.0"));
        assert!(!reports[0].updated);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &std::fs::read_to_string(dir.join("app.json")).unwrap()
            )
            .unwrap()["version"],
            serde_json::json!("1.0.0"),
            "a plain --version run must not rewrite the manifest"
        );
    }

    /// `--version --update` rewrites the manifest inline and then terminates
    /// (the manifest carries no `autoupdate`, so the rewrite stays offline).
    #[tokio::test]
    async fn version_override_with_update_completes() {
        let (dir, session) = offline_bucket(
            "cv_version_update",
            &[(
                "app.json",
                r#"{"version":"1.0.0","homepage":"https://example.com","checkver":"https://example.com/v([\\d.]+)"}"#,
            )],
        );
        let reports = execute_within_budget(
            options_for(dir.clone(), &["app"], Some("2.0.0"), true),
            &session,
        )
        .await;

        assert_eq!(reports.len(), 1);
        assert!(reports[0].updated, "got {:?}", reports[0]);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &std::fs::read_to_string(dir.join("app.json")).unwrap()
            )
            .unwrap()["version"],
            serde_json::json!("2.0.0")
        );
    }
}
