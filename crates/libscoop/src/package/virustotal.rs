//! VirusTotal URL scanning integration.
//!
//! 通过 VirusTotal REST API v3 对下载 URL 进行恶意软件扫描。
//!
//! # 使用方式
//!
//! 需要通过 `VT_API_KEY` 环境变量或配置文件中的 `virustotal_api_key`
//! 字段提供 API 密钥。
//!
//! ```no_run
//! use libscoop::package::virustotal::{check_url, ScanStats};
//!
//! let stats: ScanStats = check_url("https://example.com/app.exe", "your_api_key").unwrap();
//! println!("Total engines: {}, Malicious: {}", stats.total, stats.malicious);
//! ```
//!
//! # 注意事项
//!
//! - API 请求会轮询最多 5 次（每次间隔 3 秒）等待分析完成。
//! - 网络错误或超时将以 `anyhow::Error` 形式返回。

use anyhow::Result;

/// Scan statistics returned by VirusTotal.
#[derive(Debug, Clone)]
pub struct ScanStats {
    /// Total number of scanning engines queried.
    pub total: u32,
    /// Number of engines that flagged the URL as malicious.
    pub malicious: u32,
    /// Number of engines that flagged the URL as suspicious.
    pub suspicious: u32,
    /// Number of engines that found the URL harmless.
    pub harmless: u32,
}

/// Submit a URL to VirusTotal and poll for the analysis result.
///
/// # Arguments
///
/// * `url`     — The URL to scan (a download link).
/// * `api_key` — A valid VirusTotal API v3 key.
///
/// # Errors
///
/// Returns an error if the HTTP request fails, the JSON response is unexpected,
/// or the analysis does not complete within the polling window (5 × 3 s).
pub fn check_url(url: &str, api_key: &str) -> Result<ScanStats> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build()
        .new_agent();

    // Step 1: submit URL for analysis
    let submit_body = serde_json::json!({ "url": url });
    let submit_resp = agent
        .post("https://www.virustotal.com/api/v3/urls")
        .header("x-apikey", api_key)
        .send_json(&submit_body)
        .map_err(|e| anyhow::anyhow!("VT submit error: {}", e))?;

    let submit_json: serde_json::Value = submit_resp
        .into_body()
        .read_json()
        .map_err(|e| anyhow::anyhow!("VT response error: {}", e))?;

    let analysis_id = submit_json["data"]["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("VT: no analysis ID in response"))?;

    // Step 2: poll for analysis results (up to 5 attempts with 3 s delay)
    let analysis_url = format!(
        "https://www.virustotal.com/api/v3/analyses/{}",
        analysis_id
    );

    for _ in 0..5 {
        std::thread::sleep(std::time::Duration::from_secs(3));

        let resp = agent
            .get(&analysis_url)
            .header("x-apikey", api_key)
            .call()
            .map_err(|e| anyhow::anyhow!("VT poll error: {}", e))?;

        let json: serde_json::Value = resp
            .into_body()
            .read_json()
            .map_err(|e| anyhow::anyhow!("VT parse error: {}", e))?;

        let status = json["data"]["attributes"]["status"]
            .as_str()
            .unwrap_or("");
        if status == "completed" {
            let stats = &json["data"]["attributes"]["stats"];
            return Ok(ScanStats {
                total: stats["total"].as_u64().unwrap_or(0) as u32,
                malicious: stats["malicious"].as_u64().unwrap_or(0) as u32,
                suspicious: stats["suspicious"].as_u64().unwrap_or(0) as u32,
                harmless: stats["harmless"].as_u64().unwrap_or(0) as u32,
            });
        }
    }

    Err(anyhow::anyhow!("VT analysis timed out after 5 polling attempts"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_stats_fields_accessible() {
        let s = ScanStats {
            total: 70,
            malicious: 2,
            suspicious: 1,
            harmless: 67,
        };
        assert_eq!(s.total, 70);
        assert_eq!(s.malicious, 2);
        assert_eq!(s.suspicious, 1);
        assert_eq!(s.harmless, 67);
    }
}
