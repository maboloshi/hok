//! GitHub API client utilities (REST and GraphQL).
//!
//! 提供与 GitHub REST v3 和 GraphQL v4 API 交互的可复用、带认证的辅助函数，
//! 主要供 `auto_pr` 流水线使用，但也可被其他模块复用。
//!
//! # 认证
//!
//! 所有函数都需要一个有效的 GitHub 个人访问令牌（PAT）或安装令牌作为 `token` 参数。
//! 令牌需具备以下权限（根据调用的函数）：
//! - `contents:write` — 提交文件到分支
//! - `pull-requests:write` — 创建 Pull Request
//!
//! # 返回值
//!
//! - REST 函数返回 `Result<serde_json::Value>`，成功时包含完整的 JSON 响应体。
//! - [`graphql_commit_push`] 返回所创建 commit 的 HTML URL。
//!
//! # 注意事项
//!
//! - 所有请求超时为 30 秒（GraphQL 为 60 秒）。
//! - 在 HTTP 4xx/5xx 响应时会以 `anyhow::Error` 形式返回带状态码的错误消息。
//! - 令牌不会被记录到日志中（日志中显示为 `******`）。

use anyhow::Result;
use base64::Engine as _;

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
        req.send_json(&body.unwrap_or(serde_json::json!({})))
    };

    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            anyhow::bail!("GitHub API error: {}", e);
        }
    };

    let status = resp.status().as_u16();
    let body_str = resp
        .into_body()
        .read_to_string()
        .map_err(|e| anyhow::anyhow!("Failed to read response body: {}", e))?;

    let value: serde_json::Value = match serde_json::from_str(&body_str) {
        Ok(v) => v,
        Err(e) => {
            if status >= 400 {
                anyhow::bail!("GitHub API error (HTTP {}) with non-JSON body", status);
            }
            anyhow::bail!(
                "Failed to parse JSON response: {} | body: {}",
                e,
                &body_str[..body_str.len().min(200)]
            );
        }
    };

    if status >= 400 {
        let msg = value["message"].as_str().unwrap_or("unknown error");
        anyhow::bail!("GitHub API error (HTTP {}): {}", status, msg);
    }

    Ok(value)
}

/// Get the commit SHA of a branch head via the GitHub Refs API.
pub fn get_ref_sha(repo: &str, branch: &str, token: &str) -> Result<String> {
    let query = format!("repos/{}/git/refs/heads/{}", repo, branch);
    let resp = github_api_request(&query, "GET", None, token)?;
    let sha = resp["object"]["sha"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("unexpected ref response format"))?
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
        .ok_or_else(|| anyhow::anyhow!("PR response missing html_url"))?
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
        .map_err(|e| anyhow::anyhow!("read file {}: {}", file_path, e))?;
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
        .map_err(|e| anyhow::anyhow!("GraphQL request failed: {}", e))?;

    let body_str = resp
        .into_body()
        .read_to_string()
        .map_err(|e| anyhow::anyhow!("Read GraphQL response failed: {}", e))?;

    let value: serde_json::Value = serde_json::from_str(&body_str).map_err(|e| {
        anyhow::anyhow!(
            "Parse GraphQL response failed: {} | body: {}",
            e,
            &body_str[..body_str.len().min(200)]
        )
    })?;

    if let Some(errors) = value["errors"].as_array() {
        if !errors.is_empty() {
            let msgs: Vec<String> = errors
                .iter()
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
