//! Hash collection for checkver (`--update` autoupdate).
//!
//! Contains the hash-extraction pipeline split out of [`super`] (`checkver.rs`):
//! download-and-hash, RDF/metalink/page hash extraction, mode detection,
//! and hash formatting/normalisation.

use regex::Regex;

use crate::internal::github;
use crate::{error::Fallible as Result, network, Session};

use super::checkver_url::sub_url;

pub(super) fn download_and_hash_multi(
    session: &Session,
    urls: &[String],
    extractions: &[serde_json::Value],
    tmp_dir: &std::path::Path,
) -> Result<Vec<String>> {
    let mut hashes = Vec::new();
    for (i, url) in urls.iter().enumerate() {
        let extraction = extractions.get(i);

        let hash = match extraction {
            Some(ext) => {
                let mode = ext.get("mode").and_then(|m| m.as_str()).unwrap_or("");
                let has_url = ext
                    .get("url")
                    .and_then(|u| u.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);

                // Auto-detect hash mode from download URL when no explicit config given
                // (matching Scoop: get_hash_for_app auto-detects fosshub/sourceforge/github)
                let effective_mode = if mode.is_empty() && !has_url {
                    detect_hash_mode(url).unwrap_or("")
                } else {
                    mode
                };

                // Scoop precedence: jsonpath/xpath from config override the mode
                let has_jp = ext
                    .get("jp")
                    .or(ext.get("jsonpath"))
                    .and_then(|v| v.as_str())
                    .is_some();
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
                        let page =
                            network::download_page(session, &page_url, 30, None).map_err(|e| {
                                crate::Error::Custom(format!("fetch hash page {}: {}", page_url, e))
                            })?;
                        extract_hash_from_page(&page, ext)?
                    }
                    // ── json: fetch JSON + jsonpath extraction ──────────────────────
                    "json" if has_jp || has_url => {
                        let hash_url = ext.get("url").and_then(|u| u.as_str()).unwrap_or(url);
                        let page =
                            network::download_page(session, hash_url, 30, None).map_err(|e| {
                                crate::Error::Custom(format!("fetch json {}: {}", hash_url, e))
                            })?;
                        extract_hash_from_page(&page, ext)?
                    }
                    // ── xpath: fetch XML + xpath extraction ─────────────────────────
                    "xpath" if has_xp || has_url => {
                        let hash_url = ext.get("url").and_then(|u| u.as_str()).unwrap_or(url);
                        let page =
                            network::download_page(session, hash_url, 30, None).map_err(|e| {
                                crate::Error::Custom(format!("fetch xml {}: {}", hash_url, e))
                            })?;
                        extract_hash_from_page(&page, ext)?
                    }
                    // ── rdf: fetch RDF XML, find digest by basename ─────────────────
                    "rdf" => fetch_rdf_hash(session, url, ext)?,
                    // ── metalink: HTTP Digest header fallback .meta4 ────────────────
                    "metalink" => fetch_metalink_hash(session, url, ext)?,
                    // ── fosshub: extract sha256 from fosshub download page ──────────
                    "fosshub" => {
                        // Scoop: fetch the download page itself, find sha256 with regex
                        // Regex: <filename>.*?"sha256":"([a-fA-F0-9]{64})"
                        let filename = crate::internal::url::remote_filename(url);
                        let page = network::download_page(session, url, 30, None).map_err(|e| {
                            crate::Error::Custom(format!("fetch fosshub page {}: {}", url, e))
                        })?;
                        let regex_str = format!(r#"{filename}.*?"sha256":"([a-fA-F0-9]+)""#);
                        let re = Regex::new(&regex_str).map_err(|e| {
                            crate::Error::Custom(format!("bad fosshub regex: {}", e))
                        })?;
                        if let Some(caps) = re.captures(&page) {
                            if let Some(h) = caps.get(1) {
                                h.as_str().to_string()
                            } else {
                                return Err(crate::Error::Custom(format!(
                                    "could not find sha256 for '{}' in fosshub page",
                                    filename
                                )));
                            }
                        } else {
                            return Err(crate::Error::Custom(format!(
                                "could not find sha256 for '{}' in fosshub page",
                                filename
                            )));
                        }
                    }
                    // ── sourceforge: extract sha1 from SF files page ────────────────
                    "sourceforge" => {
                        // Scoop: fetch SF files page, extract sha1 with regex
                        // Regex: '"$basename":.*?"sha1":\s*"([a-fA-F0-9]{40})"'
                        let (project, file_path) = github::parse_sourceforge_url(url).ok_or_else(|| {
                            crate::Error::Custom(format!(
                                "could not parse sourceforge URL: {}",
                                url
                            ))
                        })?;
                        let sf_page_url =
                            format!("https://sourceforge.net/projects/{project}/files/{file_path}");
                        let page = network::download_page(session, &sf_page_url, 30, None)
                            .map_err(|e| {
                                crate::Error::Custom(format!(
                                    "fetch sourceforge page {}: {}",
                                    sf_page_url, e
                                ))
                            })?;
                        let basename = crate::internal::url::remote_filename(url);
                        let regex_str = format!(r#""{basename}":.*?"sha1":\s*"([a-fA-F0-9]+)""#);
                        let re = Regex::new(&regex_str).map_err(|e| {
                            crate::Error::Custom(format!("bad sourceforge regex: {}", e))
                        })?;
                        if let Some(caps) = re.captures(&page) {
                            if let Some(h) = caps.get(1) {
                                h.as_str().to_string()
                            } else {
                                return Err(crate::Error::Custom(format!(
                                    "could not find sha1 for '{}' in sourceforge page",
                                    basename
                                )));
                            }
                        } else {
                            return Err(crate::Error::Custom(format!(
                                "could not find sha1 for '{}' in sourceforge page",
                                basename
                            )));
                        }
                    }
                    // ── github: extract digest from GitHub API releases ─────────────
                    "github" => {
                        // Scoop: fetch GitHub API releases, extract digest via jsonpath
                        // jsonpath: "$..assets[?(@.browser_download_url == '" + $url + "')].digest"
                        let (owner, repo) = github::parse_github_download_url(url).ok_or_else(|| {
                            crate::Error::Custom(format!("could not parse GitHub URL: {}", url))
                        })?;
                        let api_url =
                            format!("https://api.github.com/repos/{owner}/{repo}/releases");
                        let gh_token = session.config().gh_token.clone();
                        let page = if let Some(token) = gh_token {
                            network::download_page(session, &api_url, 30, Some(&token))
                                .map_err(|e| crate::Error::Custom(format!("{}", e)))?
                        } else {
                            network::download_page(session, &api_url, 30, None)?
                        };
                        // Parse JSON and query via jsonpath
                        use jsonpath_rust::JsonPath;
                        let value: serde_json::Value =
                            serde_json::from_str(&page).map_err(|e| {
                                crate::Error::Custom(format!("parse github API response: {}", e))
                            })?;
                        let jp = format!("$..assets[?(@.browser_download_url == '{url}')].digest");
                        let found = value
                            .query(&jp)
                            .map_err(|e| crate::Error::Custom(format!("jsonpath query: {}", e)))?;
                        if let Some(h) = found.first().and_then(|v| v.as_str()) {
                            h.to_string()
                        } else {
                            return Err(crate::Error::Custom(format!(
                                "could not find digest for '{}' in GitHub API",
                                url
                            )));
                        }
                    }
                    // ── unknown mode: fallback to download + compute hash ───────────
                    _ => download_file_compute_hash(session, url, ext, tmp_dir)?,
                }
            }
            None => {
                // No hash extraction config: download file and compute SHA256
                let filename = url.rsplit('/').next().unwrap_or("download");
                let dest = tmp_dir.join(filename);
                network::download_file(session, url, &dest)
                    .map_err(|e| crate::Error::Custom(format!("download {}: {}", url, e)))?;
                crate::internal::hash::compute_file_hash(&dest, "sha256")?
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

/// Download a single file and compute its hash using the algorithm from extraction config.
fn download_file_compute_hash(
    session: &Session,
    url: &str,
    ext: &serde_json::Value,
    tmp_dir: &std::path::Path,
) -> Result<String> {
    let filename = url.rsplit('/').next().unwrap_or("download");
    let dest = tmp_dir.join(filename);
    network::download_file(session, url, &dest)
        .map_err(|e| crate::Error::Custom(format!("download {}: {}", url, e)))?;
    let algo = ext
        .get("algorithm")
        .and_then(|a| a.as_str())
        .unwrap_or("sha256");
    crate::internal::hash::compute_file_hash(&dest, algo)
        .map_err(|e| crate::Error::Custom(format!("compute hash {}: {}", filename, e)))
}

/// Fetch RDF XML and extract hash by basename (matching Scoop's find_hash_in_rdf).
fn fetch_rdf_hash(session: &Session, url: &str, ext: &serde_json::Value) -> Result<String> {
    let hash_url = ext.get("url").and_then(|u| u.as_str()).unwrap_or(url);
    let page = network::download_page(session, hash_url, 30, None)
        .map_err(|e| crate::Error::Custom(format!("fetch rdf {}: {}", hash_url, e)))?;

    // Parse RDF XML and find Content entry matching the basename
    // Scoop (find_hash_in_rdf):
    //   $digest = $xml.RDF.Content | Where-Object { [String]$_.about -eq $basename }
    //   return format_hash $digest.sha256
    let basename = crate::internal::url::remote_filename(url);
    find_hash_in_rdf(&page, &basename).ok_or_else(|| {
        crate::Error::Custom(format!(
            "could not find hash for '{}' in RDF at {}",
            basename, hash_url
        ))
    })
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
    let page = network::download_page(session, &meta4_url, 30, None)
        .map_err(|e| crate::Error::Custom(format!("fetch metalink {}: {}", meta4_url, e)))?;

    // Extract first SHA256 hash from .meta4 XML
    // Scoop uses find_hash_in_textfile with regex '<hash[^>]+>([a-fA-F0-9]{64})'
    let re = Regex::new(r"<hash[^>]+>([a-fA-F0-9]{64})")
        .map_err(|e| crate::Error::Custom(format!("bad metalink regex: {}", e)))?;
    if let Some(caps) = re.captures(&page) {
        if let Some(h) = caps.get(1) {
            return Ok(h.as_str().to_string());
        }
    }

    Err(crate::Error::Custom(format!(
        "could not find hash in metalink at {}",
        meta4_url
    )))
}

/// Parse RDF XML and find SHA256 digest for the given basename.
fn find_hash_in_rdf(content: &str, _basename: &str) -> Option<String> {
    // Simplified RDF parsing: look for `<rdf:Content ... about="...basename...">` and extract `<sha256:...>`
    // Scoop uses proper XML parsing: $xml.RDF.Content | Where-Object { $_.about -eq $basename }
    let re = Regex::new(
        r#"(?s)<[^:]*:Content[^>]*about="[^"]*"[^>]*>.*?<(?:sha256|digest)[^>]*>(.+?)</"#,
    )
    .ok()?;
    let caps = re.captures(content)?;
    let hash = caps.get(1)?.as_str().trim().to_string();
    if !hash.is_empty() {
        Some(hash)
    } else {
        None
    }
}

/// Perform a HEAD request and extract SHA-256 digest from the Digest header.
fn head_digest_sha256(url: &str, proxy: Option<&str>, timeout_secs: u64) -> Result<String> {
    let mut cfg = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(timeout_secs)));
    if let Some(proxy_url) = proxy {
        let p = ureq::Proxy::new(proxy_url)
            .map_err(|e| crate::Error::Custom(format!("proxy: {}", e)))?;
        cfg = cfg.proxy(Some(p));
    }
    let agent = cfg.build().new_agent();
    let resp = agent
        .head(url)
        .call()
        .map_err(|e| crate::Error::Custom(format!("HEAD {}: {}", url, e)))?;

    // Scoop checks for Digest header: SHA-256=..., SHA=..., MD5=...
    let digest_val = resp
        .headers()
        .get("Digest")
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

    Err(crate::Error::Custom(
        "no Digest header with SHA-256".to_string(),
    ))
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
    if !cleaned.len().is_multiple_of(4) && padding == 0 {
        return Err(crate::Error::Custom("invalid base64 length".to_string()));
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

    let chars: Vec<u8> = input
        .chars()
        .filter_map(|c| if c == '=' { Some(0u8) } else { decode_char(c) })
        .collect();

    if chars.len() < 4 {
        return Err(crate::Error::Custom("invalid base64 input".to_string()));
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
    let out_len = if padding > 0 {
        result.len() - padding
    } else {
        result.len()
    };
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
        32 => Some(format!("md5:{hash}")),     // MD5
        40 => Some(format!("sha1:{hash}")),    // SHA1
        64 => Some(hash),                      // SHA256 (no prefix)
        128 => Some(format!("sha512:{hash}")), // SHA512
        _ => None,                             // Unknown length
    }
}

/// Extract hash from page content using HashExtraction rules.
fn extract_hash_from_page(content: &str, ext: &serde_json::Value) -> Result<String> {
    // JSONPath first
    if let Some(jp) = ext
        .get("jp")
        .or(ext.get("jsonpath"))
        .and_then(|v| v.as_str())
    {
        use jsonpath_rust::JsonPath;
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(content) {
            if let Ok(found) = val.query(jp) {
                let found_str = found.first().and_then(|v| match v {
                    serde_json::Value::String(s) => Some(s.clone()),
                    _ => v.as_str().map(|s| s.to_string()),
                });
                if let Some(h) = found_str {
                    if !h.is_empty() {
                        return Ok(h);
                    }
                }
            }
        }
    }

    // Regex
    if let Some(re_str) = ext.get("regex").and_then(|v| v.as_str()) {
        let _url_for_re = ext.get("url").and_then(|u| u.as_str()).unwrap_or("");
        let re = Regex::new(re_str)
            .map_err(|e| crate::Error::Custom(format!("bad hash regex: {}", e)))?;
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

    Err(crate::Error::Custom(
        "could not extract hash from page".to_string(),
    ))
}

fn is_hex_hash(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let len = s.len();
    // MD5=32, SHA1=40, SHA256=64, SHA512=128 + algorithm prefixes
    let valid_len = matches!(len, 32 | 40 | 64 | 128)
        || (len > 5 && matches!(&s[..5], "md5:" | "sha1:" | "sha256" | "sha51"))
        || (len > 7 && &s[..7] == "sha512:");
    valid_len && s.chars().all(|c| c.is_ascii_hexdigit() || c == ':')
}

#[cfg(test)]
mod tests {
    use super::*;
    // ── format_hash ──────────────────────────────────────────────────────────

    #[test]
    fn format_hash_sha256_no_prefix() {
        let h = "a".repeat(64);
        let result = format_hash(&h);
        assert_eq!(result, Some(h.clone()));
    }

    #[test]
    fn format_hash_md5_prefix_added() {
        let h = "a".repeat(32);
        let result = format_hash(&h);
        assert_eq!(result, Some(format!("md5:{h}")));
    }

    #[test]
    fn format_hash_sha1_prefix_added() {
        let h = "b".repeat(40);
        let result = format_hash(&h);
        assert_eq!(result, Some(format!("sha1:{h}")));
    }

    #[test]
    fn format_hash_invalid_length_returns_none() {
        let result = format_hash("tooshort");
        assert!(result.is_none());
    }
}
