//! Hash collection for checkver (`--update` autoupdate).
//!
//! A faithful port of upstream `lib/autoupdate.ps1`: [`get_hash_for_app`] is
//! the orchestrator (mode resolution, auto-detection, download fallback), the
//! `find_hash_in_*` helpers each implement one hash-file shape, and
//! [`format_hash`] normalises the result.
//!
//! Two upstream traits are deliberate and load-bearing:
//!
//! * a hash file that yields nothing (or something of an unusable length) is
//!   **not** an error — upstream falls back to downloading the artefact and
//!   computing SHA-256;
//! * `-match` is case-insensitive in PowerShell, so every hash-file pattern is
//!   compiled case-insensitively here.

use std::io::Read;
use std::sync::LazyLock;

use flate2::read::GzDecoder;
use regex::{Regex, RegexBuilder};

use crate::internal::url;
use crate::{
    Session,
    error::{Error, Fallible},
    network,
};

use super::Substitution;

// ─── Hash extraction (upstream `get_hash_for_app`, lib/autoupdate.ps1:208-330)

/// `HashHelper` (lib/autoupdate.ps1:581-629): hash every URL, reusing the last
/// extraction template when there are fewer templates than URLs, and fail the
/// whole update when a single URL yields no hash.
pub(super) async fn download_and_hash_multi(
    session: &Session,
    app: &str,
    version: &str,
    urls: &[String],
    extractions: &[serde_json::Value],
    base_subs: &Substitution,
    cache_dir: &std::path::Path,
) -> Fallible<Vec<String>> {
    let mut hashes = Vec::new();
    for (i, url) in urls.iter().enumerate() {
        // Every download URL gets its own `$url`/`$baseurl`/`$basename`/
        // `$urlNoExt`/`$basenameNoExt` table (upstream `get_hash_for_app`,
        // lib/autoupdate.ps1:212-219); the version variables are shared.
        let subs = base_subs.with_url(url);
        // `$HashExtraction[$i], $HashExtraction[-1] | Select-Object -First 1`
        let extraction = extractions.get(i).or_else(|| extractions.last());
        let hash =
            get_hash_for_app(session, app, version, url, extraction, &subs, cache_dir).await?;
        hashes.push(hash);
    }
    Ok(hashes)
}

/// `get_hash_for_app` (lib/autoupdate.ps1:208-330).
///
/// Resolves the extraction mode from the config, auto-detects
/// fosshub/sourceforge/github from the *download* URL when no hash-file URL is
/// configured, and returns the formatted hash. When nothing can be extracted
/// it downloads the artefact into Scoop's cache and hashes it — upstream's
/// documented fallback, not an error path.
async fn get_hash_for_app(
    session: &Session,
    app: &str,
    version: &str,
    url: &str,
    config: Option<&serde_json::Value>,
    subs: &Substitution,
    cache_dir: &std::path::Path,
) -> Fallible<String> {
    let null = serde_json::Value::Null;
    let cfg = config.unwrap_or(&null);

    let mut hashmode = cfg
        .get("mode")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    // `$basename = [System.Web.HttpUtility]::UrlDecode((url_remote_filename($url)))`
    let basename = subs.apply("$basename");

    let config_url = cfg.get("url").and_then(|u| u.as_str()).unwrap_or("");
    let hashfile_url = subs.apply(config_url);

    // `if ($hashmode.Length -eq 0 -and $config.url.Length -ne 0) { $hashmode = 'extract' }`
    if hashmode.is_empty() && !config_url.is_empty() {
        hashmode = "extract".to_string();
    }

    // jsonpath / regex / xpath override the mode, in upstream's order
    // (`find` first, then `regex`, which wins).
    let mut jsonpath = String::new();
    if let Some(jp) = cfg.get("jp").and_then(|v| v.as_str()) {
        jsonpath = jp.to_string();
        hashmode = "json".to_string();
    }
    if let Some(jp) = cfg.get("jsonpath").and_then(|v| v.as_str()) {
        jsonpath = jp.to_string();
        hashmode = "json".to_string();
    }
    let mut regex = String::new();
    if let Some(find) = cfg.get("find").and_then(|v| v.as_str()) {
        regex = find.to_string();
    }
    if let Some(re) = cfg.get("regex").and_then(|v| v.as_str()) {
        regex = re.to_string();
    }
    let mut xpath = String::new();
    if let Some(xp) = cfg.get("xpath").and_then(|v| v.as_str()) {
        xpath = xp.to_string();
        hashmode = "xpath".to_string();
    }

    // Auto-detect from the download URL — only when no hash-file URL is set.
    // Upstream uses three plain `if`s, so a later match wins.
    if hashfile_url.is_empty() {
        if FOSSHUB_URL.is_match(url) {
            hashmode = "fosshub".to_string();
        }
        if SOURCEFORGE_URL.is_match(url) {
            hashmode = "sourceforge".to_string();
        }
        if GITHUB_RELEASE_URL.is_match(url) {
            hashmode = "github".to_string();
        }
    }

    let gh_token = session.config().gh_token.clone();
    let token = gh_token.as_deref();

    let hash: Option<String> = match hashmode.as_str() {
        "extract" => find_hash_in_textfile(session, &hashfile_url, subs, &regex).await,
        "json" => find_hash_in_json(session, &hashfile_url, subs, &jsonpath, token).await,
        "xpath" => find_hash_in_xml(session, &hashfile_url, subs, &xpath).await,
        "rdf" => find_hash_in_rdf(session, &hashfile_url, &basename).await,
        "metalink" => match find_hash_in_headers(session, url).await {
            Some(hash) => Some(hash),
            // `find_hash_in_textfile "$url.meta4" $subscriptions` — no regex, so
            // the default + metalink fallback apply.
            None => find_hash_in_textfile(session, &format!("{url}.meta4"), subs, "").await,
        },
        "fosshub" => {
            // `$matches.filename` from the fosshub URL pattern.
            let filename = FOSSHUB_URL
                .captures(url)
                .and_then(|c| c.name("filename"))
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            let pattern = format!(r#"{filename}.*?"sha256":"([a-fA-F0-9]{{64}})""#);
            find_hash_in_textfile(session, url, subs, &pattern).await
        }
        "sourceforge" => {
            // The download host carries no checksums, so upstream rewrites the
            // URL to the project's files page before extracting the sha1.
            let sf_url = SOURCEFORGE_URL.captures(url).map(|c| {
                let project = c.name("project").map(|m| m.as_str()).unwrap_or("");
                let file = c.name("file").map(|m| m.as_str()).unwrap_or("");
                let full = format!("https://sourceforge.net/projects/{project}/files/{file}");
                url::strip_filename(url::strip_url_fragment(&full))
                    .trim_end_matches('/')
                    .to_string()
            });
            let Some(sf_url) = sf_url else {
                return download_and_compute_sha256(session, app, version, url, cache_dir).await;
            };
            let pattern = format!(r#""{basename}":.*?"sha1":\s*"([a-fA-F0-9]{{40}})""#);
            find_hash_in_textfile(session, &sf_url, subs, &pattern).await
        }
        "github" => {
            let Some(api_url) = url::github_releases_list_api_url(url) else {
                return download_and_compute_sha256(session, app, version, url, cache_dir).await;
            };
            let jp = format!("$..assets[?(@.browser_download_url == '{url}')].digest");
            find_hash_in_json(session, &api_url, subs, &jp, token).await
        }
        // `download`, an empty mode, and any unrecognised mode all fall through
        // to the download fallback (upstream's switch has no such case).
        _ => None,
    };

    if let Some(hash) = hash {
        return Ok(hash);
    }
    download_and_compute_sha256(session, app, version, url, cache_dir).await
}

/// Download `url` into Scoop's download cache and compute its SHA-256.
///
/// The destination mirrors upstream `Invoke-CachedDownload` →
/// `cache_path $app $version $url` (lib/autoupdate.ps1:319-326), so the file is
/// reusable instead of a throwaway temp copy.
async fn download_and_compute_sha256(
    session: &Session,
    app: &str,
    version: &str,
    url: &str,
    cache_dir: &std::path::Path,
) -> Fallible<String> {
    let dest = cache_dir.join(crate::package::cache_file_name(app, version, url));
    network::download_file(session, url, &dest)
        .await
        .map_err(|e| Error::Custom(format!("download {}: {}", url, e)))?;
    crate::internal::hash::compute_file_hash(&dest, "sha256")
        .map_err(|e| Error::Custom(format!("compute hash {}: {}", url, e)))
}

/// fosshub download URLs (lib/autoupdate.ps1:259).
static FOSSHUB_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:.*fosshub\.com/).*(?:/|\?dwl=)(?<filename>.*)$").unwrap());

/// SourceForge download URLs (lib/autoupdate.ps1:263).
static SOURCEFORGE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:downloads\.)?sourceforge\.net/projects?/(?<project>[^/]+)/(?:files/)?(?<file>.*)",
    )
    .unwrap()
});

/// GitHub release asset URLs (lib/autoupdate.ps1:267).
static GITHUB_RELEASE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https://github\.com/(?<owner>[^/]+)/(?<repo>[^/]+)/releases/download/[^/]+/[^/]+")
        .unwrap()
});

// ─── find_hash_in_* helpers ────────────────────────────────────────────────

/// `find_hash_in_textfile` (lib/autoupdate.ps1:41-110).
///
/// Returns `format_hash`'s result, so an unusable value becomes `None` and the
/// caller falls back to downloading.
async fn find_hash_in_textfile(
    session: &Session,
    url: &str,
    subs: &Substitution,
    regex: &str,
) -> Option<String> {
    let body = fetch_hash_file(session, url, None).await?;
    extract_hash_from_hash_file(&body, subs, regex)
}

/// The pure part of `find_hash_in_textfile`, split out so the hash-file
/// semantics can be tested without a network.
///
/// Truth values for every branch were recorded from PowerShell 5.1 by driving
/// upstream's own post-download logic (2026-09-13).
fn extract_hash_from_hash_file(hashfile: &str, subs: &Substitution, regex: &str) -> Option<String> {
    // `if ($regex.Length -eq 0) { $regex = '^\s*([a-fA-F0-9]+)\s*$' }`
    let regex = if regex.is_empty() {
        r"^\s*([a-fA-F0-9]+)\s*$"
    } else {
        regex
    };
    // Templates first (plain), then the version/URL variables (regex-escaped):
    // `substitute $regex $templates $false` / `substitute $regex $substitutions $true`.
    let regex = subs.apply_escaped(&substitute_hash_templates(regex));

    let mut hash = String::new();
    if let Some(re) = regex_ci(&regex)
        && let Some(captures) = re.captures(hashfile)
        && let Some(group) = captures.get(1)
    {
        // `$matches[1] -replace '\s', ''`
        hash = group
            .as_str()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
    }

    // convert base64 encoded hash values (lib/autoupdate.ps1:82-92)
    hash = convert_base64_hash_value(&hash);

    // find hash with filename in $hashfile (lib/autoupdate.ps1:94-107)
    if hash.is_empty() {
        let filename_regex = substitute_hash_templates(
            r"([a-fA-F0-9]{32,128})[\x20\t]+.*$basename(?:\s|$)|$basename[\x20\t]+.*?([a-fA-F0-9]{32,128})",
        );
        let filename_regex = subs.apply_escaped(&filename_regex);
        if let Some(re) = regex_ci(&filename_regex)
            && let Some(captures) = re.captures(hashfile)
        {
            // Upstream assigns `$matches[1]` only, so the
            // `<basename><ws><hash>` alternative extracts nothing and falls
            // back to downloading. Reproduced deliberately.
            hash = captures
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
        }
        // A separate `if`, not an `elseif`: a metalink body always wins.
        if let Some(captures) = METALINK_HASH.captures(hashfile)
            && let Some(group) = captures.get(1)
        {
            hash = group.as_str().to_string();
        }
    }

    format_hash(&hash)
}

/// `find_hash_in_json` (lib/autoupdate.ps1:112-138).
async fn find_hash_in_json(
    session: &Session,
    url: &str,
    subs: &Substitution,
    jsonpath: &str,
    token: Option<&str>,
) -> Option<String> {
    let body = fetch_hash_file(session, url, token).await?;
    // `json_path` escapes substituted values only for `=~` filters
    // (lib/json.ps1:98).
    let jsonpath = if jsonpath.contains("=~") {
        subs.apply_escaped(jsonpath)
    } else {
        subs.apply(jsonpath)
    };
    let hash = json_path_first(&body, &jsonpath)?;
    format_hash(&hash)
}

/// `json_path $json $jsonpath $substitutions` with upstream's single-result
/// rule (lib/json.ps1:108-120): one token is stringified, several are
/// serialised as a JSON array.
fn json_path_first(json: &str, jsonpath: &str) -> Option<String> {
    use jsonpath_rust::JsonPath;

    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    // jsonpath-rust cannot parse unquoted field names with special characters
    // (e.g. `$.dist-tags.latest`); normalize to bracket form.
    let jsonpath = crate::internal::string::normalize_jsonpath(jsonpath);
    let found = value.query(&jsonpath).ok()?;
    match found.as_slice() {
        [] => None,
        [single] => Some(match single {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Null => return None,
            other => other.to_string(),
        }),
        many => {
            Some(serde_json::Value::Array(many.iter().map(|v| (*v).clone()).collect()).to_string())
        }
    }
}

/// `find_hash_in_xml` (lib/autoupdate.ps1:140-179).
async fn find_hash_in_xml(
    session: &Session,
    url: &str,
    subs: &Substitution,
    xpath: &str,
) -> Option<String> {
    let body = fetch_hash_file(session, url, None).await?;
    let xpath = subs.apply(xpath);
    let hash = super::extract_xpath(&body, &xpath)?;
    format_hash(&hash)
}

/// `find_hash_in_rdf` (lib/autoupdate.ps1:20-39).
async fn find_hash_in_rdf(session: &Session, url: &str, basename: &str) -> Option<String> {
    let body = fetch_hash_file(session, url, None).await?;
    let hash = rdf_digest_for(&body, basename)?;
    format_hash(&hash)
}

/// `$xml.RDF.Content | Where-Object { [String]$_.about -eq $basename }` then
/// `$digest.sha256`.
///
/// Walks the DOM by *local* name like PowerShell's XML adapter (which ignores
/// namespaces), and compares `about` case-insensitively — `-eq` on strings in
/// PowerShell is case-insensitive.
fn rdf_digest_for(content: &str, basename: &str) -> Option<String> {
    use sxd_document::dom::{ChildOfElement, ChildOfRoot, Element};
    use sxd_document::parser;

    fn walk<'d>(element: Element<'d>, name: &str, out: &mut Vec<Element<'d>>) {
        if element.name().local_part() == name {
            out.push(element);
        }
        for child in element.children() {
            if let ChildOfElement::Element(child) = child {
                walk(child, name, out);
            }
        }
    }

    fn descendant_text(element: Element<'_>, name: &str) -> Option<String> {
        for child in element.children() {
            if let ChildOfElement::Element(child) = child {
                if child.name().local_part() == name {
                    return Some(element_text(child));
                }
                if let Some(found) = descendant_text(child, name) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// PowerShell's `$element.'#text'` — the concatenated direct text nodes.
    fn element_text(element: Element<'_>) -> String {
        let mut out = String::new();
        for child in element.children() {
            if let ChildOfElement::Text(text) = child {
                out.push_str(text.text());
            }
        }
        out
    }

    let package = parser::parse(content).ok()?;
    let doc = package.as_document();
    let mut contents = Vec::new();
    for child in doc.root().children() {
        if let ChildOfRoot::Element(element) = child {
            walk(element, "Content", &mut contents);
        }
    }
    for content_el in contents {
        let about = content_el
            .attributes()
            .into_iter()
            .find(|a| a.name().local_part() == "about")
            .map(|a| a.value().to_string())
            .unwrap_or_default();
        if about.eq_ignore_ascii_case(basename)
            && let Some(hash) = descendant_text(content_el, "sha256")
        {
            return Some(hash);
        }
    }
    None
}

/// `find_hash_in_headers` (lib/autoupdate.ps1:181-206).
async fn find_hash_in_headers(session: &Session, url: &str) -> Option<String> {
    let hash = head_digest_sha256(session, url, 2).await.ok()?;
    format_hash(&hash)
}

/// Fetch a hash file and decode it the way upstream's `StreamReader` does
/// (lib/autoupdate.ps1:53-69): sniff the gzip magic, and never fail on a
/// non-UTF-8 body.
async fn fetch_hash_file(session: &Session, url: &str, token: Option<&str>) -> Option<String> {
    if url.is_empty() {
        return None;
    }
    let raw = network::download_bytes(session, url, 30, token)
        .await
        .ok()?;
    // `if ($data[0] -eq 0x1F -and $data[1] -eq 0x8B) { GZipStream }`
    let body = if raw.starts_with(&[0x1f, 0x8b]) {
        let mut inflate = GzDecoder::new(raw.as_slice());
        let mut out = Vec::new();
        match inflate.read_to_end(&mut out) {
            Ok(_) => out,
            Err(_) => raw,
        }
    } else {
        raw
    };
    // Upstream reads with the response's charset (defaulting to UTF-8) and a
    // `StreamReader` never throws, so an undecodable body is replaced rather
    // than aborting the update.
    Some(String::from_utf8_lossy(&body).into_owned())
}

/// Upstream's hash-template table (lib/autoupdate.ps1:44-51), applied with
/// `substitute ... $false` — a plain, case-sensitive replace.
///
/// No key is a prefix of another (`$sha1` is *not* a prefix of `$sha512`),
/// so the order is immaterial; it is kept as upstream declares it.
static HASH_TEMPLATES: &[(&str, &str)] = &[
    ("$md5", "([a-fA-F0-9]{32})"),
    ("$sha1", "([a-fA-F0-9]{40})"),
    ("$sha256", "([a-fA-F0-9]{64})"),
    ("$sha512", "([a-fA-F0-9]{128})"),
    ("$checksum", "([a-fA-F0-9]{32,128})"),
    ("$base64", r"([a-zA-Z0-9+\/=]{24,88})"),
];

fn substitute_hash_templates(pattern: &str) -> String {
    let mut out = pattern.to_string();
    for (key, value) in HASH_TEMPLATES {
        out = out.replace(key, value);
    }
    out
}

/// `$hashfile -match $regex` — PowerShell's `-match` is case-insensitive.
fn regex_ci(pattern: &str) -> Option<Regex> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .ok()
}

/// `<hash[^>]+>([a-fA-F0-9]{64})` — the metalink fallback
/// (lib/autoupdate.ps1:102), matched case-insensitively like upstream.
static METALINK_HASH: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"<hash[^>]+>([a-fA-F0-9]{64})")
        .case_insensitive(true)
        .build()
        .unwrap()
});

// ─── Digest header (find_hash_in_headers) ──────────────────────────────────

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
    let request = crate::network::ScoopRequest::for_url(
        session,
        url,
        crate::network::HeaderProfile::Autoupdate,
    );
    let resp = crate::internal::network::head_no_redirect(url, &request.options(timeout_secs))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::base64;

    /// A `Substitution` whose `$basename` is `app.zip`.
    fn subs() -> Substitution {
        super::super::version_substitutions("1.0", &[], &std::collections::HashMap::new())
            .with_url("https://dl.example.com/app.zip")
    }

    // ── extract_hash_from_hash_file (find_hash_in_textfile) ────────────────
    // Truth values recorded from PowerShell 5.1 by driving upstream's own
    // post-download logic, 2026-09-13.

    fn extract(hashfile: &str, regex: &str) -> Option<String> {
        extract_hash_from_hash_file(hashfile, &subs(), regex)
    }

    #[test]
    fn textfile_default_regex_reads_a_hash_only_line() {
        let hash = "a".repeat(64);
        assert_eq!(
            extract(&hash, ""),
            Some(hash.clone()),
            "the default '^\\s*([a-fA-F0-9]+)\\s*$' must pick up a bare hash line"
        );
        // Whitespace padding is trimmed by `-replace '\s',''`.
        assert_eq!(extract(&format!("  {hash}  "), ""), Some(hash));
    }

    #[test]
    fn textfile_default_regex_rejects_a_hash_plus_filename_line() {
        // The default regex requires the line to be *only* a hash, so a
        // checksums.txt line falls through to the filename fallback.
        let hash = "a".repeat(64);
        assert_eq!(
            extract(&format!("{hash}  app.zip"), ""),
            Some(hash),
            "hash-first lines are matched by the filename fallback"
        );
    }

    #[test]
    fn textfile_filename_fallback_matches_hash_first_only() {
        let hash = "a".repeat(64);
        // `<hash> *<name>` — the first alternative, group 1.
        assert_eq!(extract(&format!("{hash} *app.zip"), ""), Some(hash.clone()));
        // `-match` is case-insensitive, so the basename matches in any case.
        assert_eq!(extract(&format!("{hash} *APP.ZIP"), ""), Some(hash));
        // `<name><ws><hash>` matches only the *second* alternative, whose
        // group is 2 — upstream reads `$matches[1]` and so extracts nothing,
        // then falls back to downloading. Reproduced deliberately.
        assert_eq!(extract(&format!("app.zip  {}", "a".repeat(64)), ""), None);
    }

    #[test]
    fn textfile_finds_md5_and_sha1_lengths() {
        assert_eq!(
            extract(&format!("{}  app.zip", "c".repeat(32)), ""),
            Some(format!("md5:{}", "c".repeat(32)))
        );
        assert_eq!(
            extract(&format!("{}  app.zip", "b".repeat(40)), ""),
            Some(format!("sha1:{}", "b".repeat(40)))
        );
    }

    #[test]
    fn textfile_substitutes_hash_templates() {
        let hash = "a".repeat(64);
        assert_eq!(
            extract(&format!("{hash}  app.zip"), r"$sha256\s+$basename"),
            Some(hash.clone())
        );
        assert_eq!(
            extract(&format!("sha256: {hash}  app.zip"), r"sha256:\s*($sha256)"),
            Some(hash)
        );
        // Every template resolves to its capture group.
        assert_eq!(
            substitute_hash_templates("$sha512"),
            "([a-fA-F0-9]{128})",
            "$sha1 must not eat the $sha512 prefix"
        );
    }

    #[test]
    fn textfile_regex_is_case_insensitive_like_match() {
        // PowerShell's `-match` is case-insensitive, so a lowercase pattern
        // finds uppercase hex and vice versa.
        let lower = "a".repeat(64);
        let upper = "A".repeat(64);
        assert_eq!(
            extract(&format!("sha256: {upper}"), "sha256:\\s*([a-f0-9]{64})"),
            Some(lower.clone())
        );
        assert_eq!(
            extract(&format!("sha256: {lower}"), "sha256:\\s*([A-F0-9]{64})"),
            Some(lower)
        );
    }

    #[test]
    fn textfile_metalink_fallback_reads_hash_tags() {
        let hash = "d".repeat(64);
        assert_eq!(
            extract(&format!("<hash type=\"sha256\">{hash}</hash>"), ""),
            Some(hash)
        );
    }

    #[test]
    fn textfile_without_any_hash_is_none() {
        assert_eq!(extract("no hash here", ""), None);
    }

    #[test]
    fn textfile_unusable_hash_length_falls_through() {
        // 20 hex chars -> format_hash returns None -> the caller downloads.
        assert_eq!(extract(&format!("{}  app.zip", "e".repeat(20)), ""), None);
    }

    // ── json_path_first ────────────────────────────────────────────────────

    #[test]
    fn json_path_first_stringifies_a_single_token() {
        assert_eq!(
            json_path_first(
                r#"{"assets":[{"digest":"sha256:abc"}]}"#,
                "$.assets[0].digest"
            ),
            Some("sha256:abc".to_string())
        );
    }

    #[test]
    fn json_path_first_serialises_multiple_tokens() {
        // Several tokens become a JSON array, whose text is never a valid hash
        // length — upstream then falls back to downloading.
        let text = json_path_first(r#"{"a":["x","y"]}"#, "$.a[*]").unwrap();
        assert!(text.starts_with('['), "got {text}");
    }

    #[test]
    fn json_path_first_missing_token_is_none() {
        assert_eq!(json_path_first(r#"{"a":1}"#, "$.b"), None);
    }

    // ── rdf_digest_for (find_hash_in_rdf) ──────────────────────────────────

    #[test]
    fn rdf_digest_filters_by_about() {
        let hash = "f".repeat(64);
        let hash2 = "0".repeat(64);
        let xml = format!(
            r#"<?xml version="1.0"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Content rdf:about="other.zip"><sha256>{hash2}</sha256></rdf:Content>
  <rdf:Content rdf:about="app.zip"><sha256>{hash}</sha256></rdf:Content>
</rdf:RDF>"#
        );
        assert_eq!(rdf_digest_for(&xml, "app.zip"), Some(hash));
    }

    #[test]
    fn rdf_digest_about_comparison_is_case_insensitive() {
        // PowerShell's `-eq` on strings is case-insensitive.
        let hash = "f".repeat(64);
        let xml = format!(
            r#"<rdf:RDF xmlns:rdf="x"><rdf:Content rdf:about="APP.ZIP"><sha256>{hash}</sha256></rdf:Content></rdf:RDF>"#
        );
        assert_eq!(rdf_digest_for(&xml, "app.zip"), Some(hash));
    }

    #[test]
    fn rdf_digest_without_a_matching_entry_is_none() {
        let xml = r#"<rdf:RDF xmlns:rdf="x"><rdf:Content rdf:about="other.zip"><sha256>ab</sha256></rdf:Content></rdf:RDF>"#;
        assert_eq!(rdf_digest_for(xml, "app.zip"), None);
    }

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
    fn base64_hash_file_value_is_decoded_inside_find_hash_in_textfile() {
        // The conversion applies to every textfile caller (fosshub /
        // sourceforge / metalink included), not just the `extract` mode. It
        // needs a custom regex: the default `[a-fA-F0-9]+` can only ever
        // capture hex, which the conversion leaves alone.
        let payload = base64::encode(&[0x0a; 32]);
        assert_eq!(
            extract(&format!("value={payload}"), "value=([A-Za-z0-9+/=]+)"),
            Some("0a".repeat(32)),
            "a base64 body decodes to hex before format_hash"
        );
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

    // ── auto-detection patterns ─────────────────────────────────────────────

    #[test]
    fn url_patterns_are_mutually_exclusive_on_real_shapes() {
        let gh = "https://github.com/o/r/releases/download/v1.0/app.zip";
        assert!(GITHUB_RELEASE_URL.is_match(gh));
        assert!(!SOURCEFORGE_URL.is_match(gh));
        assert!(!FOSSHUB_URL.is_match(gh));

        let sf = "https://downloads.sourceforge.net/project/foo/bar/app.zip";
        assert!(SOURCEFORGE_URL.is_match(sf));
        assert!(!GITHUB_RELEASE_URL.is_match(sf));

        let fs = "https://www.fosshub.com/Thing.html?dwl=thing.zip";
        let caps = FOSSHUB_URL.captures(fs).unwrap();
        assert_eq!(&caps["filename"], "thing.zip");
    }
}
