use clap::Parser;
use libscoop::Session;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{output, Result};

/// Auto-update manifests and create pull-requests via GitHub API (CI mode)
///
/// This command runs in CI environments (e.g., GitHub Actions) and uses the
/// GitHub API exclusively — no git/hub binary required.
#[derive(Debug, Parser)]
#[clap(name = "ci-auto-pr", arg_required_else_help = true)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = "bucket")]
    pub(crate) dir: PathBuf,

    /// Upstream repository with target branch (<user>/<repo>:<branch>)
    #[arg(short = 'u', long)]
    pub(crate) upstream: Option<String>,

    /// Push updates directly to origin branch
    #[arg(short = 'p', long)]
    pub(crate) push: bool,

    /// Create pull-requests for each update
    #[arg(short = 'r', long)]
    pub(crate) request: bool,

    /// Origin (local) branch name
    #[arg(short = 'o', long, default_value = "master")]
    pub(crate) origin_branch: String,

    /// Commit message format (<app> and <version> are replaced)
    #[arg(short = 'm', long, default_value = "<app>: Update to version <version>")]
    pub(crate) message: String,

    /// Skip manifests that are already up-to-date
    #[arg(short = 's', long = "skip-updated")]
    pub(crate) skip_updated: bool,

    /// Force update on these special snowflakes (comma-separated)
    #[arg(long)]
    pub(crate) special: Option<String>,

    /// GitHub Token (default from GITHUB_TOKEN env var)
    #[arg(short = 'T', long)]
    pub(crate) token: Option<String>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn execute(args: Args, session: &Session) -> Result<()> {
    // 1. Get token
    let token = args.token.clone().unwrap_or_else(|| {
        std::env::var("GITHUB_TOKEN").unwrap_or_default()
    });
    if token.is_empty() {
        output::err(rust_i18n::t!("cmd.auto_err_token"));
        return Ok(());
    }

    // 2. Validate modes
    if !args.push && !args.request {
        output::err(rust_i18n::t!("cmd.auto_err_mode"));
        return Ok(());
    }
    if args.request && args.upstream.is_none() {
        output::err(rust_i18n::t!("cmd.auto_err_upstream"));
        return Ok(());
    }

    let upstream = args.upstream.as_deref().unwrap_or("");
    let (upstream_repo_nwo, upstream_branch) = if args.request {
        let parts: Vec<&str> = upstream.splitn(2, ':').collect();
        (parts[0].to_string(), parts.get(1).unwrap_or(&"master").to_string())
    } else {
        (String::new(), String::new())
    };

    // 3. Resolve repository info
    let repo_nwo = match resolve_repo_nwo() {
        Ok(r) => r,
        Err(e) => {
            output::err(format!("{}", e));
            return Ok(());
        }
    };
    let origin_owner = repo_nwo.split('/').next().unwrap_or("").to_string();

    output::header(rust_i18n::t!("cmd.auto_pr_header"));
    output::named("Repository", &repo_nwo);
    output::named("Branch", &args.origin_branch);
    if args.request {
        output::named("Upstream", upstream);
    }

    // 4. Prepare directory
    let dir = if args.dir.is_relative() {
        let cwd = std::env::current_dir().unwrap_or_default();
        cwd.join(&args.dir)
    } else {
        args.dir.clone()
    };
    if !dir.is_dir() {
        output::err(rust_i18n::t!("cmd.checkver_err_dir", path = dir.display()));
        return Ok(());
    }

    // 5. Snapshot manifests before checkver
    output::progress("Snapshoting", "manifests");
    let before = read_manifests(&dir)?;
    output::ok();

    // 6. Run checkver update
    output::progress("Checking", "for updates");
    let cv_args = crate::cmd::checkver::Args {
        dir: dir.clone(),
        app: vec!["*".to_string()],
        update: true,
        force_update: false,
        skip_updated: args.skip_updated,
        version: None,
        timeout: 30,
    };
    let _ = crate::cmd::checkver::execute(cv_args, session);
    output::ok();

    // 6b. Force update special snowflakes
    if let Some(ref special) = args.special {
        for name in special.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            output::progress("Forcing", name);
            let cv_args = crate::cmd::checkver::Args {
                dir: dir.clone(),
                app: vec![name.to_string()],
                update: true,
                force_update: true,
                skip_updated: false,
                version: None,
                timeout: 30,
            };
            let _ = crate::cmd::checkver::execute(cv_args, session);
            output::ok();
        }
    }

    // 7. Detect changes
    let changed = detect_changes(&dir, &before);
    if changed.is_empty() {
        output::info(rust_i18n::t!("cmd.auto_pr_no_changes"));
        return Ok(());
    }

    output::info(format!("{} manifest(s) updated", changed.len()));

    // 8. Process each changed manifest
    let mut success_count = 0u32;
    let mut skip_count = 0u32;

    for manifest_path in &changed {
        let manifest_str = match std::fs::read_to_string(manifest_path) {
            Ok(s) => s,
            Err(e) => {
                output::err(format!("read {}: {}", manifest_path.display(), e));
                continue;
            }
        };
        let json: serde_json::Value = match serde_json::from_str(&manifest_str) {
            Ok(v) => v,
            Err(e) => {
                output::err(format!("parse {}: {}", manifest_path.display(), e));
                continue;
            }
        };
        let version = match json["version"].as_str() {
            Some(v) => v.to_string(),
            None => {
                output::err(format!("{}: no version field", manifest_path.display()));
                continue;
            }
        };
        let homepage = json["homepage"].as_str().unwrap_or("").to_string();
        let app = manifest_path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let commit_msg = args.message
            .replace("<app>", &app)
            .replace("<version>", &version);

        // Compute repo-relative path for the file
        let repo_relative = repo_relative_path(manifest_path);
        let repo_path_str = repo_relative.to_string_lossy().replace('\\', "/");

        if args.push {
            // Push mode: commit directly to origin branch
            output::progress("Pushing", &format!("{} ({})", app, version));

            let parent_sha = match get_ref_sha(&repo_nwo, &args.origin_branch, &token) {
                Ok(s) => s,
                Err(e) => {
                    output::err(format!("{}: get HEAD SHA failed: {}", app, e));
                    continue;
                }
            };

            // Set DCO sign-off
            let dco = set_dco_signature(&token);

            match graphql_commit_push(
                &repo_nwo,
                &args.origin_branch,
                &repo_path_str,
                &commit_msg,
                &dco,
                &parent_sha,
                &token,
            ) {
                Ok(url) => {
                    output::done(format!("{} ({}) -> {}", app, version, url));
                    success_count += 1;
                }
                Err(e) => {
                    output::err(format!("{}: commit failed: {}", app, e));
                }
            }
        } else {
            // Request mode: create branch + PR
            let branch_name = format!("manifest/{}-{}", app, version);

            // Check if remote branch already exists
            if let Ok(Some(_)) = get_ref_sha_optional(&repo_nwo, &branch_name, &token) {
                output::warn(format!("{} ({}): branch already exists, skipping", app, version));
                skip_count += 1;
                continue;
            }

            output::progress("Creating", &format!("{} ({})", app, version));

            // Get parent SHA from origin branch
            let parent_sha = match get_ref_sha(&repo_nwo, &args.origin_branch, &token) {
                Ok(s) => s,
                Err(e) => {
                    output::err(format!("{}: get HEAD SHA failed: {}", app, e));
                    continue;
                }
            };

            // Create remote branch
            if let Err(e) = create_ref(&repo_nwo, &branch_name, &parent_sha, &token) {
                output::err(format!("{}: create branch failed: {}", app, e));
                continue;
            }

            // DCO sign-off
            let dco = set_dco_signature(&token);

            // GraphQL commit
            if let Err(e) = graphql_commit_push(
                &repo_nwo,
                &branch_name,
                &repo_path_str,
                &commit_msg,
                &dco,
                &parent_sha,
                &token,
            ) {
                output::err(format!("{}: commit failed: {}", app, e));
                continue;
            }

            // Create PR
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

            match create_pull_request(
                &upstream_repo_nwo,
                &upstream_branch,
                &head_ref,
                &commit_msg,
                &pr_body,
                &token,
            ) {
                Ok(pr_url) => {
                    output::done(format!("{} ({}) -> PR: {}", app, version, pr_url));
                    success_count += 1;
                }
                Err(e) => {
                    output::err(format!("{}: create PR failed: {}", app, e));
                }
            }
        }
    }

    // 9. Summary
    if args.push {
        output::info(format!(
            "ci-auto-pr completed: {} pushed, {} skipped",
            success_count,
            changed.len() - success_count as usize
        ));
    } else {
        output::info(format!(
            "ci-auto-pr completed: {} PRs created, {} skipped, {} already existed",
            success_count,
            changed.len() - success_count as usize - skip_count as usize,
            skip_count
        ));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Snapshot helpers
// ---------------------------------------------------------------------------

/// Read all JSON manifest files in a directory into a HashMap of path → content hash.
fn read_manifests(dir: &Path) -> Result<HashMap<PathBuf, Vec<u8>>> {
    let mut map = HashMap::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            output::err(format!("read dir {}: {}", dir.display(), e));
            return Ok(map);
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e != "json").unwrap_or(true) {
            continue;
        }
        match std::fs::read(&path) {
            Ok(content) => {
                map.insert(path, content);
            }
            Err(e) => {
                output::err(format!("read {}: {}", path.display(), e));
            }
        }
    }
    Ok(map)
}

/// Detect which manifest files changed after checkver ran.
fn detect_changes(_dir: &Path, before: &HashMap<PathBuf, Vec<u8>>) -> Vec<PathBuf> {
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
// Repo / branch helpers
// ---------------------------------------------------------------------------

/// Resolve the repository NWO (owner/name) from env var or git remote.
fn resolve_repo_nwo() -> Result<String> {
    // Try GITHUB_REPOSITORY env var first (GitHub Actions)
    if let Ok(repo) = std::env::var("GITHUB_REPOSITORY") {
        if !repo.is_empty() {
            return Ok(repo);
        }
    }

    // Try git remote as fallback (works when running locally in a repo)
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .map_err(|_| {
            anyhow::anyhow!(
                "Cannot determine repository. Set GITHUB_REPOSITORY env var."
            )
        })?;

    if !output.status.success() {
        anyhow::bail!(
            "Cannot determine repository. Set GITHUB_REPOSITORY env var."
        );
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let url = url.trim_end_matches(".git");

    // Parse: https://github.com/owner/repo or git@github.com:owner/repo
    if let Some(nwo) = url.split("github.com/").nth(1) {
        Ok(nwo.trim_end_matches('/').to_string())
    } else if let Some(nwo) = url.split("github.com:").nth(1) {
        Ok(nwo.trim_end_matches('/').to_string())
    } else {
        anyhow::bail!("Cannot parse repository NWO from remote URL: {}", url);
    }
}

/// Make a file path relative to the current working directory.
fn repo_relative_path(path: &Path) -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(relative) = path.strip_prefix(&cwd) {
            return relative.to_path_buf();
        }
    }
    path.to_path_buf()
}

// ---------------------------------------------------------------------------
// GitHub API helpers
// ---------------------------------------------------------------------------

/// Make a GitHub REST API request.
///
/// `query`: API path (e.g., "repos/owner/repo/pulls")
/// `method`: HTTP method ("GET" or "POST")
/// `body`: Optional JSON body
/// `token`: GitHub API token
///
/// Returns the parsed JSON response.
fn github_api_request(
    query: &str,
    method: &str,
    body: Option<serde_json::Value>,
    token: &str,
) -> Result<serde_json::Value> {
    let url = format!("https://api.github.com/{}", query);
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build()
        .new_agent();

    let resp = if method == "GET" {
        let mut req = agent.get(&url);
        req = req
            .header("Authorization", &format!("Bearer {}", token))
            .header("User-Agent", "hok")
            .header("Accept", "application/vnd.github.v3+json");
        req.call()
    } else {
        let mut req = agent.post(&url);
        req = req
            .header("Authorization", &format!("Bearer {}", token))
            .header("User-Agent", "hok")
            .header("Accept", "application/vnd.github.v3+json");
        // POST always sends a body; use empty object if none provided
        req.send_json(&body.unwrap_or(serde_json::json!({})))
    };

    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            // Try to read error response body for better error messages
            let msg = format!("{}", e);
            anyhow::bail!("GitHub API error: {}", msg);
        }
    };

    let status = resp.status().as_u16();
    let body_str = resp.into_body().read_to_string()
        .map_err(|e| anyhow::anyhow!("Failed to read response body: {}", e))?;

    // Parse JSON (even for error responses, GitHub returns JSON)
    let value: serde_json::Value = match serde_json::from_str(&body_str) {
        Ok(v) => v,
        Err(e) => {
            if status >= 400 {
                anyhow::bail!("GitHub API error (HTTP {}) with non-JSON body", status);
            }
            anyhow::bail!("Failed to parse JSON response: {} | body: {}", e, &body_str[..body_str.len().min(200)]);
        }
    };

    if status >= 400 {
        let msg = value["message"].as_str().unwrap_or("unknown error");
        anyhow::bail!("GitHub API error (HTTP {}): {}", status, msg);
    }

    Ok(value)
}

/// Get the SHA of a reference (branch) from the GitHub API.
fn get_ref_sha(repo: &str, branch: &str, token: &str) -> Result<String> {
    let query = format!("repos/{}/git/refs/heads/{}", repo, branch);
    let resp = github_api_request(&query, "GET", None, token)?;
    let sha = resp["object"]["sha"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("unexpected ref response format"))?
        .to_string();
    Ok(sha)
}

/// Get the SHA of a reference, returning None if it doesn't exist.
fn get_ref_sha_optional(repo: &str, branch: &str, token: &str) -> Result<Option<String>> {
    let query = format!("repos/{}/git/refs/heads/{}", repo, branch);
    match github_api_request(&query, "GET", None, token) {
        Ok(resp) => {
            let sha = resp["object"]["sha"]
                .as_str()
                .map(|s| s.to_string());
            Ok(sha)
        }
        Err(e) => {
            // 404 means branch doesn't exist
            let msg = e.to_string();
            if msg.contains("HTTP 404") || msg.contains("Not Found") {
                Ok(None)
            } else {
                Err(e)
            }
        }
    }
}

/// Create a new branch reference on GitHub.
fn create_ref(repo: &str, branch: &str, sha: &str, token: &str) -> Result<()> {
    let query = format!("repos/{}/git/refs", repo);
    let body = serde_json::json!({
        "ref": format!("refs/heads/{}", branch),
        "sha": sha,
    });
    github_api_request(&query, "POST", Some(body), token)?;
    Ok(())
}

/// Create a pull request via GitHub REST API.
fn create_pull_request(
    upstream_repo: &str,
    upstream_branch: &str,
    head: &str,
    title: &str,
    body: &str,
    token: &str,
) -> Result<String> {
    let query = format!("repos/{}/pulls", upstream_repo);
    let request_body = serde_json::json!({
        "title": title,
        "body": body,
        "head": head,
        "base": upstream_branch,
    });
    let resp = github_api_request(&query, "POST", Some(request_body), token)?;
    let pr_url = resp["html_url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("PR response missing html_url"))?
        .to_string();
    Ok(pr_url)
}

// ---------------------------------------------------------------------------
// GraphQL commit helpers
// ---------------------------------------------------------------------------

/// Generate a DCO (Developer Certificate of Origin) sign-off string.
fn set_dco_signature(token: &str) -> String {
    let login = match github_api_request("user", "GET", None, token) {
        Ok(resp) => resp["login"].as_str().unwrap_or("github-actions").to_string(),
        Err(_) => "github-actions".to_string(),
    };
    let id: u64 = match github_api_request("user", "GET", None, token) {
        Ok(resp) => resp["id"].as_u64().unwrap_or(41898282),
        Err(_) => 41898282,
    };

    format!("Signed-off-by: {login} <{id}+{login}@users.noreply.github.com>")
}

/// Commit and push a file to a branch using GitHub's GraphQL API.
///
/// Uses the `createCommitOnBranch` mutation — no git binary required.
fn graphql_commit_push(
    repo: &str,
    branch: &str,
    file_path: &str,
    title: &str,
    dco_body: &str,
    parent_sha: &str,
    token: &str,
) -> Result<String> {
    // Read and encode the file
    let content = std::fs::read_to_string(file_path)
        .map_err(|e| anyhow::anyhow!("read file {}: {}", file_path, e))?;
    // Normalize line endings to LF (git convention)
    let content_lf = content.replace("\r\n", "\n");
    let encoded = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        content_lf.as_bytes(),
    );

    let query = r#"
        mutation ($input: CreateCommitOnBranchInput!) {
            createCommitOnBranch(input: $input) {
                commit { url }
            }
        }
    "#;

    let variables = serde_json::json!({
        "input": {
            "branch": {
                "repositoryNameWithOwner": repo,
                "branchName": branch
            },
            "message": {
                "headline": title,
                "body": dco_body
            },
            "fileChanges": {
                "additions": [
                    {
                        "path": file_path,
                        "contents": encoded
                    }
                ]
            },
            "expectedHeadOid": parent_sha
        }
    });

    let body = serde_json::json!({
        "query": query,
        "variables": variables
    });

    let url = "https://api.github.com/graphql";
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(60)))
        .build()
        .new_agent();

    let resp = agent
        .post(url)
        .header("Authorization", &format!("Bearer {}", token))
        .header("User-Agent", "hok")
        .send_json(&body)
        .map_err(|e| anyhow::anyhow!("GraphQL request failed: {}", e))?;

    let body_str = resp.into_body().read_to_string()
        .map_err(|e| anyhow::anyhow!("Read GraphQL response failed: {}", e))?;

    let value: serde_json::Value = serde_json::from_str(&body_str)
        .map_err(|e| anyhow::anyhow!("Parse GraphQL response failed: {} | body: {}", e, &body_str[..body_str.len().min(200)]))?;

    // Check for GraphQL errors
    if let Some(errors) = value["errors"].as_array() {
        if !errors.is_empty() {
            let msgs: Vec<String> = errors.iter()
                .filter_map(|e| e["message"].as_str())
                .map(|s| s.to_string())
                .collect();
            anyhow::bail!("GraphQL error(s): {}", msgs.join("; "));
        }
    }

    let commit_url = value["data"]["createCommitOnBranch"]["commit"]["url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("GraphQL response missing commit URL"))?
        .to_string();

    Ok(commit_url)
}
