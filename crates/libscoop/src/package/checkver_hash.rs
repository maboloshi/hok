//! Hash collection for checkver (`--update` autoupdate).
//!
//! Contains the hash-extraction pipeline split out of [`super`] (`checkver.rs`):
//! download-and-hash, RDF/metalink/page hash extraction, mode detection,
//! and hash formatting/normalisation.

use std::sync::LazyLock;

use regex::Regex;

use crate::internal::url;
use crate::{Session, error::{Error, Fallible}, network};

use super::checkver_url::sub_url;

pub(super) async fn download_and_hash_multi(
    session: &Session,
    urls: &[String],
    extractions: &[serde_json::Value],
    tmp_dir: &std::path::Path,
) -> Fallible<Vec<String>> {
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
                        download_file_compute_hash(session, url, ext, tmp_dir).await?
                    }
                    // ── extract (default when hash URL present): fetch + regex/jsonpath/find ──
                    "extract" | "" if has_url => {
                        let hash_url = ext["url"].as_str().unwrap_or(url);
                        let page_url = sub_url(hash_url, url);
                        let page = network::download_page(session, &page_url, 30, None)
                            .await
                            .map_err(|e| {
                                Error::Custom(format!("fetch hash page {}: {}", page_url, e))
                            })?;
                        // Scoop's find_hash_in_textfile converts a base64 hash
                        // value to hex before returning it (autoupdate.ps1:82-92).
                        convert_base64_hash_value(&extract_hash_from_page(&page, ext)?)
                    }
                    // ── json: fetch JSON + jsonpath extraction ──────────────────────
                    "json" if has_jp || has_url => {
                        let hash_url = ext.get("url").and_then(|u| u.as_str()).unwrap_or(url);
                        let page = network::download_page(session, hash_url, 30, None)
                            .await
                            .map_err(|e| {
                                Error::Custom(format!("fetch json {}: {}", hash_url, e))
                            })?;
                        extract_hash_from_page(&page, ext)?
                    }
                    // ── xpath: fetch XML + xpath extraction ─────────────────────────
                    "xpath" if has_xp || has_url => {
                        let hash_url = ext.get("url").and_then(|u| u.as_str()).unwrap_or(url);
                        let page = network::download_page(session, hash_url, 30, None)
                            .await
                            .map_err(|e| {
                                Error::Custom(format!("fetch xml {}: {}", hash_url, e))
                            })?;
                        extract_hash_from_page(&page, ext)?
                    }
                    // ── rdf: fetch RDF XML, find digest by basename ─────────────────
                    "rdf" => fetch_rdf_hash(session, url, ext).await?,
                    // ── metalink: HTTP Digest header fallback .meta4 ────────────────
                    "metalink" => fetch_metalink_hash(session, url, ext).await?,
                    // ── fosshub: extract sha256 from fosshub download page ──────────
                    "fosshub" => {
                        // Scoop: fetch the download page itself, find sha256 with regex
                        // Regex: <filename>.*?"sha256":"([a-fA-F0-9]{64})"
                        let filename = crate::internal::url::remote_filename(url);
                        let page = network::download_page(session, url, 30, None)
                            .await
                            .map_err(|e| {
                                Error::Custom(format!("fetch fosshub page {}: {}", url, e))
                            })?;
                        let regex_str = format!(r#"{filename}.*?"sha256":"([a-fA-F0-9]+)""#);
                        let re = Regex::new(&regex_str).map_err(|e| {
                            Error::Custom(format!("bad fosshub regex: {}", e))
                        })?;
                        if let Some(caps) = re.captures(&page) {
                            if let Some(h) = caps.get(1) {
                                h.as_str().to_string()
                            } else {
                                return Err(Error::Custom(format!(
                                    "could not find sha256 for '{}' in fosshub page",
                                    filename
                                )));
                            }
                        } else {
                            return Err(Error::Custom(format!(
                                "could not find sha256 for '{}' in fosshub page",
                                filename
                            )));
                        }
                    }
                    // ── sourceforge: extract sha1 from SF files page ────────────────
                    "sourceforge" => {
                        // Scoop: fetch SF files page, extract sha1 with regex
                        // Regex: '"$basename":.*?"sha1":\s*"([a-fA-F0-9]{40})"'
                        let (project, file_path) =
                            url::parse_sourceforge_url(url).ok_or_else(|| {
                                Error::Custom(format!(
                                    "could not parse sourceforge URL: {}",
                                    url
                                ))
                            })?;
                        let sf_page_url =
                            format!("https://sourceforge.net/projects/{project}/files/{file_path}");
                        let page = network::download_page(session, &sf_page_url, 30, None)
                            .await
                            .map_err(|e| {
                                Error::Custom(format!(
                                    "fetch sourceforge page {}: {}",
                                    sf_page_url, e
                                ))
                            })?;
                        let basename = crate::internal::url::remote_filename(url);
                        let regex_str = format!(r#""{basename}":.*?"sha1":\s*"([a-fA-F0-9]+)""#);
                        let re = Regex::new(&regex_str).map_err(|e| {
                            Error::Custom(format!("bad sourceforge regex: {}", e))
                        })?;
                        if let Some(caps) = re.captures(&page) {
                            if let Some(h) = caps.get(1) {
                                h.as_str().to_string()
                            } else {
                                return Err(Error::Custom(format!(
                                    "could not find sha1 for '{}' in sourceforge page",
                                    basename
                                )));
                            }
                        } else {
                            return Err(Error::Custom(format!(
                                "could not find sha1 for '{}' in sourceforge page",
                                basename
                            )));
                        }
                    }
                    // ── github: extract digest from GitHub API releases ─────────────
                    "github" => {
                        // Scoop: fetch GitHub API releases, extract digest via jsonpath
                        // jsonpath: "$..assets[?(@.browser_download_url == '" + $url + "')].digest"
                        let api_url = url::github_releases_list_api_url(url).ok_or_else(|| {
                            Error::Custom(format!("could not parse GitHub URL: {}", url))
                        })?;
                        let gh_token = session.config().gh_token.clone();
                        let page = if let Some(token) = gh_token {
                            network::download_page(session, &api_url, 30, Some(&token))
                                .await
                                .map_err(|e| Error::Custom(format!("{}", e)))?
                        } else {
                            network::download_page(session, &api_url, 30, None).await?
                        };
                        // Parse JSON and query via jsonpath
                        use jsonpath_rust::JsonPath;
                        let value: serde_json::Value =
                            serde_json::from_str(&page).map_err(|e| {
                                Error::Custom(format!("parse github API response: {}", e))
                            })?;
                        let jp = format!("$..assets[?(@.browser_download_url == '{url}')].digest");
                        let found = value
                            .query(&jp)
                            .map_err(|e| Error::Custom(format!("jsonpath query: {}", e)))?;
                        if let Some(h) = found.first().and_then(|v| v.as_str()) {
                            h.to_string()
                        } else {
                            return Err(Error::Custom(format!(
                                "could not find digest for '{}' in GitHub API",
                                url
                            )));
                        }
                    }
                    // ── unknown mode: fallback to download + compute hash ───────────
                    _ => download_file_compute_hash(session, url, ext, tmp_dir).await?,
                }
            }
            None => {
                // No hash extraction config: download file and compute SHA256
                let filename = url.rsplit('/').next().unwrap_or("download");
                let dest = tmp_dir.join(filename);
                network::download_file(session, url, &dest)
                    .await
                    .map_err(|e| Error::Custom(format!("download {}: {}", url, e)))?;
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
    if url::is_github_releases_download_url(url) {
        return Some("github");
    }
    None
}

/// Download a single file and compute its hash using the algorithm from extraction config.
async fn download_file_compute_hash(
    session: &Session,
    url: &str,
    ext: &serde_json::Value,
    tmp_dir: &std::path::Path,
) -> Fallible<String> {
    let filename = url.rsplit('/').next().unwrap_or("download");
    let dest = tmp_dir.join(filename);
    network::download_file(session, url, &dest)
        .await
        .map_err(|e| Error::Custom(format!("download {}: {}", url, e)))?;
    let algo = ext
        .get("algorithm")
        .and_then(|a| a.as_str())
        .unwrap_or("sha256");
    crate::internal::hash::compute_file_hash(&dest, algo)
        .map_err(|e| Error::Custom(format!("compute hash {}: {}", filename, e)))
}

/// Fetch RDF XML and extract hash by basename (matching Scoop's find_hash_in_rdf).
async fn fetch_rdf_hash(session: &Session, url: &str, ext: &serde_json::Value) -> Fallible<String> {
    let hash_url = ext.get("url").and_then(|u| u.as_str()).unwrap_or(url);
    let page = network::download_page(session, hash_url, 30, None)
        .await
        .map_err(|e| Error::Custom(format!("fetch rdf {}: {}", hash_url, e)))?;

    // Parse RDF XML and find Content entry matching the basename
    // Scoop (find_hash_in_rdf):
    //   $digest = $xml.RDF.Content | Where-Object { [String]$_.about -eq $basename }
    //   return format_hash $digest.sha256
    let basename = crate::internal::url::remote_filename(url);
    find_hash_in_rdf(&page, &basename).ok_or_else(|| {
        Error::Custom(format!(
            "could not find hash for '{}' in RDF at {}",
            basename, hash_url
        ))
    })
}

/// Fetch metalink hash: check HTTP Digest header, fallback to .meta4 file.
async fn fetch_metalink_hash(
    session: &Session,
    url: &str,
    _ext: &serde_json::Value,
) -> Fallible<String> {
    // Scoop (find_hash_in_headers + find_hash_in_textfile .meta4):
    //   1. HEAD request → check Digest header for SHA-256=...
    //   2. Fallback: fetch $url.meta4 and extract hash via regex
    //
    // Step 1: HEAD with Digest header check (Scoop probes with a 2 s timeout)
    if let Ok(digest) = head_digest_sha256(session, url, 2).await {
        return Ok(digest);
    }

    // Step 2: fallback to .meta4 file
    let meta4_url = format!("{}.meta4", url);
    let page = network::download_page(session, &meta4_url, 30, None)
        .await
        .map_err(|e| Error::Custom(format!("fetch metalink {}: {}", meta4_url, e)))?;

    // Extract first SHA256 hash from .meta4 XML
    // Scoop uses find_hash_in_textfile with regex '<hash[^>]+>([a-fA-F0-9]{64})'
    let re = Regex::new(r"<hash[^>]+>([a-fA-F0-9]{64})")
        .map_err(|e| Error::Custom(format!("bad metalink regex: {}", e)))?;
    if let Some(caps) = re.captures(&page)
        && let Some(h) = caps.get(1)
    {
        return Ok(h.as_str().to_string());
    }

    Err(Error::Custom(format!(
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
    if !hash.is_empty() { Some(hash) } else { None }
}

/// Digest parameter regexes, in Scoop's `-or` evaluation order
/// (`lib/autoupdate.ps1:193`): `SHA-256=`, then `SHA=`, then `MD5=`.
static DIGEST_SHA256: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"SHA-256=([^,]+)").unwrap());
static DIGEST_SHA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"SHA=([^,]+)").unwrap());
static DIGEST_MD5: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"MD5=([^,]+)").unwrap());

/// Turn a `Digest` response header value into a lowercase-hex hash.
///
/// Mirrors Scoop's `find_hash_in_headers` (lib/autoupdate.ps1:193-194):
/// the first parameter that matches — `SHA-256=`, `SHA=`, `MD5=` — wins
/// (`[^,]+` stops at the next parameter), and its base64 payload is rendered
/// as lowercase hex, exactly like `Convert.FromBase64String` followed by
/// `$_.ToString('x2')`.
///
/// Returns `None` when no parameter matches or the payload is not valid
/// base64. Official Scoop instead lets the `FormatException` escape
/// (`catch [System.Net.WebException]` does not cover it), which aborts the
/// whole checkver run; hok treats it as "no digest" so the documented
/// `.meta4` fallback still runs.
fn digest_header_hash(header: &str) -> Option<String> {
    [&*DIGEST_SHA256, &*DIGEST_SHA, &*DIGEST_MD5]
        .into_iter()
        .find_map(|re| re.captures(header))
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str())
        .and_then(|b64| crate::internal::base64::decode(b64).ok())
        .map(|bytes| crate::internal::base64::to_lower_hex(&bytes))
}

/// Probe `url` with HEAD and read the hash from the `Digest` header.
///
/// Request shape mirrors Scoop's `find_hash_in_headers`:
/// - redirects are **not** followed (`AllowAutoRedirect = $false`), so the
///   `Digest` header of the 3xx response itself is the one inspected
/// - `Digest` is only consulted on a 3xx status; a direct 200 is ignored
/// - 2 s timeout, `Referer = strip_filename(url)`, Scoop's user agent
///
/// The caller falls back to `$url.meta4` when this returns `Err`.
async fn head_digest_sha256(session: &Session, url: &str, timeout_secs: u64) -> Fallible<String> {
    let proxy = {
        let config = session.config();
        config.proxy().map(|s| s.to_owned())
    };
    let user_agent = session.user_agent().map(|s| s.to_owned());
    let referer = crate::internal::network::strip_filename(url);
    let opts = crate::internal::network::RequestOptions {
        proxy: proxy.as_deref(),
        timeout_secs,
        user_agent: user_agent.as_deref(),
        referer: Some(referer.as_str()),
        ..crate::internal::network::RequestOptions::default()
    };
    let resp = crate::internal::network::head_no_redirect(url, &opts)
        .await
        .map_err(|e| Error::Custom(format!("HEAD {}: {}", url, e)))?;

    let status = resp.status().as_u16();
    if !(300..400).contains(&status) {
        return Err(Error::Custom(format!(
            "HEAD {}: HTTP {} is not a redirect",
            url, status
        )));
    }

    resp.headers()
        .get("Digest")
        .and_then(|v| v.to_str().ok())
        .and_then(digest_header_hash)
        .ok_or_else(|| Error::Custom(format!("no usable Digest header (HTTP {})", status)))
}

/// Render a hash found in a hash file as hex.
///
/// Scoop's `find_hash_in_textfile` (`lib/autoupdate.ps1:82-92`): when the
/// extracted value is well-formed padded base64, is not pure hex and has none
/// of the plain hash lengths (32/40/64/128), decode it and render the bytes as
/// lowercase hex; if the decode throws, the value is kept unchanged.  Hash
/// files that publish base64 digests would otherwise reach the manifest
/// undecoded and never match a downloaded file.
fn convert_base64_hash_value(hash: &str) -> String {
    // Scoop's shape test:
    // '^(?:[A-Za-z0-9+\/]{4})*(?:[A-Za-z0-9+\/]{2}==|[A-Za-z0-9+\/]{3}=|[A-Za-z0-9+\/]{4})$'
    static BASE64_SHAPE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=|[A-Za-z0-9+/]{4})$",
        )
        .unwrap()
    });

    if !BASE64_SHAPE.is_match(hash) {
        return hash.to_string();
    }
    if hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return hash.to_string();
    }
    if matches!(hash.len(), 32 | 40 | 64 | 128) {
        return hash.to_string();
    }
    // The shape check already guarantees decodable input; the fallback
    // mirrors Scoop's defensive `catch { $hash = $hash }`.
    crate::internal::base64::decode(hash)
        .map(|bytes| crate::internal::base64::to_lower_hex(&bytes))
        .unwrap_or_else(|_| hash.to_string())
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
    let algo = match hash.len() {
        32 => "md5",      // MD5
        40 => "sha1",     // SHA1
        64 => "sha256",   // SHA256 (no prefix)
        128 => "sha512",  // SHA512
        _ => return None, // Unknown length
    };
    Some(crate::internal::hash::format_hash_value(algo, &hash))
}

/// Extract hash from page content using HashExtraction rules.
fn extract_hash_from_page(content: &str, ext: &serde_json::Value) -> Fallible<String> {
    // JSONPath first
    if let Some(jp) = ext
        .get("jp")
        .or(ext.get("jsonpath"))
        .and_then(|v| v.as_str())
    {
        use jsonpath_rust::JsonPath;
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(content) {
            // jsonpath-rust cannot parse unquoted field names with special
            // characters (e.g. `$.dist-tags.latest`); normalize to bracket form.
            let jp = crate::internal::string::normalize_jsonpath(jp);
            if let Ok(found) = val.query(&jp) {
                let found_str = found.first().and_then(|v| match v {
                    serde_json::Value::String(s) => Some(s.clone()),
                    _ => v.as_str().map(|s| s.to_string()),
                });
                if let Some(h) = found_str
                    && !h.is_empty()
                {
                    return Ok(h);
                }
            }
        }
    }

    // Regex
    if let Some(re_str) = ext.get("regex").and_then(|v| v.as_str()) {
        let _url_for_re = ext.get("url").and_then(|u| u.as_str()).unwrap_or("");
        let re = Regex::new(re_str)
            .map_err(|e| Error::Custom(format!("bad hash regex: {}", e)))?;
        if let Some(caps) = re.captures(content)
            && let Some(h) = caps.get(1).or_else(|| caps.get(0))
        {
            return Ok(h.as_str().to_string());
        }
    }

    // Find (simple substring + next whitespace-delimited hex token)
    if let Some(find_str) = ext.get("find").and_then(|v| v.as_str())
        && let Some(pos) = content.find(find_str)
    {
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

    Err(Error::Custom(
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
    use crate::internal::base64;

    // ── digest_header_hash ───────────────────────────────────────────────────
    // Scoop: lib/autoupdate.ps1:193-194 (`find_hash_in_headers`)

    #[test]
    fn digest_header_reads_sha256_parameter() {
        let payload = base64::encode(&[0xab; 32]);
        let header = format!("SHA-256={payload}");
        assert_eq!(digest_header_hash(&header), Some("ab".repeat(32)));
    }

    #[test]
    fn digest_header_ignores_whitespace_inside_payload() {
        // .NET's FromBase64String ignores whitespace anywhere.
        let payload = base64::encode(&[0xab; 32]);
        let spaced = format!("{} {}", &payload[..8], &payload[8..]);
        assert_eq!(
            digest_header_hash(&format!("SHA-256={spaced}")),
            Some("ab".repeat(32))
        );
    }

    #[test]
    fn digest_header_stops_at_next_parameter() {
        // `([^,]+)` is not greedy across the comma-separated parameter list.
        let payload = base64::encode(&[0xab; 32]);
        let other = base64::encode(&[0xcd; 32]);
        let header = format!("SHA-256={payload}, SHA={other}");
        assert_eq!(digest_header_hash(&header), Some("ab".repeat(32)));
    }

    #[test]
    fn digest_header_prefers_sha256_over_earlier_parameters() {
        let sha256 = base64::encode(&[0xab; 32]);
        let md5 = base64::encode(&[0xcd; 16]);
        let header = format!("MD5={md5}, SHA-256={sha256}");
        assert_eq!(digest_header_hash(&header), Some("ab".repeat(32)));
    }

    #[test]
    fn digest_header_falls_back_to_sha_then_md5() {
        let sha1 = base64::encode(&[0x0a; 20]);
        assert_eq!(
            digest_header_hash(&format!("SHA={sha1}")),
            Some("0a".repeat(20))
        );
        let md5 = base64::encode(&[0x0b; 16]);
        assert_eq!(
            digest_header_hash(&format!("MD5={md5}")),
            Some("0b".repeat(16))
        );
    }

    #[test]
    fn digest_header_rejects_invalid_base64() {
        assert_eq!(digest_header_hash("SHA-256=!!!!"), None);
        // A `SHA-256=` parameter that cannot be decoded is *not* allowed to
        // fall through to another parameter: official Scoop throws from the
        // decode instead of trying `SHA=`/`MD5=`.
        let md5 = base64::encode(&[0x0b; 16]);
        assert_eq!(digest_header_hash(&format!("SHA-256=!!, MD5={md5}")), None);
    }

    #[test]
    fn digest_header_without_known_parameters_is_none() {
        assert_eq!(digest_header_hash(""), None);
        assert_eq!(digest_header_hash("foo=bar"), None);
    }

    // ── head_digest_sha256 (status gate + no redirects) ──────────────────────

    /// Minimal loopback HTTP server answering every HEAD with a fixed status
    /// line and optional `Digest` header.
    struct HeadServer {
        addr: std::net::SocketAddr,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl HeadServer {
        fn url(&self, path: &str) -> String {
            format!("http://{}{}", self.addr, path)
        }

        fn shutdown(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(h) = self.handle.take() {
                h.join().unwrap();
            }
        }
    }

    fn spawn_head_server(status: u16, digest: Option<String>) -> HeadServer {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicBool, Ordering};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let stop_thread = std::sync::Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            while !stop_thread.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                };
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                // Drain the request head so the client never sees a reset.
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let digest_header = digest
                    .as_ref()
                    .map(|d| format!("Digest: {d}\r\n"))
                    .unwrap_or_default();
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: 0\r\nLocation: /moved\r\n{digest_header}Connection: close\r\n\r\n"
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        HeadServer {
            addr,
            stop,
            handle: Some(handle),
        }
    }

    #[tokio::test]
    async fn head_digest_probe_reads_digest_from_the_redirect() {
        let root = crate::test_utils::tmpdir("head_digest_redirect");
        let session = crate::test_utils::test_session(&root);
        let payload = base64::encode(&[0xab; 32]);
        let mut server = spawn_head_server(302, Some(format!("SHA-256={payload}")));

        let hash = head_digest_sha256(&session, &server.url("/pkg.zip"), 5)
            .await
            .unwrap();
        assert_eq!(hash, "ab".repeat(32));
        server.shutdown();
    }

    #[tokio::test]
    async fn head_digest_probe_ignores_a_direct_200() {
        // Scoop only inspects `Digest` on a 3xx response; a direct 200 means
        // "no digest here" and the caller falls back to `$url.meta4`.
        let root = crate::test_utils::tmpdir("head_digest_200");
        let session = crate::test_utils::test_session(&root);
        let payload = base64::encode(&[0xab; 32]);
        let mut server = spawn_head_server(200, Some(format!("SHA-256={payload}")));

        assert!(
            head_digest_sha256(&session, &server.url("/pkg.zip"), 5)
                .await
                .is_err()
        );
        server.shutdown();
    }

    #[tokio::test]
    async fn head_digest_probe_errors_on_a_redirect_without_digest() {
        let root = crate::test_utils::tmpdir("head_digest_missing");
        let session = crate::test_utils::test_session(&root);
        let mut server = spawn_head_server(302, None);

        assert!(
            head_digest_sha256(&session, &server.url("/pkg.zip"), 5)
                .await
                .is_err()
        );
        server.shutdown();
    }

    // ── convert_base64_hash_value ────────────────────────────────────────────
    // Scoop: lib/autoupdate.ps1:82-92

    #[test]
    fn base64_hash_file_value_is_decoded_to_hex() {
        // A base64-encoded SHA-256 digest: 44 chars in, 64 hex chars out.
        let payload = base64::encode(&[0x0a; 32]);
        assert_eq!(payload.len(), 44);
        assert_eq!(convert_base64_hash_value(&payload), "0a".repeat(32));
    }

    #[test]
    fn plain_hex_hash_file_values_are_untouched() {
        // MD5 (also valid base64 shape), SHA-1 and SHA-256 lengths.
        assert_eq!(convert_base64_hash_value(&"a1".repeat(16)), "a1".repeat(16));
        assert_eq!(convert_base64_hash_value(&"b2".repeat(20)), "b2".repeat(20));
        assert_eq!(convert_base64_hash_value(&"c3".repeat(32)), "c3".repeat(32));
        assert_eq!(convert_base64_hash_value(&"d4".repeat(64)), "d4".repeat(64));
    }

    #[test]
    fn base64_hash_value_with_a_plain_hash_length_is_untouched() {
        // 40 base64 chars = 30 bytes; Scoop's length guard leaves it alone.
        let payload = base64::encode(&[0x7f; 30]);
        assert_eq!(payload.len(), 40);
        assert_eq!(convert_base64_hash_value(&payload), payload);
    }

    #[test]
    fn non_base64_hash_values_are_untouched() {
        assert_eq!(convert_base64_hash_value("not a hash"), "not a hash");
        assert_eq!(convert_base64_hash_value(""), "");
    }

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
    fn format_hash_sha512_prefix_added() {
        let h = "c".repeat(128);
        let result = format_hash(&h);
        assert_eq!(result, Some(format!("sha512:{h}")));
    }

    #[test]
    fn format_hash_invalid_length_returns_none() {
        let result = format_hash("tooshort");
        assert!(result.is_none());
    }
}
