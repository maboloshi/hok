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
//! - All requests are async (via `reqwest`) and have a 30-second timeout (60 seconds for GraphQL).
//! - On HTTP 4xx/5xx responses, an error message with the status code is returned as [`crate::Error`].
//! - Tokens are never logged (shown as `******` in logs).

use crate::error::{Error, Fallible};
use reqwest::Method;

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
pub async fn github_api_request(
    query: &str,
    method: &str,
    body: Option<serde_json::Value>,
    token: &str,
) -> Fallible<serde_json::Value> {
    let url = format!("https://api.github.com/{}", query);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| Error::Custom(format!("GitHub client error: {}", e)))?;

    let http_method = Method::from_bytes(method.as_bytes())
        .map_err(|e| Error::Custom(format!("Invalid method {}: {}", method, e)))?;

    let mut req = client
        .request(http_method, &url)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", crate::constant::DEFAULT_USER_AGENT)
        .header("Accept", "application/vnd.github.v3+json");

    // POST always sends a body; use empty object if none provided.
    if method == "POST" {
        let payload = body.unwrap_or(serde_json::json!({}));
        req = req.json(&payload);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| Error::Custom(format!("GitHub API error: {}", e)))?;

    let status = resp.status().as_u16();
    let body_str = resp
        .text()
        .await
        .map_err(|e| Error::Custom(format!("Failed to read response body: {}", e)))?;

    let value: serde_json::Value = match serde_json::from_str(&body_str) {
        Ok(v) => v,
        Err(e) => {
            if status >= 400 {
                return Err(Error::Custom(format!(
                    "GitHub API error (HTTP {}) with non-JSON body",
                    status
                )));
            }
            return Err(Error::Custom(format!(
                "Failed to parse JSON response: {} | body: {}",
                e,
                &body_str[..body_str.len().min(200)]
            )));
        }
    };

    if status >= 400 {
        let msg = value["message"].as_str().unwrap_or("unknown error");
        return Err(Error::Custom(format!(
            "GitHub API error (HTTP {}): {}",
            status, msg
        )));
    }

    Ok(value)
}

/// Get the commit SHA of a branch head via the GitHub Refs API.
pub async fn get_ref_sha(repo: &str, branch: &str, token: &str) -> Fallible<String> {
    let query = format!("repos/{}/git/refs/heads/{}", repo, branch);
    let resp = github_api_request(&query, "GET", None, token).await?;
    let sha = resp["object"]["sha"]
        .as_str()
        .ok_or_else(|| Error::Custom("unexpected ref response format".to_string()))?
        .to_string();
    Ok(sha)
}

/// Get the commit SHA of a branch head, returning `None` if the branch does
/// not exist (HTTP 404), or an error for other failures.
pub async fn get_ref_sha_optional(repo: &str, branch: &str, token: &str) -> Fallible<Option<String>> {
    let query = format!("repos/{}/git/refs/heads/{}", repo, branch);
    match github_api_request(&query, "GET", None, token).await {
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
pub async fn create_ref(repo: &str, branch: &str, sha: &str, token: &str) -> Fallible<()> {
    let query = format!("repos/{}/git/refs", repo);
    let body = serde_json::json!({
        "ref": format!("refs/heads/{}", branch),
        "sha": sha,
    });
    github_api_request(&query, "POST", Some(body), token).await?;
    Ok(())
}

/// Open a pull request via the GitHub REST Pulls API.
///
/// Returns the HTML URL of the created pull request.
pub async fn create_pull_request(
    upstream_repo: &str,
    upstream_branch: &str,
    head: &str,
    title: &str,
    body: &str,
    token: &str,
) -> Fallible<String> {
    let query = format!("repos/{}/pulls", upstream_repo);
    let request_body = serde_json::json!({
        "title": title,
        "body":  body,
        "head":  head,
        "base":  upstream_branch,
    });
    let resp = github_api_request(&query, "POST", Some(request_body), token).await?;
    let pr_url = resp["html_url"]
        .as_str()
        .ok_or_else(|| Error::Custom("PR response missing html_url".to_string()))?
        .to_string();
    Ok(pr_url)
}

// ---------------------------------------------------------------------------
// GraphQL helpers
// ---------------------------------------------------------------------------

/// Build a DCO (Developer Certificate of Origin) sign-off string.
///
/// Resolution order:
/// 1. If `app_slug` is provided, resolve the corresponding bot user via
///    `GET /users/{app_slug}[bot]`.
/// 2. Otherwise (or if step 1 fails), resolve the authenticated user via
///    `GET /user`.
/// 3. Fall back to `github-actions[bot]` if all lookups fail, so the pipeline
///    never aborts when the token lacks permission to read user info.
pub async fn set_dco_signature(token: &str, app_slug: Option<&str>) -> String {
    let user_info = match app_slug {
        Some(slug) => {
            let bot_username = format!("{}[bot]", slug);
            match get_user_by_username(&bot_username, token).await {
                Some(info) => Some(info),
                None => get_authenticated_user(token).await,
            }
        }
        None => get_authenticated_user(token).await,
    };

    let (login, id) = user_info.unwrap_or_else(|| ("github-actions[bot]".to_string(), 41898282));
    format!("Signed-off-by: {login} <{id}+{login}@users.noreply.github.com>")
}

/// Fetch a user by username, returning `(login, id)` on success.
async fn get_user_by_username(username: &str, token: &str) -> Option<(String, u64)> {
    let query = format!("users/{}", username);
    let resp = github_api_request(&query, "GET", None, token).await;
    user_info_from(&resp)
}

/// Fetch the currently authenticated user, returning `(login, id)` on success.
async fn get_authenticated_user(token: &str) -> Option<(String, u64)> {
    let resp = github_api_request("user", "GET", None, token).await;
    user_info_from(&resp)
}

/// Extract `(login, id)` from a GitHub user JSON response.
fn user_info_from(resp: &Fallible<serde_json::Value>) -> Option<(String, u64)> {
    let resp = resp.as_ref().ok()?;
    let login = resp["login"].as_str()?;
    let id = resp["id"].as_u64()?;
    Some((login.to_string(), id))
}

/// Commit a single file to a remote branch using GitHub's GraphQL
/// `createCommitOnBranch` mutation — no local git binary required.
///
/// Returns the HTML URL of the created commit.
pub async fn graphql_commit_push(
    repo: &str,
    branch: &str,
    file_path: &str,
    title: &str,
    dco_body: &str,
    parent_sha: &str,
    token: &str,
) -> Fallible<String> {
    // Read the file and base64-encode it with normalised line endings.
    let content = std::fs::read_to_string(file_path)
        .map_err(|e| Error::Custom(format!("read file {}: {}", file_path, e)))?;
    let content_lf = content.replace("\r\n", "\n");
    let encoded = crate::internal::base64::encode(content_lf.as_bytes());

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

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| Error::Custom(format!("GraphQL client error: {}", e)))?;

    let resp = client
        .post("https://api.github.com/graphql")
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", crate::constant::DEFAULT_USER_AGENT)
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Custom(format!("GraphQL request failed: {}", e)))?;

    let body_str = resp
        .text()
        .await
        .map_err(|e| Error::Custom(format!("Read GraphQL response failed: {}", e)))?;

    let value: serde_json::Value = serde_json::from_str(&body_str).map_err(|e| {
        Error::Custom(format!(
            "Parse GraphQL response failed: {} | body: {}",
            e,
            &body_str[..body_str.len().min(200)]
        ))
    })?;

    if let Some(errors) = value["errors"].as_array()
        && !errors.is_empty()
    {
        let msgs: Vec<String> = errors
            .iter()
            .filter_map(|e| e["message"].as_str())
            .map(|s| s.to_string())
            .collect();
        return Err(Error::Custom(format!(
            "GraphQL error(s): {}",
            msgs.join("; ")
        )));
    }

    let commit_url = value["data"]["createCommitOnBranch"]["commit"]["url"]
        .as_str()
        .ok_or_else(|| Error::Custom("GraphQL response missing commit URL".to_string()))?
        .to_string();

    Ok(commit_url)
}
