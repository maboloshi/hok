//! Query matching engine — query options and matchers.
//!
//! Split from `package/query.rs`: the matcher machinery (explicit vs regex,
//! bucket-prefix awareness, name prefiltering, description/binary matching)
//! is independent of the query walkers in `query.rs` and can evolve alone.

use regex::{Regex, RegexBuilder};

use crate::error::Fallible;
use crate::package::identity::split_bucket_query;
use crate::package::manifest::Manifest;

/// Options that may be used to query Scoop packages.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum QueryOption {
    /// Enable query through package binaries.
    Binary,

    /// Enable query through package description.
    Description,

    /// Explicit mode. Regex is disabled in this mode.
    ///
    /// Query will be performed through the package name only. `Description`
    /// and `Binary` options will be ignored.
    Explicit,

    /// Additionally check if the matched package is upgradable.
    ///
    /// This option only takes effect on querying installed packages.
    Upgradable,

    /// Check upgradable status without filtering out non-upgradable packages.
    ///
    /// Like `Upgradable` but does NOT exclude packages that are already
    /// at the latest version.
    UpgradableCheck,
}

/// A trait represents a matcher that can be used to do string matching.
pub(crate) trait Matcher {
    fn is_match(&self, s: &str) -> bool;
}

/// A matcher that does explicit match.
///
/// # Note
///
/// This matcher is case-insensitive.
struct ExplicitMatcher<'a>(&'a str);

/// A matcher that does regex match.
struct RegexMatcher(Regex);

impl Matcher for ExplicitMatcher<'_> {
    fn is_match(&self, s: &str) -> bool {
        self.0.eq_ignore_ascii_case(s)
    }
}

impl Matcher for RegexMatcher {
    fn is_match(&self, s: &str) -> bool {
        self.0.is_match(s)
    }
}

type QueryMatchers<'a> = Vec<(Option<String>, Box<dyn Matcher + Send + Sync + 'a>)>;

pub(crate) fn has_extra_query(options: &[QueryOption]) -> bool {
    options.contains(&QueryOption::Binary) || options.contains(&QueryOption::Description)
}

pub(crate) fn build_matchers<'a>(
    queries: &[&'a str],
    is_wildcard_query: bool,
    is_explicit_mode: bool,
) -> Fallible<QueryMatchers<'a>> {
    if is_wildcard_query {
        return Ok(vec![]);
    }

    let mut matchers: QueryMatchers<'a> = vec![];
    for query in queries {
        let (bucket_prefix, name) = split_bucket_query(query);

        if is_explicit_mode {
            matchers.push((bucket_prefix, Box::new(ExplicitMatcher(name))));
        } else {
            let re = RegexBuilder::new(name)
                .case_insensitive(true)
                .multi_line(true)
                .build()?;
            matchers.push((bucket_prefix, Box::new(RegexMatcher(re))));
        }
    }

    Ok(matchers)
}

pub(crate) fn name_prefiltered_out(
    name: &str,
    is_wildcard_query: bool,
    matchers: &QueryMatchers<'_>,
    options: &[QueryOption],
) -> bool {
    !is_wildcard_query
        && !has_extra_query(options)
        && !matchers.iter().any(|(_, matcher)| matcher.is_match(name))
}

pub(crate) fn manifest_matches(
    name: &str,
    bucket: &str,
    manifest: &Manifest,
    is_wildcard_query: bool,
    is_explicit_mode: bool,
    matchers: &QueryMatchers<'_>,
    options: &[QueryOption],
) -> bool {
    if is_wildcard_query {
        return true;
    }

    let prefixed_name_matched = matchers
        .iter()
        .filter(|(_, matcher)| matcher.is_match(name))
        .any(|(prefix, _)| {
            prefix.is_none() || prefix.as_deref().unwrap().eq_ignore_ascii_case(bucket)
        });

    if prefixed_name_matched {
        return true;
    }

    if is_explicit_mode {
        return false;
    }

    if options.contains(&QueryOption::Description)
        && matchers.iter().any(|(_, matcher)| {
            manifest
                .description()
                .map(|description| matcher.is_match(description))
                .unwrap_or(false)
        })
    {
        return true;
    }

    if options.contains(&QueryOption::Binary) {
        let binaries = manifest.shims().unwrap_or_default();
        if matchers
            .iter()
            .any(|(_, matcher)| binaries.iter().any(|binary| matcher.is_match(binary)))
        {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── build_matchers ───────────────────────────────────────────────────────

    #[test]
    fn wildcard_query_yields_empty_matchers() {
        let matchers = build_matchers(&["*"], true, false).unwrap();
        assert!(matchers.is_empty());
    }

    #[test]
    fn explicit_mode_builds_explicit_matcher() {
        let matchers = build_matchers(&["curl"], false, true).unwrap();
        assert_eq!(matchers.len(), 1);
    }

    #[test]
    fn regex_mode_builds_regex_matcher() {
        let matchers = build_matchers(&["curl"], false, false).unwrap();
        assert_eq!(matchers.len(), 1);
    }

    #[test]
    fn bucket_prefixed_query_is_parsed() {
        let matchers = build_matchers(&["extras/curl"], false, true).unwrap();
        assert_eq!(matchers.len(), 1);
        let (prefix, _) = &matchers[0];
        assert_eq!(prefix.as_deref(), Some("extras"));
    }

    #[test]
    fn backslash_bucket_prefixed_query_is_parsed() {
        let matchers = build_matchers(&["extras\\curl"], false, true).unwrap();
        assert_eq!(matchers.len(), 1);
        let (prefix, _) = &matchers[0];
        assert_eq!(prefix.as_deref(), Some("extras"));
    }

    #[test]
    fn leading_backslash_is_not_a_separator() {
        // A regex escape like `\d` must not be split into a bucket prefix.
        let matchers = build_matchers(&["\\d"], false, false).unwrap();
        assert_eq!(matchers.len(), 1);
        let (prefix, _) = &matchers[0];
        assert!(prefix.is_none(), "leading backslash must not split bucket");
    }

    // ── name_prefiltered_out ─────────────────────────────────────────────────

    #[test]
    fn wildcard_never_prefilters() {
        let matchers = build_matchers(&[], true, false).unwrap();
        assert!(!name_prefiltered_out("anything", true, &matchers, &[]));
    }

    #[test]
    fn explicit_match_is_not_prefiltered() {
        let matchers = build_matchers(&["curl"], false, true).unwrap();
        assert!(!name_prefiltered_out("curl", false, &matchers, &[]));
    }

    #[test]
    fn non_match_is_prefiltered_when_no_extra_options() {
        let matchers = build_matchers(&["curl"], false, true).unwrap();
        assert!(name_prefiltered_out("wget", false, &matchers, &[]));
    }

    #[test]
    fn description_option_prevents_prefilter() {
        let matchers = build_matchers(&["downloader"], false, false).unwrap();
        // With description option, name prefilter should NOT apply
        let options = vec![QueryOption::Description];
        assert!(!name_prefiltered_out("curl", false, &matchers, &options));
    }

    // ── manifest_matches ─────────────────────────────────────────────────────

    fn make_manifest(version: &str) -> Manifest {
        let json = format!(
            r#"{{"version":"{version}","homepage":"https://example.com","license":"MIT"}}"#
        );
        Manifest::from_json("test", &json).unwrap()
    }

    #[test]
    fn wildcard_matches_all() {
        let m = make_manifest("1.0");
        let matchers = build_matchers(&[], true, false).unwrap();
        assert!(manifest_matches(
            "any-pkg",
            "bucket",
            &m,
            true,
            false,
            &matchers,
            &[]
        ));
    }

    #[test]
    fn explicit_exact_match() {
        let m = make_manifest("1.0");
        let matchers = build_matchers(&["curl"], false, true).unwrap();
        assert!(manifest_matches(
            "curl",
            "main",
            &m,
            false,
            true,
            &matchers,
            &[]
        ));
    }

    #[test]
    fn explicit_no_match() {
        let m = make_manifest("1.0");
        let matchers = build_matchers(&["curl"], false, true).unwrap();
        assert!(!manifest_matches(
            "wget",
            "main",
            &m,
            false,
            true,
            &matchers,
            &[]
        ));
    }

    #[test]
    fn regex_case_insensitive_match() {
        let m = make_manifest("1.0");
        let matchers = build_matchers(&["CURL"], false, false).unwrap();
        assert!(manifest_matches(
            "curl",
            "main",
            &m,
            false,
            false,
            &matchers,
            &[]
        ));
    }

    #[test]
    fn bucket_prefixed_query_only_matches_correct_bucket() {
        let m = make_manifest("1.0");
        let matchers = build_matchers(&["extras/curl"], false, true).unwrap();
        // Matches when bucket is "extras"
        assert!(manifest_matches(
            "curl",
            "extras",
            &m,
            false,
            true,
            &matchers,
            &[]
        ));
        // Does NOT match when bucket is "main"
        assert!(!manifest_matches(
            "curl",
            "main",
            &m,
            false,
            true,
            &matchers,
            &[]
        ));
    }
}
