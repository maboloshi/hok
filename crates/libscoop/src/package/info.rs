//! The `info` view of a package.
//!
//! Mirrors upstream `scoop-info.ps1` (libexec) field for field: the same
//! values under the same conditions, including the `-v/--verbose` extras
//! (manifest path, install layout, on-disk sizes, download size). This module
//! only *assembles* the view — labels, colours and layout belong to the CLI
//! renderer (`src/cmd/info.rs`), so the CLI keeps no filesystem access and no
//! business logic of its own.
//!
//! # Named deviations from upstream
//!
//! - `Installed size` totals are computed with reparse points (the `current`
//!   junction) skipped, like PowerShell's `Get-ChildItem -Recurse`; following
//!   them would count the current version twice.
//! - A `bin` entry that upstream writes as a one-element array renders as its
//!   path instead of `$_[1]`-as-null (`".exe"`); hok normalizes the bare
//!   string form to that same one-element shape. A `shortcuts` entry written as
//!   a bare string renders the string, where upstream indexes `$_[1]` and
//!   prints its second character.
//! - `Download size` probes the architecture hok would install for
//!   (`default_architecture` included) where upstream uses the host OS
//!   architecture (`Get-DefaultArchitecture`).

use std::path::Path;

use crate::Session;
use crate::bucket::Bucket;
use crate::constant::ISOLATED_PACKAGE_BUCKET;
use crate::error::Fallible;
use crate::internal::arch::Arch;
use crate::internal::{git, os, time};
use crate::network;
use crate::package::manifest::{License, Manifest};
use crate::package::{
    Package, QueryOption, identity, install_state, manifest_source, query, source,
};

/// How long a `Download size` probe may take before it counts as a failure.
const HEAD_TIMEOUT_SECS: u64 = 30;

/// Upstream's placeholder for the three substitution directories when `info`
/// runs without `-v` (libexec/scoop-info.ps1:57-59). It reaches the user
/// verbatim through `Environment`, `Path Added` and `Notes`.
const ROOT_PLACEHOLDER: &str = "<root>";

/// The assembled `info` view of one package, in upstream's field order.
#[derive(Debug, Clone)]
pub struct Info {
    /// Upstream `Name`, without the `(DEPRECATED)` marker (see [`Info::deprecated`]).
    pub name: String,
    /// Upstream `$status.deprecated`: the app's manifest lives in the origin
    /// bucket's `deprecated/` folder.
    pub deprecated: bool,
    /// Upstream `Description` — omitted when the manifest has none.
    pub description: Option<String>,
    /// Upstream `Version`.
    pub version: VersionOutput,
    /// Upstream `Source`: the standalone query, the install bucket, the install
    /// url, or the bucket the manifest was found in.
    pub source: String,
    /// Upstream `Website` — the homepage without its trailing `/`.
    pub website: Option<String>,
    /// Upstream `License`.
    pub license: Option<String>,
    /// Upstream `Dependencies` — `depends` joined with ` | `.
    pub dependencies: Option<String>,
    /// Upstream `Updated at` — the manifest's last commit date, or its mtime.
    pub updated_at: Option<String>,
    /// Upstream `Updated by` — that commit's author, or the file's owner.
    pub updated_by: Option<String>,
    /// Upstream `Manifest` — only filled under `-v`.
    pub manifest_file: Option<String>,
    /// Upstream `Installed` — one row per installed version directory.
    pub installed: Option<Vec<String>>,
    /// Upstream `Installed size` — only under `-v`, for installed apps.
    pub installed_size: Option<InstalledSize>,
    /// Upstream `Download size` — only under `-v`, for apps not installed.
    pub download_size: Option<DownloadSize>,
    /// Upstream `Binaries` — joined with ` | `.
    pub binaries: Option<String>,
    /// Upstream `Shortcuts` — joined with ` | `.
    pub shortcuts: Option<String>,
    /// Upstream `Environment` — one `NAME = value` row per variable.
    pub environment: Option<Vec<String>>,
    /// Upstream `Path Added` — one row per added directory.
    pub path_added: Option<Vec<String>>,
    /// Upstream `Suggestions` — joined with ` | `.
    pub suggestions: Option<String>,
    /// Upstream `Notes` — one row per note.
    pub notes: Option<Vec<String>>,
}

/// Upstream `$version_output` (libexec/scoop-info.ps1:28, 61-76).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionOutput {
    /// The manifest version: nothing is installed, or — for a standalone
    /// query — the installed app came from a different source.
    Manifest(String),
    /// The installed version, identical to the manifest version.
    Installed(String),
    /// The installed version with an update available.
    UpdateAvailable { installed: String, latest: String },
}

/// Upstream's `$fileTotals` behind `Installed size` (scoop-info.ps1:149-196).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InstalledSize {
    /// Files under the current version directory.
    pub current: u64,
    /// Files under the app directory minus [`InstalledSize::current`].
    pub old: u64,
    /// Files under the app's persist directory.
    pub persisted: u64,
    /// Cached downloads of this app.
    pub cached: u64,
    /// App + persisted + cached.
    pub total: u64,
}

/// Upstream's `Download size` probe result (scoop-info.ps1:198-226).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadSize {
    /// Total bytes over the default-architecture URLs; `None` when unknown.
    pub bytes: Option<u64>,
    /// Upstream appends ` (latest version is cached)` when the last probed URL
    /// is already in the download cache.
    pub cached: bool,
    /// The probe that left the size unknown, when any.
    pub error: Option<DownloadSizeError>,
}

/// Why a `Download size` probe could not produce a size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadSizeError {
    /// The server answered without a `Content-Length` header.
    NoContentLength { host: String },
    /// The request failed outright (upstream: "the server ... is down").
    Down { host: String },
}

/// What `info` found for a query.
#[derive(Debug)]
pub enum Lookup {
    /// No manifest matched the query.
    NotFound,
    /// One package to describe.
    Single(Box<Info>),
    /// Several matches, at least one of them installed: describe exactly those
    /// (upstream's `Get-Manifest` describes the installed app).
    Installed(Vec<Info>),
    /// Several matches, none installed: the compact candidate list.
    Candidates(Vec<Package>),
}

/// Resolve `query` and assemble the `info` view(s).
///
/// Upstream resolves the query with `Get-Manifest` (a URL / local manifest
/// first, then an exact-name bucket lookup) and then prefers the *global*
/// install when the app is installed there (`$global = installed $app $true`,
/// libexec/scoop-info.ps1:24). Both are mirrored here.
pub async fn lookup(session: &Session, query: &str, verbose: bool) -> Fallible<Lookup> {
    // A manifest URL, a UNC path or an existing local file is a standalone
    // source (upstream scoop-info.ps1:36).
    if identity::is_manifest_url(query) || Path::new(query).exists() {
        return Ok(Lookup::Single(Box::new(
            standalone(session, query, verbose).await?,
        )));
    }

    // Prefer the global install when the app lives there: upstream describes
    // that install (paths, versions and `Installed` all follow `$global`).
    if installed_in_global(session, identity::split_bucket_query(query).1) {
        session.set_global(true);
    }

    // Upstream `Get-Manifest` looks the app up by exact name in each added
    // bucket (`manifest $app $bucket`), never as a pattern — `Explicit` gives
    // the same one-app-per-bucket result (`cat`/`home`/`depends` do the same).
    let packages = query::query(session, vec![query], vec![QueryOption::Explicit], false)?;
    match packages.len() {
        0 => Ok(Lookup::NotFound),
        1 => Ok(Lookup::Single(Box::new(
            for_package(session, &packages[0], verbose).await?,
        ))),
        _ => {
            // A bare name can match the same app in several buckets. Upstream
            // describes the app as recorded in `install.json`, i.e. the bucket
            // it was installed from, so prefer the strictly installed
            // candidate; failing that, any installed candidate; otherwise list
            // the candidates.
            let strict = packages
                .iter()
                .filter(|pkg| pkg.is_strictly_installed())
                .collect::<Vec<_>>();
            let candidates = if strict.is_empty() {
                packages
                    .iter()
                    .filter(|pkg| pkg.is_installed())
                    .collect::<Vec<_>>()
            } else {
                strict
            };

            if candidates.is_empty() {
                return Ok(Lookup::Candidates(packages));
            }
            let mut infos = Vec::with_capacity(candidates.len());
            for pkg in candidates {
                infos.push(for_package(session, pkg, verbose).await?);
            }
            Ok(Lookup::Installed(infos))
        }
    }
}

/// Assemble the `info` view of a bucket package.
pub async fn for_package(session: &Session, pkg: &Package, verbose: bool) -> Fallible<Info> {
    let name = pkg.name();
    let installed = installed_facts(pkg);
    // Upstream `$status.deprecated`: only a *bucket* install has an origin
    // bucket to look the deprecated folder up in (core.ps1:557).
    let deprecated = installed
        .as_ref()
        .and_then(|f| f.bucket)
        .and_then(|bucket| source::deprecated_manifest_path(session, bucket, name));

    let parts = Parts {
        name,
        manifest: pkg.manifest(),
        bucket: Some(pkg.bucket()),
        installed,
        standalone: None,
        same_source: false,
        deprecated: deprecated.as_deref(),
        verbose,
    };
    build(session, parts).await
}

/// Assemble the `info` view of a standalone query: a manifest URL, a UNC path
/// or a local manifest file (upstream `Get-Manifest`'s `$url` branch).
async fn standalone(session: &Session, query: &str, verbose: bool) -> Fallible<Info> {
    let resolved = manifest_source::resolve_manifest(session, query, None).await?;
    let name = resolved.name;

    // Upstream absolutizes a local path before using it as the source
    // (scoop-info.ps1:36-40). `std::path::absolute` matches `Get-AbsolutePath`
    // (no symlink resolution, no `\\?\` device prefix).
    let source = absolute(query);

    if installed_in_global(session, &name) {
        session.set_global(true);
    }
    let installed_pkg =
        query::query_installed(session, &[name.as_str()], &[QueryOption::Explicit])?
            .into_iter()
            .next();
    let installed = installed_pkg.as_ref().and_then(installed_facts);

    // Upstream `$same_source`: the query and the recorded install url point at
    // the same manifest (scoop-info.ps1:41-50).
    let same_source = installed.as_ref().is_some_and(|f| {
        f.url
            .is_some_and(|url| absolute(url).eq_ignore_ascii_case(&source))
    });

    let deprecated = installed
        .as_ref()
        .and_then(|f| f.bucket)
        .and_then(|bucket| source::deprecated_manifest_path(session, bucket, &name));

    let parts = Parts {
        name: &name,
        manifest: &resolved.manifest,
        bucket: None,
        installed,
        standalone: Some(&source),
        same_source,
        deprecated: deprecated.as_deref(),
        verbose,
    };
    build(session, parts).await
}

/// The installed half of the view: upstream `install_info` + `app_status`.
struct InstalledFacts<'a> {
    /// Upstream `$status.version` (the current installed version).
    version: &'a str,
    /// Upstream `$install.bucket` — `None` for URL / local installs.
    bucket: Option<&'a str>,
    /// Upstream `$install.architecture`.
    arch: Option<&'a str>,
    /// Upstream `$install.url`.
    url: Option<&'a str>,
}

/// Extract the installed facts of a package, or `None` when it is not installed.
fn installed_facts(pkg: &Package) -> Option<InstalledFacts<'_>> {
    let version = pkg.installed_version()?;
    Some(InstalledFacts {
        version,
        // hok reports URL / local installs under the isolated bucket, where
        // upstream leaves `install.json`'s `bucket` null and falls back to
        // `url` (scoop-info.ps1:90-96).
        bucket: pkg
            .installed_bucket()
            .filter(|b| *b != ISOLATED_PACKAGE_BUCKET),
        arch: pkg.installed_arch(),
        url: pkg.installed_url(),
    })
}

/// Everything [`build`] needs — the upstream locals of `scoop-info.ps1`.
struct Parts<'a> {
    /// Upstream `$app`.
    name: &'a str,
    /// The manifest to describe.
    manifest: &'a Manifest,
    /// Upstream `$bucket`: the bucket the manifest was found in.
    bucket: Option<&'a str>,
    /// Upstream `$install` / `$status.version`, when the app is installed.
    installed: Option<InstalledFacts<'a>>,
    /// Upstream `$url` — set for standalone (URL / local manifest) queries.
    standalone: Option<&'a str>,
    /// Upstream `$same_source` (standalone queries only).
    same_source: bool,
    /// Upstream `$status.deprecated`: the deprecated manifest file, if any.
    deprecated: Option<&'a Path>,
    /// Upstream's `-v` flag.
    verbose: bool,
}

/// Assemble one [`Info`] from the resolved parts.
async fn build(session: &Session, parts: Parts<'_>) -> Fallible<Info> {
    let Parts {
        name,
        manifest,
        bucket,
        installed,
        standalone,
        same_source,
        deprecated,
        verbose,
    } = parts;

    // Upstream `$manifest_file` (scoop-info.ps1:29-33, 61-75): the bucket
    // manifest, the standalone source, and — for installed apps — the manifest
    // of the *installing* bucket, overridden by the install url and finally by
    // the deprecated manifest.
    let mut manifest_file = match (bucket, standalone) {
        (_, Some(url)) => Some(url.to_owned()),
        (Some(bucket), None) => bucket_manifest_path(session, bucket, name),
        (None, None) => None,
    };
    if let Some(facts) = &installed {
        manifest_file = facts
            .bucket
            .and_then(|bucket| bucket_manifest_path(session, bucket, name));
        if let Some(url) = facts.url {
            manifest_file = Some(url.to_owned());
        }
        if let Some(deprecated) = deprecated {
            manifest_file = Some(deprecated.display().to_string());
        }
    }

    // Substitution directories for `$dir` / `$original_dir` / `$persist_dir`
    // (scoop-info.ps1:53-59): real paths under `-v`, upstream's `<root>`
    // placeholder otherwise.
    let (dir, original_dir, persist_dir) = if verbose {
        (
            session
                .app_current_dir(name)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| ROOT_PLACEHOLDER.to_owned()),
            session
                .versioned_dir(name, manifest.version())
                .display()
                .to_string(),
            session.persist_dir(name).display().to_string(),
        )
    } else {
        (
            ROOT_PLACEHOLDER.to_owned(),
            ROOT_PLACEHOLDER.to_owned(),
            ROOT_PLACEHOLDER.to_owned(),
        )
    };

    let version = match &installed {
        None => VersionOutput::Manifest(manifest.version().to_owned()),
        Some(facts) => {
            if standalone.is_some() && !same_source {
                VersionOutput::Manifest(manifest.version().to_owned())
            } else if facts.version == manifest.version() {
                VersionOutput::Installed(facts.version.to_owned())
            } else {
                VersionOutput::UpdateAvailable {
                    installed: facts.version.to_owned(),
                    latest: manifest.version().to_owned(),
                }
            }
        }
    };

    let source = match (standalone, &installed) {
        (Some(url), _) => url.to_owned(),
        (None, Some(facts)) => match (facts.bucket, facts.url) {
            (Some(bucket), _) => bucket.to_owned(),
            (None, Some(url)) => url.to_owned(),
            (None, None) => bucket.unwrap_or_default().to_owned(),
        },
        (None, None) => bucket.unwrap_or_default().to_owned(),
    };

    let homepage = manifest.homepage();
    let website = (!homepage.is_empty()).then(|| homepage.trim_end_matches('/').to_owned());

    let dependencies = manifest
        .depends()
        .map(|deps| deps.join(" | "))
        .filter(|joined| !joined.is_empty());

    // Upstream reads git metadata (and the size/version probes) only for a
    // manifest that exists on disk; a standalone URL never does.
    let (updated_at, updated_by) = manifest_file
        .as_deref()
        .map(Path::new)
        .filter(|path| path.is_file())
        .map(manifest_updated)
        .unwrap_or((None, None));

    // Upstream gates `Installed` on `!$standalone -or $same_source`.
    let installed_rows = if standalone.is_some() && !same_source {
        None
    } else {
        installed.as_ref().map(|_| {
            installed_version_dirs(session, name)
                .into_iter()
                .map(|version| {
                    if verbose {
                        session.versioned_dir(name, &version).display().to_string()
                    } else {
                        version
                    }
                })
                .collect::<Vec<_>>()
        })
    };

    let (installed_size, download_size) = match (verbose, &installed) {
        (true, Some(_)) => (Some(measure_installed_size(session, name)), None),
        (true, None) => (
            None,
            Some(probe_download_size(session, manifest, name).await),
        ),
        (false, _) => (None, None),
    };

    // Upstream passes `$install.architecture`, which is `$null` for an app that
    // is not installed; `arch_specific` then only ever falls back to the
    // generic field (scoop-info.ps1:229-268).
    let arch = installed
        .as_ref()
        .and_then(|facts| facts.arch)
        .and_then(|arch| Arch::parse(arch).ok());
    let (bin, shortcuts, env_set, env_add_path) = match arch {
        Some(arch) => (
            manifest.bin_for(arch),
            manifest.shortcuts_for(arch),
            manifest.env_set_for(arch),
            manifest.env_add_path_for(arch),
        ),
        None => (
            manifest.bin_generic(),
            manifest.shortcuts_generic(),
            manifest.env_set_generic(),
            manifest.env_add_path_generic(),
        ),
    };

    let binaries =
        bin.and_then(|entries| join_rendered(entries.iter().map(|entry| render_bin_entry(entry))));
    let shortcuts = shortcuts.and_then(|entries| {
        join_rendered(entries.iter().map(|entry| render_shortcut_entry(entry)))
    });

    let environment = env_set.map(|vars| {
        let mut keys = vars.keys().collect::<Vec<_>>();
        keys.sort();
        keys.into_iter()
            .map(|key| format!("{key} = {}", substitute(&vars[key], "$dir", &dir)))
            .collect::<Vec<_>>()
    });

    let path_added = env_add_path.map(|paths| {
        paths
            .iter()
            .filter(|path| !path.is_empty())
            .map(|path| {
                if *path == "." {
                    dir.clone()
                } else {
                    format!("{dir}\\{path}")
                }
            })
            .collect::<Vec<_>>()
    });

    // Upstream joins each suggestion group's values, then the groups, in
    // manifest order (`PSObject.Properties`, scoop-info.ps1:270-276) — hence
    // `suggest()` is an order-preserving map.
    let suggestions = manifest.suggest().map(|suggest| {
        suggest
            .values()
            .map(|values| values.devectorize().join(" | "))
            .collect::<Vec<_>>()
            .join(" | ")
    });

    let notes = manifest.notes().map(|notes| {
        notes
            .iter()
            .map(|note| {
                // `$dir` is a suffix of neither `$original_dir` nor
                // `$persist_dir`, so the replacement order does not matter.
                let note = substitute(note, "$original_dir", &original_dir);
                let note = substitute(&note, "$persist_dir", &persist_dir);
                substitute(&note, "$dir", &dir)
            })
            .collect::<Vec<_>>()
    });

    Ok(Info {
        name: name.to_owned(),
        deprecated: deprecated.is_some(),
        description: manifest.description().map(str::to_owned),
        version,
        source,
        website,
        license: license_text(manifest.license(), verbose),
        dependencies,
        updated_at,
        updated_by,
        // Upstream only adds `Manifest` under `-v`.
        manifest_file: if verbose { manifest_file } else { None },
        installed: installed_rows,
        installed_size,
        download_size,
        binaries,
        shortcuts,
        environment,
        path_added,
        suggestions,
        notes,
    })
}

/// Upstream `manifest_path $app $bucket` (lib/manifest.ps1:1-3): the live
/// manifest file of `name` in `bucket`, falling back to the deprecated one.
fn bucket_manifest_path(session: &Session, bucket: &str, name: &str) -> Option<String> {
    let bucket_dir = session.bucket_dir(bucket);
    let bucket_obj = Bucket::from(&bucket_dir).ok()?;
    bucket_obj
        .path_of_manifest(name)
        .or_else(|| source::deprecated_manifest_path(session, bucket, name))
        .map(|path| path.display().to_string())
}

/// Whether `name` resolves to an installed version under the *global* root —
/// upstream `installed $app $true` (lib/core.ps1:386-395).
fn installed_in_global(session: &Session, name: &str) -> bool {
    let app_dir = session.config().global_path().join("apps").join(name);
    install_state::resolve_current_version(&app_dir, session.config().no_junction()).is_some()
}

/// All installed versions of `name`, oldest first.
///
/// Mirrors upstream `Get-InstalledVersion` (lib/versions.ps1:71-102): only
/// version directories holding an `install.json` count, the `current` junction
/// and `_*.old*` backups are excluded, and the order is the `install.json`
/// last-write time — *not* the version order that `hok list --versions` uses.
fn installed_version_dirs(session: &Session, name: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(session.app_dir(name)) else {
        return Vec::new();
    };

    let mut versions = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let version = entry.file_name().to_string_lossy().to_string();
        if version == "current" || (version.starts_with('_') && version.contains(".old")) {
            continue;
        }
        let Ok(modified) =
            std::fs::metadata(entry.path().join("install.json")).and_then(|meta| meta.modified())
        else {
            continue;
        };
        versions.push((modified, version));
    }
    versions.sort_by_key(|(modified, _)| *modified);
    versions.into_iter().map(|(_, version)| version).collect()
}

/// Upstream's `$fileTotals` (scoop-info.ps1:149-195).
fn measure_installed_size(session: &Session, name: &str) -> InstalledSize {
    let app_dir = session.app_dir(name);
    let app_total = path_size(&app_dir);
    let current = session
        .app_current_dir(name)
        .map(|dir| path_size(&dir))
        .unwrap_or(0);
    let persisted = path_size(&session.persist_dir(name));
    let cached = cached_download_size(session, name);

    InstalledSize {
        current,
        old: app_total.saturating_sub(current),
        persisted,
        cached,
        total: app_total + persisted + cached,
    }
}

/// Recursive file-size total of `path` (0 when it does not exist).
///
/// Reparse points (`current` is a junction) are not followed: PowerShell's
/// `Get-ChildItem -Recurse` does not descend into them either, and following
/// the junction would count the current version twice.
fn path_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if is_reparse_point(&entry) {
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

/// Whether a directory entry is a reparse point (a symlink, or the `current`
/// junction) that must not be traversed.
fn is_reparse_point(entry: &std::fs::DirEntry) -> bool {
    /// `FILE_ATTRIBUTE_REPARSE_POINT`.
    #[cfg(windows)]
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    // `DirEntry::metadata` does not traverse the link, so the attributes
    // describe the link itself.
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        entry
            .metadata()
            .map(|meta| meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
            .unwrap_or(true)
    }
    #[cfg(not(windows))]
    {
        entry
            .file_type()
            .map(|file_type| file_type.is_symlink())
            .unwrap_or(true)
    }
}

/// Size of this app's cached downloads — upstream
/// `Get-ChildItem $cachedir -Filter "$app#*"` (scoop-info.ps1:157).
fn cached_download_size(session: &Session, name: &str) -> u64 {
    let prefix = format!("{name}#");
    let Ok(entries) = std::fs::read_dir(session.config().cache_path()) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .filter_map(|entry| entry.metadata().ok())
        .filter(|meta| meta.is_file())
        .map(|meta| meta.len())
        .sum()
}

/// Probe the default architecture's download URLs with `HEAD` for a total
/// size, mirroring upstream's `Get-RemoteFileSize` loop
/// (scoop-info.ps1:198-226).
async fn probe_download_size(session: &Session, manifest: &Manifest, name: &str) -> DownloadSize {
    let version = manifest.version();
    // `Arch::current()` honours the `default_architecture` config / `-a`, i.e.
    // the architecture hok would install for (see the module docs).
    let urls = manifest.url_for(Arch::current());
    let cache_dir = session.config().cache_path().to_path_buf();
    let cookies = manifest.cookie();

    let mut total = 0u64;
    let mut cached = false;
    let mut error = None;

    for url in urls {
        // Upstream checks the cache *before* probing, and reports the state of
        // the last iterated URL.
        cached = cache_dir
            .join(crate::package::cache_file_name(name, version, url))
            .exists();

        let result = network::head_url(session, url, HEAD_TIMEOUT_SECS, cookies).await;
        // Upstream distinguishes the two failures by exception type: an HTTP
        // error carries a `RuntimeException` ("did not send a Content-Length
        // header"), while a transport failure is a `WebException` ("is down").
        match (result.status_code, result.content_length) {
            (0, _) => {
                error = Some(DownloadSizeError::Down { host: host_of(url) });
                total = 0;
                break;
            }
            (_, Some(length)) if result.error.is_none() => total += length,
            _ => {
                error = Some(DownloadSizeError::NoContentLength { host: host_of(url) });
                total = 0;
                break;
            }
        }
    }

    DownloadSize {
        bytes: (total != 0).then_some(total),
        cached,
        error,
    }
}

/// The host of `url` for upstream's failure messages ("the server at X ...").
fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    rest.split(['/', '\\']).next().unwrap_or(rest).to_owned()
}

/// The absolute form of a local path, matching PowerShell's `Get-AbsolutePath`
/// (no symlink resolution, no `\\?\` device prefix). Non-paths (URLs) are
/// returned unchanged.
fn absolute(path: &str) -> String {
    if identity::is_manifest_url(path) {
        return path.to_owned();
    }
    std::path::absolute(path)
        .map(|abs| abs.display().to_string())
        .unwrap_or_else(|_| path.to_owned())
}

/// Upstream `Updated at` / `Updated by` (scoop-info.ps1:123-134): the manifest
/// file's last commit, falling back to its mtime and owner.
fn manifest_updated(path: &Path) -> (Option<String>, Option<String>) {
    if let Some((secs, author)) = git::last_commit_for_file(path) {
        return (time::format_short_local(secs), Some(author));
    }

    let modified = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|since| time::format_short_local(since.as_secs() as i64));
    (modified, os::file_owner_name(path))
}

/// Upstream's `License` rendering (scoop-info.ps1:102-117): the bare license
/// without `-v`, with its reference link under `-v`.
fn license_text(license: &License, verbose: bool) -> Option<String> {
    let identifier = license.identifier();
    if identifier.is_empty() {
        return None;
    }
    if !verbose {
        return Some(identifier.to_owned());
    }
    if let Some(url) = license.url() {
        return Some(format!("{identifier} ({url})"));
    }
    if identifier.starts_with("http://")
        || identifier.starts_with("https://")
        || identifier.starts_with("ftp://")
    {
        return Some(identifier.to_owned());
    }
    // `A|B` (the multi-license form) links every part.
    if identifier.contains('|') || identifier.contains(',') {
        let links = identifier
            .split(['|', ','])
            .map(|part| format!("https://spdx.org/licenses/{part}.html"))
            .collect::<Vec<_>>()
            .join(", ");
        return Some(format!("{identifier} ({links})"));
    }
    Some(format!(
        "{identifier} (https://spdx.org/licenses/{identifier}.html)"
    ))
}

/// Upstream's `Environment` / `Notes` placeholder substitution
/// (`substitute`, lib/core.ps1:1203): a literal `$dir`-style token swap.
fn substitute(text: &str, token: &str, value: &str) -> String {
    text.replace(token, value)
}

/// One `Binaries` entry (scoop-info.ps1:233-237): a bare path stays as-is, the
/// `[file, alias]` tuple shows as `<alias>.<extension-of-file>`.
fn render_bin_entry(entry: &[&str]) -> String {
    match entry {
        [file, alias, ..] => {
            let extension = file.rsplit('.').next().unwrap_or_default();
            format!("{alias}.{extension}")
        }
        [file] => (*file).to_owned(),
        [] => String::new(),
    }
}

/// One `Shortcuts` entry: upstream takes element 1 of the `[target, name]`
/// tuple form and shows a bare name as-is.
fn render_shortcut_entry(entry: &[&str]) -> String {
    match entry {
        [_, name, ..] => (*name).to_owned(),
        [name] => (*name).to_owned(),
        [] => String::new(),
    }
}

/// Join rendered entries with upstream's ` | `, dropping empty ones so a
/// malformed entry cannot produce an empty field.
fn join_rendered(items: impl Iterator<Item = String>) -> Option<String> {
    let rendered = items.filter(|item| !item.is_empty()).collect::<Vec<_>>();
    (!rendered.is_empty()).then(|| rendered.join(" | "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{
        FakeNetwork, mark_installed, test_session, tmpdir, write_bucket_manifest,
    };
    use std::sync::Arc;

    /// A manifest exercising every conditional field upstream renders.
    const FULL: &str = r#"{
        "version": "1.0.0",
        "description": "An app",
        "homepage": "https://example.com/",
        "license": "MIT",
        "depends": ["dep1", "dep2"],
        "env_set": {"APP_HOME": "$dir\\home"},
        "env_add_path": ["bin", "."],
        "suggest": {"Notes": ["a", "b"], "Alt": ["c"]},
        "notes": ["Run $dir\\x", "Then $original_dir and $persist_dir"],
        "bin": ["generic.exe"]
    }"#;

    /// Resolve `name` the way the CLI does (bucket manifest + install state).
    fn resolve(session: &Session, name: &str) -> Package {
        query::query(session, vec![name], vec![], false)
            .unwrap()
            .remove(0)
    }

    /// Like [`test_session`], with the global root redirected inside the test
    /// root so the global-install preference can be exercised offline.
    fn test_session_with_global(root: &Path) -> Session {
        let escape = |path: &Path| path.to_string_lossy().replace('\\', "\\\\");
        let config_path = root.join("hok-global.json");
        std::fs::write(
            &config_path,
            format!(
                r#"{{"root_path": "{}", "global_path": "{}", "cache_path": "{}"}}"#,
                escape(root),
                escape(&root.join("global")),
                escape(&root.join("cache")),
            ),
        )
        .unwrap();
        Session::new_with(&config_path).unwrap()
    }

    /// Add a version directory (with the `install.json` upstream requires for
    /// a version to count as installed) next to the `current` metadata.
    fn add_version_dir(root: &Path, name: &str, version: &str) {
        let dir = root.join("apps").join(name).join(version);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("install.json"),
            r#"{"architecture": "64bit", "bucket": "main"}"#,
        )
        .unwrap();
    }

    #[tokio::test]
    async fn not_installed_uses_generic_fields_and_the_root_placeholder() {
        let root = tmpdir("info_not_installed");
        let session = test_session(&root);
        write_bucket_manifest(&root, "main", "app", FULL);

        let info = for_package(&session, &resolve(&session, "app"), false)
            .await
            .unwrap();

        assert_eq!(info.version, VersionOutput::Manifest("1.0.0".to_owned()));
        assert_eq!(info.source, "main");
        // `TrimEnd('/')`.
        assert_eq!(info.website.as_deref(), Some("https://example.com"));
        assert_eq!(info.license.as_deref(), Some("MIT"));
        assert_eq!(info.dependencies.as_deref(), Some("dep1 | dep2"));
        assert_eq!(info.binaries.as_deref(), Some("generic.exe"));
        // Upstream substitutes `$dir` with the literal `<root>` without `-v`.
        assert_eq!(info.environment.unwrap(), vec!["APP_HOME = <root>\\home"]);
        assert_eq!(info.path_added.unwrap(), vec!["<root>\\bin", "<root>"]);
        // Declaration order, not alphabetical (upstream renders property order).
        assert_eq!(info.suggestions.as_deref(), Some("a | b | c"));
        assert_eq!(
            info.notes.unwrap(),
            vec!["Run <root>\\x", "Then <root> and <root>"]
        );
        assert!(info.installed.is_none());
        assert!(info.installed_size.is_none() && info.download_size.is_none());
        assert!(info.manifest_file.is_none(), "Manifest is verbose-only");
        assert!(!info.deprecated);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn installed_app_uses_its_architecture_versions_and_paths() {
        let root = tmpdir("info_installed");
        let session = test_session(&root);
        let manifest = r#"{
            "version": "1.0.0",
            "bin": "generic.exe",
            "architecture": {"64bit": {"bin": [["x64.exe", "tool"]], "env_add_path": ["x64bin"]}}
        }"#;
        write_bucket_manifest(&root, "main", "app", manifest);
        mark_installed(&root, "app", "main", manifest, false);
        add_version_dir(&root, "app", "1.0.0");
        // Version dirs without an `install.json`, plus `current` and old
        // backups, must not be listed (upstream versions.ps1:98).
        std::fs::create_dir_all(root.join("apps").join("app").join("junk")).unwrap();
        add_version_dir(&root, "app", "_1.0.0.old");

        let info = for_package(&session, &resolve(&session, "app"), true)
            .await
            .unwrap();

        assert_eq!(info.version, VersionOutput::Installed("1.0.0".to_owned()));
        // Installed architecture (64bit) wins over the generic `bin`.
        assert_eq!(info.binaries.as_deref(), Some("tool.exe"));
        // …and over the generic `env_add_path`.
        assert!(info.path_added.unwrap()[0].ends_with("\\x64bin"));

        // `-v` lists the full version directory path.
        let installed = info.installed.unwrap();
        assert_eq!(installed.len(), 1);
        assert!(
            installed[0].replace('\\', "/").ends_with("apps/app/1.0.0"),
            "{installed:?}"
        );

        // `-v` also reports the manifest path and the on-disk size.
        assert!(info.manifest_file.unwrap().ends_with("app.json"));
        let size = info.installed_size.expect("installed size");
        assert!(size.total > 0);
        assert!(size.total >= size.current);
        assert_eq!((size.persisted, size.cached), (0, 0));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn verbose_download_size_reports_a_missing_content_length() {
        let root = tmpdir("info_download_unknown");
        let session = test_session(&root);
        // The fake serves no body for this URL: HEAD answers 200 without a
        // `Content-Length`, upstream's "did not send a Content-Length header".
        session.set_network(Arc::new(FakeNetwork::new())).unwrap();
        write_bucket_manifest(
            &root,
            "main",
            "app",
            r#"{"version": "1.0.0", "url": "https://fake.example.com/app.zip"}"#,
        );

        let info = for_package(&session, &resolve(&session, "app"), true)
            .await
            .unwrap();

        let size = info.download_size.expect("download size");
        assert_eq!(size.bytes, None);
        assert_eq!(
            size.error,
            Some(DownloadSizeError::NoContentLength {
                host: "fake.example.com".to_owned(),
            })
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn lookup_resolves_a_local_manifest_path_as_standalone() {
        let root = tmpdir("info_standalone");
        let session = test_session(&root);
        let manifest = root.join("lonely.json");
        std::fs::write(
            &manifest,
            r#"{"version": "3.1.4", "url": "https://x/y.zip"}"#,
        )
        .unwrap();

        let found = lookup(&session, manifest.to_str().unwrap(), false)
            .await
            .unwrap();
        let Lookup::Single(info) = found else {
            panic!("expected a single result");
        };

        assert_eq!(info.name, "lonely");
        // The source is the absolutized path, without a `\\?\` device prefix.
        assert_eq!(info.source, manifest.display().to_string());
        assert_eq!(info.version, VersionOutput::Manifest("3.1.4".to_owned()));
        // Nothing is installed under that name.
        assert!(info.installed.is_none());
        assert!(info.manifest_file.is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn lookup_prefers_the_global_install() {
        let root = tmpdir("info_global");
        let session = test_session_with_global(&root);
        write_bucket_manifest(&root, "main", "app", r#"{"version": "1.0.0"}"#);
        // Installed under the *global* root only.
        let current = root.join("global").join("apps").join("app").join("current");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("manifest.json"), r#"{"version": "1.0.0"}"#).unwrap();
        std::fs::write(
            current.join("install.json"),
            r#"{"architecture": "64bit", "bucket": "main"}"#,
        )
        .unwrap();

        let found = lookup(&session, "app", false).await.unwrap();
        let Lookup::Single(info) = found else {
            panic!("expected a single result");
        };

        assert!(session.is_global(), "the global root is selected");
        assert_eq!(info.version, VersionOutput::Installed("1.0.0".to_owned()));
        assert_eq!(info.source, "main");
        assert!(info.installed.is_some());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn newer_bucket_manifest_reports_update_available() {
        let root = tmpdir("info_outdated");
        let session = test_session(&root);
        write_bucket_manifest(&root, "main", "app", r#"{"version": "2.0.0"}"#);
        mark_installed(&root, "app", "main", r#"{"version": "1.0.0"}"#, false);

        let info = for_package(&session, &resolve(&session, "app"), false)
            .await
            .unwrap();

        assert_eq!(
            info.version,
            VersionOutput::UpdateAvailable {
                installed: "1.0.0".to_owned(),
                latest: "2.0.0".to_owned(),
            }
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn deprecated_manifest_is_flagged_and_used_as_the_manifest_file() {
        let root = tmpdir("info_deprecated");
        let session = test_session(&root);
        let dir = root
            .join("buckets")
            .join("main")
            .join("bucket")
            .join("deprecated");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("legacy.json"), r#"{"version": "1.0.0"}"#).unwrap();
        mark_installed(&root, "legacy", "main", r#"{"version": "1.0.0"}"#, false);

        let info = for_package(&session, &resolve(&session, "legacy"), true)
            .await
            .unwrap();

        assert!(info.deprecated);
        assert!(info.manifest_file.unwrap().ends_with("legacy.json") && info.deprecated);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn verbose_download_size_probes_the_default_architecture_urls() {
        let root = tmpdir("info_download_size");
        let session = test_session(&root);
        let url = "https://fake.example.com/app-1.0.0.zip";
        let fake = Arc::new(FakeNetwork::new());
        fake.serve(url, vec![0u8; 2048]);
        session.set_network(fake).unwrap();

        write_bucket_manifest(
            &root,
            "main",
            "app",
            &format!(r#"{{"version": "1.0.0", "url": "{url}"}}"#),
        );

        let info = for_package(&session, &resolve(&session, "app"), true)
            .await
            .unwrap();

        let size = info.download_size.expect("download size");
        assert_eq!(size.bytes, Some(2048));
        assert!(!size.cached);
        assert_eq!(size.error, None);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bin_entries_render_alias_with_the_file_extension() {
        assert_eq!(render_bin_entry(&["foo.exe"]), "foo.exe");
        assert_eq!(render_bin_entry(&["foo.exe", "bar"]), "bar.exe");
        assert_eq!(render_bin_entry(&["dir/foo"]), "dir/foo");
        assert_eq!(render_bin_entry(&["noext", "bar"]), "bar.noext");
    }

    #[test]
    fn shortcut_entries_take_the_name() {
        assert_eq!(render_shortcut_entry(&["$dir\\x.lnk", "Name"]), "Name");
        assert_eq!(render_shortcut_entry(&["Name"]), "Name");
    }

    #[test]
    fn license_rendering_adds_reference_links_only_when_verbose() {
        let mit = License::new("MIT".to_owned(), None);
        assert_eq!(license_text(&mit, false).as_deref(), Some("MIT"));
        assert_eq!(
            license_text(&mit, true).as_deref(),
            Some("MIT (https://spdx.org/licenses/MIT.html)")
        );

        let with_url = License::new("MIT".to_owned(), Some("https://x/mit".to_owned()));
        assert_eq!(
            license_text(&with_url, true).as_deref(),
            Some("MIT (https://x/mit)")
        );

        // A multi-license string links every part.
        let multi = License::new("A, B".to_owned(), None);
        assert_eq!(
            license_text(&multi, true).as_deref(),
            Some("A, B (https://spdx.org/licenses/A.html, https://spdx.org/licenses/ B.html)")
        );
        assert_eq!(license_text(&License::default(), false), None);
    }

    #[test]
    fn host_of_strips_scheme_and_path() {
        assert_eq!(host_of("https://example.com/a/b.zip"), "example.com");
        assert_eq!(host_of("http://example.com"), "example.com");
        assert_eq!(host_of("not-a-url"), "not-a-url");
    }

    /// `current` is a junction in a real install; following it would count the
    /// current version twice.
    #[cfg(windows)]
    #[test]
    fn path_size_does_not_follow_junctions() {
        let root = tmpdir("info_path_size");
        let target = root.join("target");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("big.bin"), vec![0u8; 4096]).unwrap();
        junction::create(&target, root.join("current")).unwrap();

        assert_eq!(path_size(&root), 4096);

        let _ = std::fs::remove_dir_all(&root);
    }
}
