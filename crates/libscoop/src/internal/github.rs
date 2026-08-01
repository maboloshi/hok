//! GitHub API client utilities (REST and GraphQL).
//!
//! Provides reusable, authenticated helper functions for interacting with the GitHub REST v3 and GraphQL v4 APIs,
//! primarily for use by the `auto_pr` pipeline, but also reusable by other modules.
//!
//! # Authentication
//!
//! All functions require a valid GitHub personal access token (PAT) or installation token as the `token` parameter.
//! The token must have the following permissions (depending on the function called):
//! - `contents:write` — Commit files to a branch
//! - `pull-requests:write` — Create a Pull Request
//!
//! # Return Values
//!
//! - REST functions return `Result<serde_json::Value>`, containing the full JSON response body on success.
//! - [`graphql_commit_push`] returns the HTML URL of the created commit.
//!
//! # Notes
//!
//! - All requests have a 30-second timeout (60 seconds for GraphQL).
//! - On HTTP 4xx/5xx responses, an error message with the status code is returned as [`crate::Error`].
//! - Tokens are never logged (shown as `******` in logs).

use crate::error::Fallible as Result;
use base64::Engine as _;
use regex::Regex;

// ---------------------------------------------------------------------------
// REST helpers
// ---------------------------------------------------------------------------

/// Make a GitHub REST API request.
///
/// * `query`  – API path relative to `https://api.github.com/` (e.g. `repos/owner/repo/pulls`).
/// * `method` – HTTP method: `"GET"` or `"POST"`.
/// * `body`   – Optional JSON body (for POST requests).
/// * `token`  – GitHub personal-access token or installation token.
///
/// Returns the parsed JSON response on success, or an error with the
/// HTTP status and GitHub error message on failure.
pub fn github_api_request(
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
        let req = agent
            .get(&url)
            .header("Authorization", &format!("Bearer {}", token))
            .header("User-Agent", "hok")
            .header("Accept", "application/vnd.github.v3+json");
        req.call()
    } else {
        let req = agent
            .post(&url)
            .header("Authorization", &format!("Bearer {}", token))
            .header("User-Agent", "hok")
            .header("Accept", "application/vnd.github.v3+json");
        // POST always sends a body; use empty object if none provided.
        req.send_json(body.unwrap_or(serde_json::json!({})))
    };

    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            return Err(crate::Error::Custom(format!("GitHub API error: {}", e)));
        }
    };

    let status = resp.status().as_u16();
    let body_str = resp
        .into_body()
        .read_to_string()
        .map_err(|e| crate::Error::Custom(format!("Failed to read response body: {}", e)))?;

    let value: serde_json::Value = match serde_json::from_str(&body_str) {
        Ok(v) => v,
        Err(e) => {
            if status >= 400 {
                return Err(crate::Error::Custom(format!(
                    "GitHub API error (HTTP {}) with non-JSON body",
                    status
                )));
            }
            return Err(crate::Error::Custom(format!(
                "Failed to parse JSON response: {} | body: {}",
                e,
                &body_str[..body_str.len().min(200)]
            )));
        }
    };

    if status >= 400 {
        let msg = value["message"].as_str().unwrap_or("unknown error");
        return Err(crate::Error::Custom(format!(
            "GitHub API error (HTTP {}): {}",
            status, msg
        )));
    }

    Ok(value)
}

/// Get the commit SHA of a branch head via the GitHub Refs API.
pub fn get_ref_sha(repo: &str, branch: &str, token: &str) -> Result<String> {
    let query = format!("repos/{}/git/refs/heads/{}", repo, branch);
    let resp = github_api_request(&query, "GET", None, token)?;
    let sha = resp["object"]["sha"]
        .as_str()
        .ok_or_else(|| crate::Error::Custom("unexpected ref response format".to_string()))?
        .to_string();
    Ok(sha)
}

/// Get the commit SHA of a branch head, returning `None` if the branch does
/// not exist (HTTP 404), or an error for other failures.
pub fn get_ref_sha_optional(repo: &str, branch: &str, token: &str) -> Result<Option<String>> {
    let query = format!("repos/{}/git/refs/heads/{}", repo, branch);
    match github_api_request(&query, "GET", None, token) {
        Ok(resp) => {
            let sha = resp["object"]["sha"].as_str().map(|s| s.to_string());
            Ok(sha)
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("HTTP 404") || msg.contains("Not Found") {
                Ok(None)
            } else {
                Err(e)
            }
        }
    }
}

/// Create a new remote branch by POSTing a ref to the Refs API.
pub fn create_ref(repo: &str, branch: &str, sha: &str, token: &str) -> Result<()> {
    let query = format!("repos/{}/git/refs", repo);
    let body = serde_json::json!({
        "ref": format!("refs/heads/{}", branch),
        "sha": sha,
    });
    github_api_request(&query, "POST", Some(body), token)?;
    Ok(())
}

/// Open a pull request via the GitHub REST Pulls API.
///
/// Returns the HTML URL of the created pull request.
pub fn create_pull_request(
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
        "body":  body,
        "head":  head,
        "base":  upstream_branch,
    });
    let resp = github_api_request(&query, "POST", Some(request_body), token)?;
    let pr_url = resp["html_url"]
        .as_str()
        .ok_or_else(|| crate::Error::Custom("PR response missing html_url".to_string()))?
        .to_string();
    Ok(pr_url)
}

// ---------------------------------------------------------------------------
// GraphQL helpers
// ---------------------------------------------------------------------------

/// Build a DCO (Developer Certificate of Origin) sign-off string for the
/// authenticated user.
pub fn set_dco_signature(token: &str) -> String {
    let resp = github_api_request("user", "GET", None, token);
    let login = resp
        .as_ref()
        .ok()
        .and_then(|r| r["login"].as_str())
        .unwrap_or("github-actions")
        .to_string();
    let id: u64 = resp
        .as_ref()
        .ok()
        .and_then(|r| r["id"].as_u64())
        .unwrap_or(41898282);
    format!("Signed-off-by: {login} <{id}+{login}@users.noreply.github.com>")
}

/// Commit a single file to a remote branch using GitHub's GraphQL
/// `createCommitOnBranch` mutation — no local git binary required.
///
/// Returns the HTML URL of the created commit.
pub fn graphql_commit_push(
    repo: &str,
    branch: &str,
    file_path: &str,
    title: &str,
    dco_body: &str,
    parent_sha: &str,
    token: &str,
) -> Result<String> {
    // Read the file and base64-encode it with normalised line endings.
    let content = std::fs::read_to_string(file_path)
        .map_err(|e| crate::Error::Custom(format!("read file {}: {}", file_path, e)))?;
    let content_lf = content.replace("\r\n", "\n");
    let encoded = base64::engine::general_purpose::STANDARD.encode(content_lf.as_bytes());

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
                    { "path": file_path, "contents": encoded }
                ]
            },
            "expectedHeadOid": parent_sha
        }
    });

    let body = serde_json::json!({
        "query": query,
        "variables": variables
    });

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(60)))
        .build()
        .new_agent();

    let resp = agent
        .post("https://api.github.com/graphql")
        .header("Authorization", &format!("Bearer {}", token))
        .header("User-Agent", "hok")
        .send_json(&body)
        .map_err(|e| crate::Error::Custom(format!("GraphQL request failed: {}", e)))?;

    let body_str = resp
        .into_body()
        .read_to_string()
        .map_err(|e| crate::Error::Custom(format!("Read GraphQL response failed: {}", e)))?;

    let value: serde_json::Value = serde_json::from_str(&body_str).map_err(|e| {
        crate::Error::Custom(format!(
            "Parse GraphQL response failed: {} | body: {}",
            e,
            &body_str[..body_str.len().min(200)]
        ))
    })?;

    if let Some(errors) = value["errors"].as_array() {
        if !errors.is_empty() {
            let msgs: Vec<String> = errors
                .iter()
                .filter_map(|e| e["message"].as_str())
                .map(|s| s.to_string())
                .collect();
            return Err(crate::Error::Custom(format!(
                "GraphQL error(s): {}",
                msgs.join("; ")
            )));
        }
    }

    let commit_url = value["data"]["createCommitOnBranch"]["commit"]["url"]
        .as_str()
        .ok_or_else(|| crate::Error::Custom("GraphQL response missing commit URL".to_string()))?
        .to_string();

    Ok(commit_url)
}

// ---------------------------------------------------------------------------
// URL parsing helpers (GitHub / SourceForge)
// ---------------------------------------------------------------------------

/// Build the GitHub API "latest release" URL from a homepage or releases URL.
///
/// Accepts both `https://github.com/<owner>/<repo>` (homepage) and
/// `https://github.com/<owner>/<repo>/releases/...` forms.
pub fn github_api_url(url_or_homepage: &str) -> Option<String> {
    // Try to match as a github.com URL first (handles both homepage and releases URLs)
    let re = Regex::new(r"github\.com[:/]([^/]+/[^/]+?)(?:/|$)").ok()?;
    let caps = re.captures(url_or_homepage)?;
    let repo = caps.get(1)?.as_str().trim_end_matches('/');
    Some(format!(
        "https://api.github.com/repos/{}/releases/latest",
        repo
    ))
}

/// Extract SourceForge project name from homepage URL.
pub fn extract_sourceforge_project(homepage: &str) -> Option<String> {
    let re = Regex::new(r"sourceforge\.net/projects/([^/]+)").ok()?;
    let caps = re.captures(homepage)?;
    caps.get(1).map(|m| m.as_str().to_string())
}

/// Parse a SourceForge download URL to extract (project, file_path).
pub fn parse_sourceforge_url(url: &str) -> Option<(String, String)> {
    // Scoop regex: '(?:downloads\.)?sourceforge.net\/projects?\/(?<project>[^\/]+)\/(?:files\/)?(?<file>.*)'
    let re = Regex::new(
        r"(?:downloads\.)?(?:sourceforge\.net|sf\.net)/projects?/([^/]+)(?:/files)?/(.*)",
    )
    .ok()?;
    let caps = re.captures(url)?;
    let project = caps.get(1)?.as_str().to_string();
    let file_path = caps.get(2)?.as_str().trim_end_matches('/').to_string();
    Some((project, file_path))
}

/// Parse a GitHub release download URL to extract (owner, repo).
pub fn parse_github_download_url(url: &str) -> Option<(String, String)> {
    // Pattern: https://github.com/<owner>/<repo>/releases/download/...
    let re = Regex::new(r"github\.com/([^/]+)/([^/]+)/releases/download/").ok()?;
    let caps = re.captures(url)?;
    let owner = caps.get(1)?.as_str().to_string();
    let repo = caps.get(2)?.as_str().to_string();
    Some((owner, repo))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── github_api_url ───────────────────────────────────────────────────────

    #[test]
    fn github_api_url_from_homepage() {
        let result = github_api_url("https://github.com/BurntSushi/ripgrep");
        assert!(result.is_some());
        let url = result.unwrap();
        assert!(url.contains("api.github.com"));
        assert!(url.contains("BurntSushi/ripgrep"));
    }

    #[test]
    fn github_api_url_from_releases_url() {
        let result = github_api_url("https://github.com/sharkdp/bat/releases/latest");
        assert!(result.is_some());
        assert!(result.unwrap().contains("api.github.com"));
    }

    #[test]
    fn github_api_url_non_github_returns_none() {
        let result = github_api_url("https://example.com/owner/repo");
        assert!(result.is_none());
    }

    // ── extract_sourceforge_project ──────────────────────────────────────────

    #[test]
    fn sourceforge_extracts_project_from_homepage() {
        let result = extract_sourceforge_project("https://sourceforge.net/projects/sevenzip/");
        assert_eq!(result.as_deref(), Some("sevenzip"));
    }

    #[test]
    fn sourceforge_returns_none_for_non_sf_url() {
        let result = extract_sourceforge_project("https://example.com/project");
        assert!(result.is_none());
    }

    // ── parse_sourceforge_url / parse_github_download_url ────────────────────

    #[test]
    fn parse_sourceforge_download_url() {
        let (project, file_path) = parse_sourceforge_url(
            "https://downloads.sourceforge.net/project/sevenzip/7-Zip/24.09/7z2409-x64.exe",
        )
        .unwrap();
        assert_eq!(project, "sevenzip");
        assert_eq!(file_path, "7-Zip/24.09/7z2409-x64.exe");
    }

    #[test]
    fn parse_github_release_download_url() {
        let (owner, repo) = parse_github_download_url(
            "https://github.com/BurntSushi/ripgrep/releases/download/14.1.1/ripgrep-14.1.1-x86_64-pc-windows-msvc.zip",
        )
        .unwrap();
        assert_eq!(owner, "BurntSushi");
        assert_eq!(repo, "ripgrep");
    }
}
