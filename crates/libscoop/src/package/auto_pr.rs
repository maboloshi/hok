//! CI auto-PR: update manifests and open pull requests via the GitHub API.
//!
//! This module contains the core business logic extracted from the `auto-pr`
//! CLI command. It is designed to run in CI environments (e.g., GitHub
//! Actions) and uses the GitHub API exclusively — no local `git` or `hub`
//! binary required.
//!
//! # Design
//!
//! - **GitHub API only**: All remote operations (commits, branch creation,
//!   pull-request creation) go through [`crate::internal::github`].
//! - **Per-manifest PR creation**: Each updated manifest gets its own PR
//!   with an auto-generated title, body, and branch derived from the package
//!   name and version.
//! - **Fork-based workflow**: Designed for the ScoopInstaller fork + PR
//!   workflow — commits target a fork branch, then a PR is opened against
//!   the upstream default branch.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Fallible};

use crate::Session;
use crate::internal::github;
use crate::internal::url;
use crate::package::checkver::CheckverReport;
use crate::package::manifest_walker;

// ---------------------------------------------------------------------------
// Public configuration type
// ---------------------------------------------------------------------------

/// Plain-data configuration for [`run_auto_pr`].
///
/// All fields use standard library types — no CLI-specific structures.
pub struct AutoPrConfig {
    /// GitHub API token (e.g. `GITHUB_TOKEN`).
    pub token: String,
    /// Bucket directory to scan for manifests.
    pub dir: PathBuf,
    /// When `true`, commit updated manifests directly to `origin_branch`.
    pub push: bool,
    /// When `true`, create a pull request for each updated manifest.
    pub request: bool,
    /// Upstream `owner/repo` (used when `request` is `true`).
    pub upstream_repo_nwo: String,
    /// Upstream base branch for PR targets (used when `request` is `true`).
    pub upstream_branch: String,
    /// Origin branch name (the branch that currently holds the manifests).
    pub origin_branch: String,
    /// Commit message template; `<app>` and `<version>` are substituted.
    pub message: String,
    /// Skip manifests whose version did not change.
    pub skip_updated: bool,
    /// Additional manifests to force-update even if already up-to-date.
    pub special: Vec<String>,
    /// GitHub App slug (e.g. `my-bot`); when set, the DCO sign-off is
    /// generated for the corresponding bot user (`my-bot[bot]`) instead
    /// of the authenticated user.
    pub app_slug: Option<String>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run the full auto-PR pipeline.
///
/// 1. Snapshots manifests in `config.dir`.
/// 2. Runs `checkver --update` to refresh versions, streaming the per-manifest
///    [`CheckverReport`]s to `on_report` as each completes.
/// 3. Detects which manifests changed.
/// 4. For each changed manifest either pushes a commit or creates a PR,
///    depending on `config.push` / `config.request`.
///
/// `on_report` is the presentation hook: `None` keeps the compact
/// progress/ok markers, `Some(callback)` streams the checkver reports to the
/// caller, which decides how (or whether) to render them.
pub async fn run_auto_pr(
    config: AutoPrConfig,
    session: &Session,
    mut on_report: Option<impl FnMut(&CheckverReport)>,
) -> Fallible<()> {
    // Validate token
    if config.token.is_empty() {
        return Err(Error::Custom(
            hok_i18n::t!("cmd.auto_err_token").to_string(),
        ));
    }

    // Validate modes
    if !config.push && !config.request {
        return Err(Error::Custom(hok_i18n::t!("cmd.auto_err_mode").to_string()));
    }
    if config.request && config.upstream_repo_nwo.is_empty() {
        return Err(Error::Custom(
            hok_i18n::t!("cmd.auto_err_upstream").to_string(),
        ));
    }

    // Resolve repository info (owner/repo)
    let repo_nwo = resolve_repo_nwo()?;

    let origin_owner = repo_nwo.split('/').next().unwrap_or("").to_string();

    session.output().header(hok_i18n::t!("cmd.auto_pr_header"));
    session.output().named("Repository", &repo_nwo);
    session.output().named("Branch", &config.origin_branch);
    if config.request {
        let upstream = format!("{}:{}", config.upstream_repo_nwo, config.upstream_branch);
        session.output().named("Upstream", &upstream);
    }

    // Resolve absolute directory
    let dir = if config.dir.is_relative() {
        std::env::current_dir()
            .unwrap_or_default()
            .join(&config.dir)
    } else {
        config.dir.clone()
    };
    if !dir.is_dir() {
        return Err(Error::Custom(
            hok_i18n::t!("cmd.dir_not_found", path = dir.display()).to_string(),
        ));
    }

    // Snapshot manifests before running checkver
    session.output().progress(
        hok_i18n::t!("cmd.auto_pr_snapshotting"),
        hok_i18n::t!("cmd.auto_pr_manifests"),
    );
    let before = read_manifests(session, &dir)?;
    session.output().ok();

    // Run checkver to update all manifests. Reports stream to the caller's
    // callback; without one, the compact progress/ok pair is shown instead.
    if on_report.is_none() {
        session.output().progress(
            hok_i18n::t!("cmd.auto_pr_checking"),
            hok_i18n::t!("cmd.auto_pr_for_updates"),
        );
    }
    let cv_args = crate::package::checkver::CheckverOptions {
        dir: dir.clone(),
        app: vec!["*".to_string()],
        update: true,
        force_update: false,
        skip_updated: config.skip_updated,
        version: None,
        timeout: 30,
        throw_error: false,
    };
    let _ = crate::package::checkver::execute(cv_args, session, |report| {
        if let Some(cb) = on_report.as_mut() {
            cb(report);
        }
    })
    .await;
    if on_report.is_none() {
        session.output().ok();
    }

    // Force-update special manifests
    for name in &config.special {
        if on_report.is_none() {
            session
                .output()
                .progress(hok_i18n::t!("cmd.auto_pr_forcing"), name);
        }
        let cv_args = crate::package::checkver::CheckverOptions {
            dir: dir.clone(),
            app: vec![name.to_string()],
            update: true,
            force_update: true,
            skip_updated: false,
            version: None,
            timeout: 30,
            throw_error: false,
        };
        let _ = crate::package::checkver::execute(cv_args, session, |report| {
            if let Some(cb) = on_report.as_mut() {
                cb(report);
            }
        })
        .await;
        if on_report.is_none() {
            session.output().ok();
        }
    }

    // Detect which manifests changed
    let changed = detect_changes(&before);
    if changed.is_empty() {
        session
            .output()
            .info(hok_i18n::t!("cmd.auto_pr_no_changes"));
        return Ok(());
    }
    session
        .output()
        .info(hok_i18n::t!("cmd.manifests_updated", count = changed.len()));

    // Process each changed manifest
    let mut success_count = 0u32;
    let mut skip_count = 0u32;

    for manifest_path in &changed {
        let manifest_str = match std::fs::read_to_string(manifest_path) {
            Ok(s) => s,
            Err(e) => {
                session.output().error(hok_i18n::t!(
                    "cmd.failed_read_file",
                    path = manifest_path.display(),
                    error = e
                ));
                continue;
            }
        };
        // Tolerant read: upstream `bin/auto-pr.ps1:190` uses `parse_json`.
        let json: serde_json::Value = match crate::internal::json::parse_str(&manifest_str) {
            Ok(v) => v,
            Err(e) => {
                session.output().error(hok_i18n::t!(
                    "cmd.failed_parse_file",
                    path = manifest_path.display(),
                    error = e
                ));
                continue;
            }
        };
        let version = match json["version"].as_str() {
            Some(v) => v.to_string(),
            None => {
                session.output().error(hok_i18n::t!(
                    "cmd.manifest_no_version_field",
                    path = manifest_path.display()
                ));
                continue;
            }
        };
        let homepage = json["homepage"].as_str().unwrap_or("").to_string();
        let app = manifest_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let commit_msg = config
            .message
            .replace("<app>", &app)
            .replace("<version>", &version);

        // Repo-relative path for the file (forward slashes for GitHub API)
        let repo_relative = repo_relative_path(manifest_path);
        let repo_path_str = repo_relative.to_string_lossy().replace('\\', "/");

        if config.push {
            // Push mode: commit directly to origin branch
            session.output().progress(
                hok_i18n::t!("cmd.auto_pr_pushing"),
                format!("{} ({})", app, version),
            );

            let parent_sha =
                match github::get_ref_sha(&repo_nwo, &config.origin_branch, &config.token).await {
                    Ok(s) => s,
                    Err(e) => {
                        session.output().error(hok_i18n::t!(
                            "cmd.auto_pr_head_sha_failed",
                            app = app,
                            error = e
                        ));
                        continue;
                    }
                };

            let dco = github::set_dco_signature(&config.token, config.app_slug.as_deref()).await;

            match github::graphql_commit_push(
                &repo_nwo,
                &config.origin_branch,
                &repo_path_str,
                &commit_msg,
                &dco,
                &parent_sha,
                &config.token,
            )
            .await
            {
                Ok(url) => {
                    session.output().done(hok_i18n::t!(
                        "cmd.auto_pr_pushed",
                        app = app,
                        version = version,
                        url = url
                    ));
                    success_count += 1;
                }
                Err(e) => {
                    session.output().error(hok_i18n::t!(
                        "cmd.auto_pr_commit_failed",
                        app = app,
                        error = e
                    ));
                }
            }
        } else {
            // Request mode: create branch + PR
            let branch_name = format!("manifest/{}-{}", app, version);

            // Skip if the remote branch already exists
            if let Ok(Some(_)) =
                github::get_ref_sha_optional(&repo_nwo, &branch_name, &config.token).await
            {
                session.output().warn(hok_i18n::t!(
                    "cmd.auto_pr_branch_exists",
                    app = app,
                    version = version
                ));
                skip_count += 1;
                continue;
            }

            session.output().progress(
                hok_i18n::t!("cmd.auto_pr_creating"),
                format!("{} ({})", app, version),
            );

            let parent_sha =
                match github::get_ref_sha(&repo_nwo, &config.origin_branch, &config.token).await {
                    Ok(s) => s,
                    Err(e) => {
                        session.output().error(hok_i18n::t!(
                            "cmd.auto_pr_head_sha_failed",
                            app = app,
                            error = e
                        ));
                        continue;
                    }
                };

            if let Err(e) =
                github::create_ref(&repo_nwo, &branch_name, &parent_sha, &config.token).await
            {
                session.output().error(hok_i18n::t!(
                    "cmd.auto_pr_create_branch_failed",
                    app = app,
                    error = e
                ));
                continue;
            }

            let dco = github::set_dco_signature(&config.token, config.app_slug.as_deref()).await;

            if let Err(e) = github::graphql_commit_push(
                &repo_nwo,
                &branch_name,
                &repo_path_str,
                &commit_msg,
                &dco,
                &parent_sha,
                &config.token,
            )
            .await
            {
                session.output().error(hok_i18n::t!(
                    "cmd.auto_pr_commit_failed",
                    app = app,
                    error = e
                ));
                continue;
            }

            let pr_body = format!(
                "{}\n\n\
                 Hello lovely humans,\n\
                 a new version of [{}]({}) is available.\n\n\
                 | State       | Update :rocket: |\n\
                 | :---------- | :-------------- |\n\
                 | New version | {}              |\n",
                commit_msg, app, homepage, version
            );
            let head_ref = format!("{}:{}", origin_owner, branch_name);

            match github::create_pull_request(
                &config.upstream_repo_nwo,
                &config.upstream_branch,
                &head_ref,
                &commit_msg,
                &pr_body,
                &config.token,
            )
            .await
            {
                Ok(pr_url) => {
                    session.output().done(hok_i18n::t!(
                        "cmd.auto_pr_pr_created",
                        app = app,
                        version = version,
                        url = pr_url
                    ));
                    success_count += 1;
                }
                Err(e) => {
                    session.output().error(hok_i18n::t!(
                        "cmd.auto_pr_create_pr_failed",
                        app = app,
                        error = e
                    ));
                }
            }
        }
    }

    // Summary
    if config.push {
        session.output().info(hok_i18n::t!(
            "cmd.auto_pr_completed_push",
            pushed = success_count,
            skipped = changed.len() - success_count as usize
        ));
    } else {
        session.output().info(hok_i18n::t!(
            "cmd.auto_pr_completed_pr",
            created = success_count,
            skipped = changed.len() - success_count as usize - skip_count as usize,
            existed = skip_count
        ));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

/// Read all JSON manifest files in `dir` into a map of `path → raw bytes`.
fn read_manifests(session: &Session, dir: &Path) -> Fallible<HashMap<PathBuf, Vec<u8>>> {
    let mut map = HashMap::new();
    let manifest_paths = match manifest_walker::discover(dir) {
        Ok(paths) => paths,
        Err(e) => {
            session.output().error(hok_i18n::t!(
                "cmd.failed_read_dir",
                path = dir.display(),
                error = e
            ));
            return Ok(map);
        }
    };

    for path in manifest_paths {
        match std::fs::read(&path) {
            Ok(content) => {
                map.insert(path, content);
            }
            Err(e) => {
                session.output().error(hok_i18n::t!(
                    "cmd.failed_read_file",
                    path = path.display(),
                    error = e
                ));
            }
        }
    }
    Ok(map)
}

/// Return the subset of manifest paths whose content changed since the
/// `before` snapshot was taken.
fn detect_changes(before: &HashMap<PathBuf, Vec<u8>>) -> Vec<PathBuf> {
    let mut changed = Vec::new();
    for (path, before_content) in before {
        let current = match std::fs::read(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if current != *before_content {
            changed.push(path.clone());
        }
    }
    changed
}

// ---------------------------------------------------------------------------
// Repository / path helpers
// ---------------------------------------------------------------------------

/// Resolve the repository NWO (`owner/name`) from the `GITHUB_REPOSITORY`
/// environment variable (set by GitHub Actions) or from the git remote URL.
fn resolve_repo_nwo() -> Fallible<String> {
    if let Ok(repo) = std::env::var("GITHUB_REPOSITORY")
        && !repo.is_empty()
    {
        return Ok(repo);
    }

    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .map_err(|_| {
            Error::Custom("Cannot determine repository. Set GITHUB_REPOSITORY env var.".to_string())
        })?;

    if !output.status.success() {
        return Err(Error::Custom(
            "Cannot determine repository. Set GITHUB_REPOSITORY env var.".to_string(),
        ));
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if let Some((owner, repo)) = url::github_owner_repo(&url) {
        Ok(format!("{owner}/{repo}"))
    } else {
        Err(Error::Custom(format!(
            "Cannot parse repository NWO from remote URL: {}",
            url
        )))
    }
}

/// Make `path` relative to the current working directory, returning the
/// original path unchanged if stripping the prefix fails.
fn repo_relative_path(path: &Path) -> PathBuf {
    if let Ok(cwd) = std::env::current_dir()
        && let Ok(relative) = path.strip_prefix(&cwd)
    {
        return relative.to_path_buf();
    }
    path.to_path_buf()
}
