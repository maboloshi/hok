//! Package representation and module organisation.
//!
//! Defines the core types and submodules of the `package` subdomain, serving as the main entry point for libscoop's package management functionality.
//!
//! # Design
//!
//! This module and its submodules collectively undertake the following responsibilities:
//!
//! - **Manifest Management** — Parse and validate Scoop manifest JSON files (`manifest`).
//! - **Package Discovery** — Recursively scan bucket directories for manifest files (`manifest_walker`).
//! - **Package Caching** — Cache bucket manifest indexes into SQLite to speed up queries ([`manifest_cache`]).
//! - **Package Querying** — Search packages across buckets by name, description, or binary name ([`query`]).
//! - **Dependency Resolution** — Resolve installation dependency order using a directed acyclic graph (DAG) (`resolve`).
//! - **Downloading** — Concurrent, resumable package file downloads (`download`).
//! - **Synchronization** — Complete install / upgrade / uninstall pipeline ([`sync`]).
//! - **Validation** — URL validity checks ([`checkurls`]), hash computation and comparison ([`checkhashes`]),
//!   version detection ([`checkver`]).
//! - **Tooling** — Manifest generation from a download URL ([`create`]), installed-version listing ([`list`]),
//!   dependency tree traversal ([`depends`]), shim inspection ([`shim`]), and export/import of the
//!   installed package set ([`export`], [`import`]).
//!
//! # Core Types
//!
//! - [`Package`] — Runtime package representation, holding manifest, installation status, upgradability status, etc.
//! - [`Manifest`] — Strongly typed parsing result of the manifest JSON.
//! - [`QueryOption`] — Enum controlling package query behavior (regex / exact / by description / upgradable, etc.).
//! - [`SyncOption`] — Enum controlling install/upgrade behavior.
//!
//! # Design Notes
//!
//! - **Lazy Fields**: `origin`, `install_state`, and `upgradable` are all populated lazily using `OnceCell`,
//!   avoiding parsing of installation state when not needed.
//! - **Concurrency Safety**: `Package` implements `Send + Sync`; the `query` submodule uses `rayon`
//!   to scan bucket manifests in parallel without requiring ownership of `Session`.

pub mod auto_pr;
// ─── Submodules ────────────────────────────────────────────────────────────
pub mod checkhashes;
pub mod checkup;
pub mod checkurls;
pub mod checkver;
pub mod cleanup;
pub mod create;
pub mod depends;
pub mod download;
pub mod export;
pub mod formatjson;
pub mod hold;
pub(crate) mod identity;
pub mod import;
pub mod list;
pub(crate) mod manifest;
pub mod manifest_cache;
pub(crate) mod manifest_source;
pub(crate) mod manifest_walker;
pub mod missing_checkver;
pub(crate) mod operations;
pub mod query;
pub(crate) mod resolve;
pub mod shim;
pub mod sync;
pub mod virustotal;

// ─── Core types & re-exports ───────────────────────────────────────────────

use std::cell::OnceCell;
use std::{fmt, path::PathBuf};

pub use manifest::{HashString, InstallInfo, License, Manifest};
pub use query::QueryOption;
pub use sync::SyncOption;

pub(crate) use identity::*;

use crate::{constant::ISOLATED_PACKAGE_BUCKET, internal};

/// A Scoop package.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Package {
    /// The bucket name of this package.
    bucket: String,

    /// The name of this package.
    name: String,

    /// The manifest of this package.
    manifest: Manifest,

    #[serde(skip)]
    origin: OnceCell<OriginateFrom>,

    /// The install state of the package.
    #[serde(skip)]
    install_state: OnceCell<InstallState>,

    /// The upgradable package, if any.
    ///
    /// This field is never serialized.
    #[serde(skip)]
    upgradable: OnceCell<Option<Box<Package>>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum OriginateFrom {
    Bucket(String),
    File(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum InstallState {
    NotInstalled,
    Installed(InstallStateInstalled),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallStateInstalled {
    pub version: String,
    pub bucket: Option<String>,
    pub arch: String,
    pub held: bool,
    pub url: Option<String>,
}

impl InstallStateInstalled {
    #[inline]
    pub fn bucket(&self) -> Option<&str> {
        self.bucket.as_deref()
    }

    #[inline]
    pub fn held(&self) -> bool {
        self.held
    }

    #[inline]
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    #[inline]
    pub fn version(&self) -> &str {
        self.version.as_str()
    }
}

impl Package {
    pub fn from(name: &str, bucket: &str, manifest: Manifest) -> Package {
        Package {
            bucket: bucket.to_owned(),
            name: name.to_owned(),
            manifest,
            origin: OnceCell::new(),
            install_state: OnceCell::new(),
            upgradable: OnceCell::new(),
        }
    }

    /// The identity of this package.
    ///
    /// # Returns
    ///
    /// The package identity in the form of `bucket/name`, which is unique for
    /// each package across all buckets.
    #[inline]
    pub fn ident(&self) -> String {
        format!("{}/{}", self.bucket, self.name)
    }

    /// Get the name of this package.
    #[inline]
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Whether this package matches a `bucket/name` query.
    ///
    /// Both parts are matched case-insensitively (Scoop is case-insensitive
    /// here — Windows FS lookup); a missing bucket prefix matches any bucket.
    /// Shared by the install query loop, dependency resolution and bucket
    /// scanning so the matching semantics cannot drift.
    pub fn matches_bucket_query(&self, query: &str) -> bool {
        let (query_bucket, query_name) = identity::split_bucket_query(query);
        let bucket_matched = query_bucket
            .as_deref()
            .is_none_or(|b| self.bucket().eq_ignore_ascii_case(b));
        self.name().eq_ignore_ascii_case(query_name) && bucket_matched
    }

    /// Get the bucket name of this package.
    ///
    /// # Note
    ///
    /// Although this method in some cases returns a bucket namer which can be
    /// the same as the bucket name from the install state of a package, it is
    /// not guaranteed to be.
    ///
    /// This method is not identical to `installed_bucket()`, which is designed
    /// to returns the precise installed bucket name if any.
    #[inline]
    pub fn bucket(&self) -> &str {
        self.bucket.as_str()
    }

    /// Get the version of this package.
    ///
    /// # Note
    ///
    /// Although this method in some cases returns a version number which can be
    /// the same as the version number from the installe state of a package, it
    /// is not guaranteed to be.
    ///
    /// This method is not identical to `installed_version()`, which is designed
    /// to returns the precise installed version number if any.
    #[inline]
    pub fn version(&self) -> &str {
        self.manifest.version()
    }

    /// Get the description of this package.
    #[inline]
    pub fn description(&self) -> Option<&str> {
        self.manifest.description()
    }

    /// Get the homepage of this package.
    #[inline]
    pub fn homepage(&self) -> &str {
        self.manifest.homepage()
    }

    /// Get the license of this package.
    pub fn license(&self) -> &License {
        self.manifest.license()
    }

    /// Get the cookie of this package.
    pub fn cookie(&self) -> Option<Vec<(&str, &str)>> {
        self.manifest.cookie().map(|c| {
            c.iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect::<Vec<_>>()
        })
    }

    /// Get the dependencies of this package.
    ///
    /// # Note
    ///
    /// There is no guarantee that whether a dependency is represented as a
    /// format of `bucket/name` or `name`.
    ///
    /// # Returns
    ///
    /// A list of dependencies of this package.
    pub fn dependencies(&self) -> Vec<String> {
        self.manifest.dependencies()
    }

    /// Get download urls of this package.
    ///
    /// # Note
    ///
    /// This method will return the actual download urls without the `#/dl.7z`
    /// fragment which is used to fake the file extension of the download urls.
    pub(crate) fn download_urls(&self) -> Vec<&str> {
        self.manifest
            .url()
            .into_iter()
            .map(|u| u.split_once('#').map(|s| s.0).unwrap_or(u))
            .collect::<Vec<_>>()
    }

    /// Get download urls of this package.
    pub(crate) fn download_filenames(&self) -> Vec<String> {
        self.manifest
            .url()
            .into_iter()
            .map(|u| {
                let mut hasher = crate::internal::hash::ChecksumBuilder::new()
                    .sha256()
                    .build();
                hasher.consume(u.as_bytes());
                let mut hash = hasher.finalize();
                hash.truncate(7);
                // Strip the query so `?download=1`-style URLs do not produce
                // cache filenames containing `?` (illegal on Windows). The
                // `#/rename.ext` Scoop fragment is kept — its extension is
                // what matters for archive detection.
                let path = PathBuf::from(internal::url::strip_url_query(u));
                let mut ext = path
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if !ext.is_empty() {
                    ext.insert(0, '.');
                }

                format!("{}#{}#{}{}", self.name(), self.version(), hash, ext)
            })
            .collect::<Vec<_>>()
    }

    pub(crate) fn download_hashes(&self) -> Vec<&HashString> {
        self.manifest.hash()
    }

    /// Get the installed bucket of this package.
    ///
    /// # Returns
    ///
    /// The installed bucket of this package, if any.
    pub fn installed_bucket(&self) -> Option<&str> {
        match self.install_state.get() {
            None => None,
            Some(state) => match state {
                InstallState::NotInstalled => None,
                InstallState::Installed(info) => {
                    Some(info.bucket().unwrap_or(ISOLATED_PACKAGE_BUCKET))
                }
            },
        }
    }

    /// Get the installed version of this package.
    ///
    /// # Returns
    ///
    /// The installed version of this package, if any.
    pub fn installed_version(&self) -> Option<&str> {
        match self.install_state.get() {
            None => None,
            Some(state) => match state {
                InstallState::NotInstalled => None,
                InstallState::Installed(info) => Some(info.version()),
            },
        }
    }

    /// Check if the package is held.
    ///
    /// # Note
    ///
    /// Only installed package can be held, therefore this method will always
    /// return `false` if the package is not installed.
    pub fn is_held(&self) -> bool {
        match self.install_state.get() {
            None => false,
            Some(state) => match state {
                InstallState::NotInstalled => false,
                InstallState::Installed(info) => info.held(),
            },
        }
    }

    /// Check if the package is installed.
    pub fn is_installed(&self) -> bool {
        self.installed_version().is_some()
    }

    #[inline]
    pub fn is_nightly(&self) -> bool {
        self.version() == "nightly"
    }

    /// The version used for the install layout (versioned dir, `$version`
    /// in scripts). Upstream rewrites `nightly` to `nightly-YYYYMMDD`
    /// (lib/install.ps1:21-25, `nightly_version`), giving each daily build
    /// its own versioned directory — hok mirrors that here.
    pub fn effective_version(&self) -> String {
        if self.is_nightly() {
            let now = time::OffsetDateTime::now_utc();
            format!(
                "nightly-{:04}{:02}{:02}",
                now.year(),
                u8::from(now.month()),
                now.day()
            )
        } else {
            self.version().to_owned()
        }
    }

    /// Check if the package is strictly installed, which means the package is
    /// installed from the bucket it belongs to rather than from other buckets.
    pub fn is_strictly_installed(&self) -> bool {
        match self.install_state.get() {
            None => false,
            Some(state) => match state {
                InstallState::NotInstalled => false,
                InstallState::Installed(info) => match info.bucket() {
                    Some(bucket) => bucket == self.bucket(),
                    None => false,
                },
            },
        }
    }

    /// Get the manifest of this package.
    ///
    /// # Returns
    ///
    /// The manifest reference of this package.
    #[inline]
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Get the upgradable version of this package.
    ///
    /// # Returns
    ///
    /// The upgradable version when the package is upgradable, otherwise `None`.
    pub fn upgradable_version(&self) -> Option<&str> {
        let origin_pkg = self.upgradable.get();

        if let Some(Some(pkg)) = origin_pkg {
            return Some(pkg.version());
        } else if let Some(installed_version) = self.installed_version() {
            let this_version = self.version();
            let is_upgradable = internal::compare_versions(this_version, installed_version)
                == std::cmp::Ordering::Greater;
            if is_upgradable {
                return Some(this_version);
            }
        }

        None
    }

    /// Check if this package is upgradable.
    ///
    /// # Returns
    ///
    /// The reference to the upgradable package of this package when it is
    /// upgradable, otherwise `None`.
    pub fn upgradable(&self) -> Option<&Package> {
        if let Some(Some(pkg)) = self.upgradable.get() {
            return Some(pkg.as_ref());
        }
        None
    }

    /// Get shims defined in this package.
    ///
    /// # Returns
    ///
    /// A list of shims defined in this package.
    pub fn shims(&self) -> Option<Vec<&str>> {
        self.manifest.shims()
    }

    pub fn supported_arch(&self) -> Vec<String> {
        let mut ret = vec![];
        if let Some(arch) = self.manifest.architecture() {
            if arch.ia32.is_some() {
                ret.push("ia32".to_string());
            }
            if arch.amd64.is_some() {
                ret.push("amd64".to_string());
            }
            if arch.aarch64.is_some() {
                ret.push("aarch64".to_string());
            }
        }
        ret
    }

    /// manifest.
    pub(crate) fn has_uninstall_script(&self) -> bool {
        [
            self.manifest
                .uninstaller()
                .map(|u| u.script())
                .unwrap_or_default(),
            self.manifest.pre_uninstall(),
            self.manifest.post_uninstall(),
        ]
        .into_iter()
        .any(|h| h.is_some())
    }

    pub(crate) fn fill_install_state(&self, state: InstallState) {
        let origin = match &state {
            InstallState::NotInstalled => OriginateFrom::Bucket(self.bucket.clone()),
            InstallState::Installed(info) => match info.url() {
                Some(url) => OriginateFrom::File(url.to_owned()),
                None => OriginateFrom::Bucket(
                    info.bucket().unwrap_or(ISOLATED_PACKAGE_BUCKET).to_owned(),
                ),
            },
        };

        let _ = self.origin.set(origin);
        let _ = self.install_state.set(state);
    }

    pub(crate) fn fill_upgradable(&self, upgradable: Package) {
        let upgradable = Some(Box::new(upgradable));
        let _ = self.upgradable.set(upgradable);
    }
}

impl PartialEq for Package {
    fn eq(&self, other: &Package) -> bool {
        self.name() == other.name()
    }
}

/// Hash mismatch context.
#[derive(Clone, Debug)]
pub struct HashMismatchContext {
    name: String,
    url: String,
    expected: String,
    actual: String,
}

impl fmt::Display for HashMismatchContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Hash mismatch for package '{}':\n     Url: {}\nExpected: {}\n  Actual: {}",
            self.name(),
            self.url(),
            self.expected(),
            self.actual()
        )
    }
}

impl HashMismatchContext {
    /// Create a new hash mismatch context.
    pub fn new(name: String, url: String, expected: String, actual: String) -> HashMismatchContext {
        HashMismatchContext {
            name,
            url,
            expected,
            actual,
        }
    }

    /// name of the package.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// url of corresponding hash mismatched file.
    pub fn url(&self) -> &str {
        self.url.as_str()
    }

    /// Expected hash.
    pub fn expected(&self) -> &str {
        self.expected.as_str()
    }

    /// Actual hash.
    pub fn actual(&self) -> &str {
        self.actual.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::manifest::Manifest;

    fn pkg(name: &str, version: &str) -> Package {
        let json = format!(
            r#"{{"version": "{}", "homepage": "https://example.com", "license": "MIT"}}"#,
            version
        );
        Package::from(name, "test", Manifest::from_json(name, &json).unwrap())
    }

    #[test]
    fn effective_version_rewrites_nightly_with_date() {
        // Upstream nightly_version(): nightly-YYYYMMDD
        // (lib/install.ps1:1-6) — each daily build gets its own dir.
        let pkg = pkg("nightly-app", "nightly");
        assert!(pkg.is_nightly());
        let v = pkg.effective_version();
        assert!(v.starts_with("nightly-"), "got {v}");
        let digits = &v["nightly-".len()..];
        assert_eq!(digits.len(), 8, "nightly-YYYYMMDD: {v}");
        assert!(digits.chars().all(|c| c.is_ascii_digit()), "{v}");
    }

    #[test]
    fn effective_version_passthrough_normal() {
        let pkg = pkg("app", "1.0.0");
        assert!(!pkg.is_nightly());
        assert_eq!(pkg.effective_version(), "1.0.0");
    }
}
