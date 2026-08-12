//! Manifest source resolution for install queries.
//!
//! Mirrors upstream Scoop's `Get-Manifest` (lib/manifest.ps1) and
//! `generate_user_manifest` (lib/manifest.ps1): given an install query
//! (a bare name, `bucket/name`, a manifest URL, or a local manifest path),
//! resolve it to a concrete [`Manifest`], and — for `app@version` queries —
//! generate a manifest for the requested version via autoupdate.
//!
//! The resolved result carries the install name (`appname_from_url` for
//! URL/path installs), the source bucket (if any), and the source URL/path
//! that upstream stores in `install.json` as `url`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::bucket::Bucket;
use crate::constant::ISOLATED_PACKAGE_BUCKET;
use crate::error::Fallible;
use crate::internal;
use crate::package::manifest::Manifest;
use crate::package::{checkver, identity, Package};
use crate::{Error, Session};

/// The result of resolving an install query to a concrete manifest.
#[derive(Debug)]
pub(crate) struct ResolvedManifest {
    /// The app name the package will be installed under.
    pub name: String,
    /// The parsed manifest.
    pub manifest: Manifest,
}

/// Resolve a manifest from a URL, a local path, or a bucket, mirroring
/// upstream `Get-Manifest` (lib/manifest.ps1): URL/UNC first, then an
/// existing local path, then bucket lookup.
///
/// # Errors
///
/// Returns an error when the URL cannot be fetched, the local file cannot be
/// read, or no bucket contains the requested manifest.
pub(crate) fn resolve_manifest(
    session: &Session,
    app: &str,
    bucket: Option<&str>,
) -> Fallible<ResolvedManifest> {
    // 1. URL or UNC path — fetch and parse in memory.
    if identity::is_manifest_url(app) {
        let text = crate::network::download_page(session, app, 120, None)?;
        let name = identity::appname_from_url(app);
        // The URL doubles as the virtual manifest path, so isolated
        // installs record it in `install.json` as the source `url`.
        let manifest = Manifest::parse_str(app, &text)?;
        return Ok(ResolvedManifest { name, manifest });
    }

    // 2. Local path — read the file directly.
    let path = Path::new(app);
    if path.exists() {
        let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let abs = internal::path::normalize_path(&abs);
        let text = std::fs::read_to_string(&abs)?;
        let name = identity::appname_from_url(app);
        // The absolute path doubles as the virtual manifest path, so
        // isolated installs record it in `install.json` as the source `url`.
        let manifest = Manifest::parse_str(&abs.display().to_string(), &text)?;
        return Ok(ResolvedManifest { name, manifest });
    }

    // 3. Bucket lookup — scan the requested (or all added) buckets.
    let (_bucket_name, manifest_path) = find_bucket_manifest(session, app, bucket)?;
    let manifest = Manifest::parse(&manifest_path)?;
    Ok(ResolvedManifest {
        name: app.to_owned(),
        manifest,
    })
}

/// Resolve an isolated install query (`name@version`, a manifest URL, or a
/// local manifest path) to a [`Package`] under the isolated bucket.
///
/// Returns `Ok(None)` when the query is a plain bucket reference that must
/// go through bucket scanning — mirroring the install pipeline's
/// isolated/regular split. Shared by the install and download pipelines so
/// the dispatch cannot drift.
pub(crate) fn resolve_isolated_query(session: &Session, query: &str) -> Fallible<Option<Package>> {
    let Some(aq) = identity::parse_app(query) else {
        return Ok(None);
    };

    // `name@version` — generate (or reuse) a manifest for the version.
    if let Some(version) = aq.version.as_deref() {
        let resolved = generate_user_manifest(session, &aq.app, aq.bucket.as_deref(), version)?;
        return Ok(Some(isolated_package(resolved)));
    }

    // URL / local-path manifest — resolve in isolation.
    let is_local = Path::new(&aq.app).exists();
    if identity::is_manifest_url(&aq.app) || is_local {
        let resolved = resolve_manifest(session, &aq.app, None)?;
        return Ok(Some(isolated_package(resolved)));
    }

    Ok(None)
}

/// Wrap a resolved manifest as a package under the isolated bucket.
fn isolated_package(resolved: ResolvedManifest) -> Package {
    Package::from(&resolved.name, ISOLATED_PACKAGE_BUCKET, resolved.manifest)
}

/// Generate (or reuse) a manifest for the given `app@version`, mirroring
/// upstream `generate_user_manifest` (lib/manifest.ps1).
///
/// When the resolved manifest's version already matches, it is returned
/// as-is. Otherwise the manifest must carry an `autoupdate` section; the
/// version is substituted via [`checkver::apply_autoupdate`] with empty
/// capture groups (matching upstream's `Invoke-AutoUpdate $app $path
/// $manifest $version $(@{ })`) and the result is written to the workspace
/// directory (`<root>/workspace`, upstream `usermanifestsdir`).
///
/// # Errors
///
/// Returns an error when the manifest has no `autoupdate` capability, or
/// when autoupdate fails (bad URL, hash download failure, etc.).
pub(crate) fn generate_user_manifest(
    session: &Session,
    app: &str,
    bucket: Option<&str>,
    version: &str,
) -> Fallible<ResolvedManifest> {
    let resolved = resolve_manifest(session, app, bucket)?;

    if resolved.manifest.version() == version {
        return Ok(resolved);
    }

    session.output().warn(format!(
        "Given version ({version}) does not match manifest ({})",
        resolved.manifest.version()
    ));
    session.output().warn(format!(
        "Attempting to generate manifest for '{}' ({version})",
        resolved.name
    ));

    if resolved.manifest.autoupdate().is_none() {
        return Err(Error::Custom(format!(
            "'{}' does not have autoupdate capability\ncouldn't find manifest for '{}@{version}'",
            resolved.name, app
        )));
    }

    let workspace = session.workspace_dir();
    internal::fs::ensure_dir(&workspace)?;
    let out_path = workspace.join(format!("{}.json", resolved.name));

    // Base manifest text: the raw string for URL/path installs, otherwise
    // read from the bucket file (raw is only kept for string-loaded ones).
    let base_text = match resolved.manifest.raw() {
        Some(text) => text.to_owned(),
        None => std::fs::read_to_string(resolved.manifest.path())?,
    };
    std::fs::write(&out_path, base_text)?;

    checkver::apply_autoupdate(
        session,
        &out_path,
        &resolved.manifest,
        version,
        &[],
        &HashMap::new(),
    )?;

    let generated = std::fs::read_to_string(&out_path)?;
    // The workspace file doubles as the virtual manifest path, so the
    // generated install records it in `install.json` as the source `url`
    // (upstream stores the generated manifest's local path there).
    let manifest = Manifest::parse_str(&out_path.display().to_string(), &generated)?;

    Ok(ResolvedManifest {
        name: resolved.name,
        manifest,
    })
}

/// Locate the manifest file of `app` in the given bucket (or the first
/// added bucket that contains it), returning `(bucket_name, path)`.
///
/// Upstream warns when multiple buckets contain the app and picks the first
/// one (Get-Manifest, lib/manifest.ps1); we keep the same behavior.
fn find_bucket_manifest(
    session: &Session,
    app: &str,
    bucket: Option<&str>,
) -> Fallible<(String, PathBuf)> {
    let match_manifest = |de: &std::fs::DirEntry| -> bool {
        de.file_name()
            .to_str()
            .and_then(|n| n.strip_suffix(".json"))
            .map(|n| n.eq_ignore_ascii_case(app))
            .unwrap_or(false)
    };

    let mut candidates: Vec<(String, PathBuf)> = if let Some(bucket) = bucket {
        let bucket = Bucket::from(&session.bucket_dir(bucket))
            .map_err(|_| Error::PackageNotFound(app.to_owned()))?;
        bucket
            .manifests()?
            .into_iter()
            .filter(|de| match_manifest(de))
            .map(|de| (bucket.name().to_owned(), de.path()))
            .collect()
    } else {
        crate::bucket::bucket_added(session)?
            .into_iter()
            .flat_map(|b| {
                b.manifests()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|de| match_manifest(de))
                    .map(move |de| (b.name().to_owned(), de.path()))
                    .collect::<Vec<_>>()
            })
            .collect()
    };

    let (bucket, path) = candidates
        .drain(..)
        .next()
        .ok_or_else(|| Error::PackageNotFound(app.to_owned()))?;
    Ok((bucket, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils;

    fn setup(name: &str) -> (Session, std::path::PathBuf) {
        let root = test_utils::tmpdir(name);
        let session = test_utils::test_session(&root);
        (session, root)
    }

    #[test]
    fn resolve_manifest_from_local_path() {
        let (session, root) = setup("resolve-local-path");
        let manifest_path = root.join("app.json");
        std::fs::write(
            &manifest_path,
            r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
        )
        .unwrap();

        let resolved = resolve_manifest(&session, manifest_path.to_str().unwrap(), None).unwrap();
        assert_eq!(resolved.name, "app");
        assert_eq!(resolved.manifest.version(), "1.0.0");
        // The virtual manifest path records the source for install.json.
        assert!(resolved.manifest.path().is_absolute());
    }

    #[test]
    fn resolve_manifest_missing_app_errors() {
        let (session, _root) = setup("resolve-missing");
        let err = resolve_manifest(&session, "no-such-app-xyz", None).unwrap_err();
        assert!(err.to_string().contains("no-such-app-xyz"));
    }

    #[test]
    fn generate_user_manifest_matching_version_returns_as_is() {
        let (session, root) = setup("gen-match-version");
        let manifest_path = root.join("app.json");
        std::fs::write(
            &manifest_path,
            r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
        )
        .unwrap();

        let resolved =
            generate_user_manifest(&session, manifest_path.to_str().unwrap(), None, "1.0.0")
                .unwrap();
        assert_eq!(resolved.manifest.version(), "1.0.0");
        // Isolated source is preserved (URL/path install stays isolated).
    }

    #[test]
    fn generate_user_manifest_without_autoupdate_errors() {
        let (session, root) = setup("gen-no-autoupdate");
        let manifest_path = root.join("app.json");
        std::fs::write(
            &manifest_path,
            r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
        )
        .unwrap();

        let err = generate_user_manifest(&session, manifest_path.to_str().unwrap(), None, "2.0.0")
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("does not have autoupdate capability"),
            "{}",
            msg
        );
        assert!(msg.contains("@2.0.0"), "{}", msg);
    }

    #[test]
    fn generate_user_manifest_applies_version_substitution() {
        let (session, root) = setup("gen-autoupdate");
        let manifest_path = root.join("app.json");
        std::fs::write(
            &manifest_path,
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "url": "https://example.com/download/app-1.0.0.zip",
                "autoupdate": {
                    "url": "https://example.com/download/app-$version.zip"
                }
            }"#,
        )
        .unwrap();

        // Version 1.0.0 matches — no autoupdate run, no network.
        let resolved =
            generate_user_manifest(&session, manifest_path.to_str().unwrap(), None, "1.0.0")
                .unwrap();
        assert_eq!(resolved.manifest.version(), "1.0.0");

        // A non-matching version without an autoupdate `url`/`hash` still
        // substitutes the version field locally (no downloads triggered).
        let no_url = root.join("nourl.json");
        std::fs::write(
            &no_url,
            r#"{
                "version": "0.1.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "url": "https://example.com/download/x.zip",
                "autoupdate": { "extract_dir": "dir-$version" }
            }"#,
        )
        .unwrap();
        let resolved =
            generate_user_manifest(&session, no_url.to_str().unwrap(), None, "2.0.0").unwrap();
        assert_eq!(resolved.manifest.version(), "2.0.0");
        assert!(session.workspace_dir().join("nourl.json").exists());
    }
}
