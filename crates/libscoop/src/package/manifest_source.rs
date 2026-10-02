//! Manifest source resolution — hok's `Get-Manifest`.
//!
//! Mirrors upstream Scoop's `Get-Manifest` (`lib/manifest.ps1:34-112`) and
//! `generate_user_manifest` (`lib/manifest.ps1:175-207`): given a query (a bare
//! name, `bucket/name`, `name@version`, a manifest URL/UNC, or a local manifest
//! path), resolve it to a concrete [`Manifest`], and — for `app@version`
//! queries — generate a manifest for the requested version via autoupdate.
//!
//! This is the **single entry point** for "which manifest does this query refer
//! to?": `cat`, `home` and `info` resolve through it, and the install/download
//! pipelines use it for URL / local-path / `@version` queries, so the source
//! resolution cannot drift into one implementation per command.
//!
//! The resolution order is upstream's, exactly:
//!
//! 1. URL / UNC (`manifest.ps1:38-41`) — fetch (HTTP/FTP) or read (UNC).
//! 2. Installed (`:43-75`) — only reached when a version resolves in either
//!    scope. The origin comes from the install record (`scoop-install.json`,
//!    else `install.json`): its `bucket` names the
//!    bucket to read the manifest from (with a `<bucket>\deprecated` fallback),
//!    otherwise its `url` (remote or local), otherwise the installed
//!    manifest (`scoop-manifest.json`, else `manifest.json`).
//! 3. Not installed (`:76-104`) — `parse_app` splits `bucket/name@version`; an
//!    explicit bucket reads that bucket only, otherwise every added bucket is
//!    scanned in order. A query that is an existing local file is read from
//!    disk.
//!
//! The resolved result carries the install/source name
//! (`appname_from_url` for URL/path queries), the source bucket (if any), and
//! the source URL/path that upstream stores in the install record as `url`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::bucket::Bucket;
use crate::constant::ISOLATED_PACKAGE_BUCKET;
use crate::error::Fallible;
use crate::internal;
use crate::package::manifest::{Manifest, parse_file};
use crate::package::{Package, checkver, identity, install_state, source};
use crate::{Error, Session};

/// The result of resolving a query to a concrete manifest — upstream
/// `Get-Manifest`'s `($app, $manifest, $bucket, $url)` return tuple.
#[derive(Debug)]
pub struct GetManifest {
    /// The app name the query resolves to (`appname_from_url` for URL/path
    /// queries), i.e. upstream's `$app`.
    pub name: String,
    /// The parsed manifest.
    pub manifest: Manifest,
    /// The bucket the manifest came from, when it came from one.
    pub bucket: Option<String>,
    /// The source URL/path — upstream's `$url`. Set for URL/UNC queries, for
    /// installs recorded with a `url`, and for local-file queries.
    pub url: Option<String>,
    /// Every bucket that holds a manifest for the name, sorted by bucket name
    /// (see [`bucket_manifests`]).
    ///
    /// hok extension: upstream picks the first bucket and only warns
    /// (`manifest.ps1:107-109`); hok's `cat`/`home` keep their interactive
    /// picker, which needs the full candidate set. It is only populated by the
    /// not-installed bare-name branch, so it is empty whenever the source was
    /// unambiguous. The picker re-resolves with [`get_manifest`]'s `bucket`
    /// argument.
    pub candidates: Vec<String>,
}

/// Whether `s` is a UNC path (`\\server\share\...`).
///
/// Upstream treats UNC as a URL (`manifest.ps1:38`) and hands it to
/// `WebClient.DownloadData`, which cannot open a UNC path; hok reads the file
/// instead — see [`get_manifest`].
fn is_unc(s: &str) -> bool {
    s.starts_with("\\\\")
}

/// The last path segment of a query: upstream's `installed` strips a
/// `bucket/name` prefix before probing the apps directory
/// (`core.ps1:393`).
fn last_segment(query: &str) -> String {
    query
        .split(['/', '\\'])
        .next_back()
        .unwrap_or(query)
        .to_owned()
}

/// The apps directory of one scope (`<root>/apps` or `<global>/apps`).
///
/// Deliberately scope-explicit: `Get-Manifest` inspects *both* the local and
/// the global install without switching the session's global flag, so the
/// lookup must not follow [`Session::effective_root_path`].
fn apps_dir_in(session: &Session, global: bool) -> PathBuf {
    let root = if global {
        session.config().global_path().to_path_buf()
    } else {
        session.config().root_path().to_path_buf()
    };
    root.join("apps")
}

/// The app directory of one scope: `<root>/apps/<name>`.
fn app_dir_in(session: &Session, name: &str, global: bool) -> PathBuf {
    apps_dir_in(session, global).join(name)
}

/// The resolved current version of `name` in one scope (upstream
/// `Select-CurrentVersion`); `None` when the app is not installed there.
fn installed_version(session: &Session, name: &str, global: bool) -> Option<String> {
    install_state::resolve_current_version(
        &app_dir_in(session, name, global),
        session.config().no_junction(),
    )
    .map(|resolved| resolved.version)
}

/// The manifest path of `name` inside `bucket`, if the bucket has one.
fn bucket_manifest_path(session: &Session, bucket: &str, name: &str) -> Option<PathBuf> {
    Bucket::from(&session.bucket_dir(bucket))
        .ok()?
        .path_of_manifest(name)
}

/// The manifest path of `name` under `bucket`'s `deprecated/` folder.
fn deprecated_manifest_path(session: &Session, bucket: &str, name: &str) -> Option<PathBuf> {
    source::deprecated_manifest_path(session, bucket, name)
}

/// Resolve a manifest from a URL, a local path, an installed record, or a
/// bucket — upstream `Get-Manifest` (`lib/manifest.ps1:34-112`).
///
/// `bucket` is a hok extension used by the interactive multi-bucket picker
/// (`cat`/`home`): when set, the not-installed branch reads only that bucket.
/// Upstream has no such parameter because it silently picks the first bucket.
///
/// # Errors
///
/// Returns [`Error::PackageNotFound`] when the query resolves to no manifest,
/// and propagates download/parse failures for URL and local-file sources.
pub async fn get_manifest(
    session: &Session,
    app: &str,
    bucket: Option<&str>,
) -> Fallible<GetManifest> {
    let query = app.trim_start_matches('/');

    // 1. URL or UNC path (upstream manifest.ps1:37-41).
    if identity::is_manifest_url(query) {
        let url = query.to_owned();
        let name = identity::appname_from_url(&url);
        // Upstream hands both to `url_manifest` (a WebClient download); a UNC
        // path is a local file, so read it from disk where upstream's download
        // would fail outright. Documented deviation, strictly a superset.
        let manifest = if is_unc(&url) {
            parse_file(Path::new(&url))?
        } else {
            let text = crate::network::download_page(session, &url, 120).await?;
            Manifest::parse_str(&url, &text)?
        };
        return Ok(GetManifest {
            name,
            manifest,
            bucket: None,
            url: Some(url),
            candidates: Vec::new(),
        });
    }

    // 2. Already installed (upstream manifest.ps1:43-75).
    if let Some(resolved) = installed_source(session, query).await? {
        return Ok(resolved);
    }

    // 3. Not installed (upstream manifest.ps1:76-104).
    not_installed(session, query, bucket)
}

/// The installed branch of [`get_manifest`] (upstream `manifest.ps1:43-75`).
///
/// Returns `Ok(None)` **only** when the app is not installed in either scope.
/// Once an install exists, upstream never falls through to the bucket lookup —
/// it returns a null manifest and the caller aborts — so a failed lookup here
/// is an error, not a fallthrough.
async fn installed_source(session: &Session, query: &str) -> Fallible<Option<GetManifest>> {
    let install_name = last_segment(query);
    let local = installed_version(session, &install_name, false);
    let global = installed_version(session, &install_name, true);
    if local.is_none() && global.is_none() {
        return Ok(None);
    }

    // Upstream: `$global = installed $app $true`, then the version is selected
    // in that scope. When it does not resolve (a `bucket/name` query that the
    // first probe could not split), upstream re-parses the app and retries.
    let is_global = global.is_some();
    let mut name = install_name;
    let mut version = if is_global { global } else { local };
    if version.is_none()
        && let Some(aq) = identity::parse_app(query)
    {
        name = aq.app;
        version = installed_version(session, &name, is_global);
    }
    let Some(version) = version else {
        return Err(Error::PackageNotFound(query.to_owned()));
    };

    let version_dir = app_dir_in(session, &name, is_global).join(version);
    // The record is `scoop-install.json`, else the legacy `install.json`
    // (upstream `install_info`).
    let Some(install_json) = install_state::install_info_path(&version_dir) else {
        // Upstream leaves `$manifest` null when there is no install record.
        return Err(Error::PackageNotFound(query.to_owned()));
    };
    // Upstream's `parse_json` warns and yields `$null` for a broken
    // install record, and the lookup then misses the bucket record. Tolerated
    // here the same way (the strict read stays the install/query path's job).
    let info = install_state::parse_install_info(&install_json).ok();

    let mut bucket = None;
    let mut url = None;
    let manifest = match info.as_ref().and_then(|info| info.bucket()) {
        Some(origin) => {
            // The recorded bucket names the source; when its live manifest is
            // gone, upstream falls back to the bucket's `deprecated/` folder.
            bucket = Some(origin.to_owned());
            bucket_manifest_path(session, origin, &name)
                .and_then(|path| parse_file(&path).ok())
                .or_else(|| {
                    deprecated_manifest_path(session, origin, &name)
                        .and_then(|path| parse_file(&path).ok())
                })
        }
        None => {
            // No bucket: the install recorded a source `url` (URL installs) or
            // nothing at all (a broken install).
            url = info.as_ref().and_then(|info| info.url()).map(str::to_owned);
            let mut manifest = None;
            if let Some(source_url) = url.as_deref()
                && identity::is_manifest_url(source_url)
                && !is_unc(source_url)
            {
                // Upstream warns and continues when the download fails; an
                // unreachable recorded URL must not abort the lookup.
                if let Ok(text) = crate::network::download_page(session, source_url, 120).await {
                    manifest = Manifest::parse_str(source_url, &text).ok();
                }
            }
            if manifest.is_none()
                && let Some(source_url) = url.as_deref()
                && Path::new(source_url).exists()
            {
                manifest = parse_file(source_url).ok();
            }
            manifest.or_else(|| {
                install_state::installed_manifest_path(&version_dir)
                    .and_then(|path| parse_file(path).ok())
            })
        }
    };

    match manifest {
        Some(manifest) => Ok(Some(GetManifest {
            name,
            manifest,
            bucket,
            url,
            candidates: Vec::new(),
        })),
        None => Err(Error::PackageNotFound(query.to_owned())),
    }
}

/// The not-installed branch of [`get_manifest`] (upstream `manifest.ps1:76-104`).
fn not_installed(
    session: &Session,
    query: &str,
    bucket_override: Option<&str>,
) -> Fallible<GetManifest> {
    // Upstream `parse_app`: `bucket/name@version`, a bare name, or a `*.json`
    // URL/path. An unparseable query is used verbatim (upstream's `else`).
    // The app part is used as-is — unlike the installed branch, upstream does
    // not strip a path down to its last segment here.
    let parsed = identity::parse_app(query);
    let mut name = parsed
        .as_ref()
        .map(|aq| aq.app.clone())
        .unwrap_or_else(|| query.to_owned());
    let explicit_bucket = bucket_override
        .map(str::to_owned)
        .or_else(|| parsed.as_ref().and_then(|aq| aq.bucket.clone()));

    let mut bucket = None;
    let mut candidates = Vec::new();
    let mut manifest = None;
    let mut url = None;

    match explicit_bucket {
        Some(requested) => {
            // An explicit bucket is authoritative: upstream reads only it
            // (`manifest $app $bucket`), even when another bucket also has it.
            if let Some(path) = bucket_manifest_path(session, &requested, &name) {
                manifest = Some(parse_file(&path)?);
            }
            bucket = Some(requested);
        }
        None => {
            let matches = bucket_manifests(session, &name)?;
            if let Some((found_bucket, path)) = matches.first() {
                manifest = Some(parse_file(path)?);
                bucket = Some(found_bucket.clone());
            }
            candidates = matches.into_iter().map(|(bucket, _)| bucket).collect();
        }
    }

    if manifest.is_none() {
        // Upstream: an existing local file wins; otherwise a query that looks
        // like a path (or ends in `.json`) is recorded as the source `url`.
        if Path::new(&name).exists() {
            let path = Path::new(&name);
            let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            let abs = internal::path::normalize_path(&abs).display().to_string();
            name = identity::appname_from_url(&abs);
            manifest = Some(parse_file(&abs)?);
            url = Some(abs);
        } else {
            if name.contains(['\\', '/']) || name.ends_with(".json") {
                url = Some(name.clone());
            }
            name = identity::appname_from_url(&name);
        }
    }

    match manifest {
        Some(manifest) => Ok(GetManifest {
            name,
            manifest,
            bucket,
            url,
            candidates,
        }),
        None => Err(Error::PackageNotFound(query.to_owned())),
    }
}

/// Every added bucket that holds a manifest for `app` — upstream
/// `Get-Manifest`'s `foreach ($tekcub in Get-LocalBucket)`
/// (`manifest.ps1:81-92`).
///
/// The lookup is the same bounded `exists()` probe upstream's `manifest_path`
/// performs (`lib/manifest.ps1:1-3`), so a bucket that cannot be read simply
/// contributes no candidate instead of failing the whole resolution.
///
/// Upstream's "bucket order" is `Get-LocalBucket`'s directory enumeration
/// order; hok's [`crate::bucket::bucket_added`] is parallel and therefore
/// unordered, so matches are sorted by bucket name. Deterministic beats
/// reproducing an ordering hok never had.
fn bucket_manifests(session: &Session, app: &str) -> Fallible<Vec<(String, PathBuf)>> {
    let mut candidates: Vec<(String, PathBuf)> = Vec::new();
    for bucket in crate::bucket::bucket_added(session)? {
        if let Some(path) = bucket.path_of_manifest(app) {
            candidates.push((bucket.name().to_owned(), path));
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(candidates)
}

/// Resolve an isolated install query (`name@version`, a manifest URL, or a
/// local manifest path) to a [`Package`] under the isolated bucket.
///
/// Returns `Ok(None)` when the query is a plain bucket reference that must
/// go through bucket scanning — mirroring the install pipeline's
/// isolated/regular split. Shared by the install and download pipelines so
/// the dispatch cannot drift.
pub(crate) async fn resolve_isolated_query(
    session: &Session,
    query: &str,
) -> Fallible<Option<Package>> {
    let Some(aq) = identity::parse_app(query) else {
        return Ok(None);
    };

    // `name@version` — generate (or reuse) a manifest for the version.
    if let Some(version) = aq.version.as_deref() {
        let resolved =
            generate_user_manifest(session, &aq.app, aq.bucket.as_deref(), version).await?;
        return Ok(Some(isolated_package(resolved)));
    }

    // URL / local-path manifest — resolve in isolation.
    let is_local = Path::new(&aq.app).exists();
    if identity::is_manifest_url(&aq.app) || is_local {
        let resolved = get_manifest(session, &aq.app, None).await?;
        return Ok(Some(isolated_package(resolved)));
    }

    Ok(None)
}

/// Wrap a resolved manifest as a package under the isolated bucket.
fn isolated_package(resolved: GetManifest) -> Package {
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
pub(crate) async fn generate_user_manifest(
    session: &Session,
    app: &str,
    bucket: Option<&str>,
    version: &str,
) -> Fallible<GetManifest> {
    let resolved = get_manifest(session, app, bucket).await?;

    if resolved.manifest.version() == version {
        return Ok(resolved);
    }

    session.output().warn(hok_i18n::t!(
        "cmd.version_mismatch",
        given = version,
        manifest = resolved.manifest.version()
    ));
    session.output().warn(hok_i18n::t!(
        "cmd.generating_manifest",
        name = resolved.name,
        version = version
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
    )
    .await?;

    let generated = std::fs::read_to_string(&out_path)?;
    // The workspace file doubles as the virtual manifest path, so the
    // generated install records it in the install record as the source `url`
    // (upstream stores the generated manifest's local path there).
    let manifest = Manifest::parse_str(&out_path.display().to_string(), &generated)?;

    Ok(GetManifest {
        name: resolved.name,
        manifest,
        bucket: resolved.bucket,
        url: resolved.url,
        candidates: Vec::new(),
    })
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

    /// Install `name@version` at the root with the given install.json body,
    /// including the `current` metadata `Select-CurrentVersion` reads.
    fn install(root: &Path, name: &str, version: &str, install_json: &str) {
        let dir = root.join("apps").join(name).join(version);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = format!(r#"{{"version":"{version}","description":"installed {name}"}}"#);
        std::fs::write(dir.join("manifest.json"), &manifest).unwrap();
        std::fs::write(dir.join("install.json"), install_json).unwrap();
        let current = root.join("apps").join(name).join("current");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("manifest.json"), &manifest).unwrap();
        std::fs::write(current.join("install.json"), install_json).unwrap();
    }

    /// Add a bucket `bucket` holding `name.json` with `version`.
    fn add_bucket(root: &Path, bucket: &str, name: &str, version: &str) {
        let dir = root.join("buckets").join(bucket).join("bucket");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{name}.json")),
            format!(r#"{{"version":"{version}","description":"{bucket} {name}"}}"#),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn resolves_from_local_path() {
        let (session, root) = setup("get-manifest-local");
        let manifest_path = root.join("app.json");
        std::fs::write(
            &manifest_path,
            r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
        )
        .unwrap();

        let resolved = get_manifest(&session, manifest_path.to_str().unwrap(), None)
            .await
            .unwrap();
        assert_eq!(resolved.name, "app");
        assert_eq!(resolved.manifest.version(), "1.0.0");
        assert!(resolved.manifest.path().is_absolute());
        assert!(resolved.bucket.is_none());
        // The source path is recorded (upstream's `$url`), normalized exactly
        // like the resolver does (`canonicalize` + `normalize_path`).
        let expected =
            internal::path::normalize_path(std::fs::canonicalize(&manifest_path).unwrap())
                .display()
                .to_string();
        assert_eq!(resolved.url.as_deref(), Some(expected.as_str()));
    }

    #[tokio::test]
    async fn missing_app_errors() {
        let (session, _root) = setup("get-manifest-missing");
        let err = get_manifest(&session, "no-such-app-xyz", None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no-such-app-xyz"));
    }

    /// A bare name resolves from the bucket and reports its bucket; every
    /// matching bucket is listed for the interactive picker, sorted so the
    /// result does not depend on the parallel bucket scan.
    #[tokio::test]
    async fn bare_name_reports_bucket_and_candidates() {
        let (session, root) = setup("get-manifest-buckets");
        add_bucket(&root, "main", "demo", "1.0.0");
        add_bucket(&root, "extra", "demo", "9.9.9");

        let resolved = get_manifest(&session, "demo", None).await.unwrap();
        assert_eq!(resolved.name, "demo");
        assert_eq!(resolved.candidates, vec!["extra", "main"]);
        // The first candidate is the one whose manifest is returned.
        assert_eq!(resolved.bucket.as_deref(), Some("extra"));
        assert_eq!(resolved.manifest.version(), "9.9.9");
    }

    /// An explicit bucket (and the picker's re-resolution) reads only that
    /// bucket and reports no candidates.
    #[tokio::test]
    async fn explicit_bucket_short_circuits_the_scan() {
        let (session, root) = setup("get-manifest-explicit-bucket");
        add_bucket(&root, "main", "demo", "1.0.0");
        add_bucket(&root, "extra", "demo", "9.9.9");

        let resolved = get_manifest(&session, "demo", Some("extra")).await.unwrap();
        assert_eq!(resolved.bucket.as_deref(), Some("extra"));
        assert_eq!(resolved.manifest.version(), "9.9.9");
        assert!(resolved.candidates.is_empty());
    }

    /// `bucket/name@version` resolves by name and bucket; the version part is
    /// not a manifest selector (upstream ignores it here).
    #[tokio::test]
    async fn bucket_qualified_version_query_resolves_by_name() {
        let (session, root) = setup("get-manifest-at-version");
        add_bucket(&root, "main", "demo", "1.0.0");

        let resolved = get_manifest(&session, "main/demo@2.0.0", None)
            .await
            .unwrap();
        assert_eq!(resolved.name, "demo");
        assert_eq!(resolved.bucket.as_deref(), Some("main"));
        assert_eq!(resolved.manifest.version(), "1.0.0");
    }

    /// An installed app resolves from its recorded bucket even when that
    /// bucket's live manifest is gone — via `deprecated/`.
    #[tokio::test]
    async fn installed_app_falls_back_to_deprecated() {
        let (session, root) = setup("get-manifest-deprecated");
        install(
            &root,
            "oldapp",
            "0.9.0",
            r#"{"bucket":"main","architecture":"64bit"}"#,
        );
        let dep = root.join("buckets/main/deprecated");
        std::fs::create_dir_all(&dep).unwrap();
        std::fs::write(
            dep.join("oldapp.json"),
            r#"{"version":"0.9.0","description":"deprecated"}"#,
        )
        .unwrap();

        let resolved = get_manifest(&session, "oldapp", None).await.unwrap();
        assert_eq!(resolved.name, "oldapp");
        assert_eq!(resolved.bucket.as_deref(), Some("main"));
        assert_eq!(resolved.manifest.version(), "0.9.0");
    }

    /// An app installed from a local manifest path resolves through its
    /// recorded source `url` when no bucket holds it.
    #[tokio::test]
    async fn url_install_resolves_through_the_recorded_source() {
        let (session, root) = setup("get-manifest-url-install");
        let source = root.join("local/onlyinstalled.json");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(
            &source,
            r#"{"version":"5.0.0","description":"url installed"}"#,
        )
        .unwrap();
        let install_json = serde_json::json!({
            "url": source.display().to_string(),
            "architecture": "64bit",
        })
        .to_string();
        install(&root, "onlyinstalled", "5.0.0", &install_json);

        let resolved = get_manifest(&session, "onlyinstalled", None).await.unwrap();
        assert_eq!(resolved.name, "onlyinstalled");
        assert!(resolved.bucket.is_none());
        assert_eq!(resolved.manifest.version(), "5.0.0");
    }

    /// An install whose bucket record is gone and which has no source `url`
    /// falls back to the installed `manifest.json` (upstream `installed_manifest`).
    #[tokio::test]
    async fn installed_manifest_is_the_last_resort() {
        let (session, root) = setup("get-manifest-installed-fallback");
        install(&root, "noorigin", "1.2.3", r#"{"architecture":"64bit"}"#);

        let resolved = get_manifest(&session, "noorigin", None).await.unwrap();
        assert_eq!(resolved.name, "noorigin");
        assert_eq!(resolved.manifest.version(), "1.2.3");
        assert!(resolved.bucket.is_none());
        assert!(resolved.url.is_none());
    }

    #[tokio::test]
    async fn generate_user_manifest_matching_version_returns_as_is() {
        let (session, root) = setup("gen-match-version");
        let manifest_path = root.join("app.json");
        std::fs::write(
            &manifest_path,
            r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
        )
        .unwrap();

        let resolved =
            generate_user_manifest(&session, manifest_path.to_str().unwrap(), None, "1.0.0")
                .await
                .unwrap();
        assert_eq!(resolved.manifest.version(), "1.0.0");
    }

    #[tokio::test]
    async fn generate_user_manifest_without_autoupdate_errors() {
        let (session, root) = setup("gen-no-autoupdate");
        let manifest_path = root.join("app.json");
        std::fs::write(
            &manifest_path,
            r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
        )
        .unwrap();

        let err = generate_user_manifest(&session, manifest_path.to_str().unwrap(), None, "2.0.0")
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("does not have autoupdate capability"),
            "{}",
            msg
        );
        assert!(msg.contains("@2.0.0"), "{}", msg);
    }

    #[tokio::test]
    async fn generate_user_manifest_applies_version_substitution() {
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
                .await
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
        let resolved = generate_user_manifest(&session, no_url.to_str().unwrap(), None, "2.0.0")
            .await
            .unwrap();
        assert_eq!(resolved.manifest.version(), "2.0.0");
        assert!(session.workspace_dir().join("nourl.json").exists());
    }
}
