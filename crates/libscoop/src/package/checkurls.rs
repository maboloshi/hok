use std::collections::HashMap;
use std::path::Path;

use crate::internal::url::strip_url_fragment;
use crate::package::manifest_walker;
use crate::{error::Fallible, network, Manifest, Session};

#[derive(Debug, Clone)]
pub struct UrlCheckError {
    pub url: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ManifestUrlCheck {
    pub name: String,
    pub total_urls: u32,
    pub ok_count: u32,
    pub failed_count: u32,
    pub errors: Vec<UrlCheckError>,
}

#[derive(Debug, Clone, Default)]
pub struct CheckUrlsReport {
    pub total_manifests: u32,
    pub total_urls: u32,
    pub total_valid: u32,
    pub total_invalid: u32,
    pub results: Vec<ManifestUrlCheck>,
}

pub fn check_urls(
    session: &Session,
    dir: &Path,
    app_filters: &[String],
    timeout_secs: u64,
    skip_valid: bool,
    on_result: impl FnMut(&ManifestUrlCheck),
) -> Fallible<CheckUrlsReport> {
    let mut report = CheckUrlsReport::default();
    let mut on_result = on_result;

    for (path, name) in manifest_walker::discover_matching(dir, app_filters)? {
        let manifest = match Manifest::parse(path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let raw_urls: Vec<String> = if manifest.url().is_empty() {
            manifest
                .all_urls()
                .into_iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            manifest.url().into_iter().map(|s| s.to_string()).collect()
        };

        let urls: Vec<String> = raw_urls
            .iter()
            .map(|u| strip_url_fragment(u).to_string())
            .collect();

        if urls.is_empty() {
            continue;
        }

        report.total_manifests += 1;

        let manifest_cookies: Option<HashMap<String, String>> = manifest.cookie().cloned();
        let mut ok_count = 0u32;
        let mut failed_count = 0u32;
        let mut errors = Vec::new();

        for url in &urls {
            report.total_urls += 1;
            let result = network::head_url(session, url, timeout_secs, manifest_cookies.as_ref());

            match result.error {
                None => {
                    ok_count += 1;
                    report.total_valid += 1;
                }
                Some(msg) => {
                    failed_count += 1;
                    report.total_invalid += 1;
                    errors.push(UrlCheckError {
                        url: url.clone(),
                        message: msg,
                    });
                }
            }
        }

        if ok_count == urls.len() as u32 && skip_valid {
            continue;
        }

        let check = ManifestUrlCheck {
            name: name.clone(),
            total_urls: urls.len() as u32,
            ok_count,
            failed_count,
            errors,
        };
        on_result(&check);
        report.results.push(check);
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── CheckUrlsReport default ─────────────────────────────────────────────

    #[test]
    fn report_default_is_zeroed() {
        let r = CheckUrlsReport::default();
        assert_eq!(r.total_manifests, 0);
        assert_eq!(r.total_urls, 0);
        assert_eq!(r.total_valid, 0);
        assert_eq!(r.total_invalid, 0);
        assert!(r.results.is_empty());
    }
}
