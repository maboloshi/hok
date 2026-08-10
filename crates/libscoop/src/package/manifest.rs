//! Scoop manifest parsing and data structures.
//!
//! A [manifest] is a JSON file that defines everything Scoop needs to
//! know about a package: version, download URLs, hashes, scripts,
//! dependencies, shortcuts, etc.
//!
//! # Design
//!
//! - **Schema-compatible**: Follows the [official Scoop manifest schema].
//!   Custom `Deserialize` implementations handle Scoop's flexible formats
//!   (e.g. a field that can be a single string or an array).
//! - **Two-layer structure**: [`Manifest`] wraps a [`ManifestSpec`] plus
//!   the file path and hash; `ManifestSpec` holds the actual JSON data.
//! - **Runtime architecture**: architecture-specific fields are selected at
//!   runtime from the host OS (see [`crate::internal::arch`]), mirroring
//!   Scoop's `Get-DefaultArchitecture`, not at compile time.
//! - **Hash support**: [`HashString`] represents a single hash value,
//!   knowing its algorithm (MD5, SHA1, SHA256, SHA512) from the string
//!   format. An empty hash (`""`) is a valid "no verification" value used
//!   by real-world Scoop manifests.
//!
//! [manifest]: https://github.com/ScoopInstaller/Scoop/wiki/App-Manifests
//! [official Scoop manifest schema]: https://github.com/ScoopInstaller/Scoop/blob/master/schema.json

use serde::ser::SerializeSeq;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use tracing::warn;

use crate::constant::{REGEX_ARCHIVE_7Z, REGEX_HASH};
use crate::error::Fallible;
use crate::internal;

#[path = "manifest_license.rs"]
mod manifest_license;
#[path = "manifest_parse.rs"]
mod manifest_parse;

pub use manifest_license::License;

/// A [`Manifest`] basically defines a package that is available to be installed
/// via Scoop. It's a JSON file containing all the specification needed by Scoop
/// to interact with, such as version, artifact urls and hashes, and scripts.
///
/// Following the [schema] of manifest, custom deserialzers have been implemented
/// to deserialize a Scoop manifest JSON file into a `Manifest` instance.
///
/// [schema]: https://github.com/ScoopInstaller/Scoop/blob/master/schema.json
/// [wiki]: https://github.com/ScoopInstaller/Scoop/wiki/App-Manifests
///
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    /// The path is used to determine the location of the manifest file.
    path: PathBuf,

    /// The actual manifest specification.
    inner: ManifestSpec,

    /// The hash of the manifest.
    hash: String,
}

/// [`ManifestSpec`] represents the actual data structure of a Scoop manifest.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ManifestSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<License>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub depends: Option<Vectorized<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub innosetup: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub cookie: Option<HashMap<String, String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub architecture: Option<Architecture>,

    /// Architecture-independent - `noarch` download url(s).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Vectorized<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<Vectorized<HashString>>,

    /// The `extract_dir` field is used to define the directory to which the
    /// archive should be extracted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract_dir: Option<Vectorized<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract_to: Option<Vectorized<String>>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::package::manifest::manifest_parse::deserialize_hook_script"
    )]
    pub pre_install: Option<Vectorized<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub installer: Option<Installer>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::package::manifest::manifest_parse::deserialize_hook_script"
    )]
    pub post_install: Option<Vectorized<String>>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::package::manifest::manifest_parse::deserialize_hook_script"
    )]
    pub pre_uninstall: Option<Vectorized<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub uninstaller: Option<Uninstaller>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::package::manifest::manifest_parse::deserialize_hook_script"
    )]
    pub post_uninstall: Option<Vectorized<String>>,

    /// The `bin` field is used to define binaries that need to be shimmed/added
    /// to the `shimes` directory.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::package::manifest::manifest_parse::deserialize_bin"
    )]
    pub bin: Option<Vectorized<Vectorized<String>>>,

    /// The `env_add_path` field is used to define path(s) that need to be added
    /// to the `PATH` environment variable during installation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_add_path: Option<Vectorized<String>>,

    /// The `env_set` field is used to define environment variables that should
    /// be set during installation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_set: Option<HashMap<String, String>>,

    /// The `shortcuts` field is used to define shortcuts that need to be created
    /// in the `Scoop Apps` directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shortcuts: Option<Vec<Vec<String>>>,

    /// The `persist` field is used to define files/directories that need to be
    /// persisted during uninstallation.
    #[serde(skip_serializing_if = "Option::is_none")]
    persist: Option<Vectorized<Vectorized<String>>>,

    /// The `psmodule` field is used to define PowerShell module that need to
    /// be imported during installation.
    #[serde(skip_serializing_if = "Option::is_none")]
    psmodule: Option<Psmodule>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggest: Option<HashMap<String, Vectorized<String>>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkver: Option<Checkver>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoupdate: Option<Autoupdate>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<Vectorized<String>>,
}

/// A [`Vectorized<T>`] represents a derivative [`Vec<T>`] data structure which
/// can be constructed from either an array of T **or a single T**. That means
/// when the input is a single T, it will also be deserialized to a vector of T
/// with the only T element.
///
/// Custom (De)srializers are implemented for this type to support the above
/// behavior.
///
/// There are some fields of a [`ManifestSpec`] using this type. In general,
/// when the type of value of a field is `stringOrArrayOfStrings` defined in
/// Scoop's manifest schema, it will be deserialized to a Vectorized\<String>.
/// To illustrate, `notes`, `pre_install` and `post_install` are these kind of
/// fields.
///
/// It is also used for the `stringOrArrayOfStringsOrAnArrayOfArrayOfStrings`,
/// a tow times wrapped vector of strings. `bin` and `persist` are these kind
/// of fields.
#[derive(Clone, Debug)]
pub struct Vectorized<T>(pub(crate) Vec<T>);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Architecture {
    /// Ia32 architecture specification.
    #[serde(rename = "32bit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ia32: Option<ArchitectureSpec>,

    /// Amd64 architecture specification.
    #[serde(rename = "64bit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amd64: Option<ArchitectureSpec>,

    /// Aarch64 architecture specification.
    #[serde(rename = "arm64")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aarch64: Option<ArchitectureSpec>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Installer {
    #[serde(skip_serializing_if = "Option::is_none")]
    args: Option<Vectorized<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    keep: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    script: Option<Vectorized<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Uninstaller {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vectorized<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<Vectorized<String>>,
}

/// PowerShell module information of a Scoop package.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Psmodule {
    name: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Sourceforge {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Checkver {
    #[serde(alias = "re")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regex: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    #[serde(alias = "jp")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jsonpath: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub xpath: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverse: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub replace: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub useragent: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<Vectorized<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub sourceforge: Option<Sourceforge>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Autoupdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub architecture: Option<AutoupdateArchitecture>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract_dir: Option<Vectorized<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<Vectorized<HashExtraction>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<Vectorized<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Vectorized<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ArchitectureSpec {
    /// Same as `ManifestSpec::bin`
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::package::manifest::manifest_parse::deserialize_bin"
    )]
    pub bin: Option<Vectorized<Vectorized<String>>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkver: Option<Checkver>,

    /// Same as `ManifestSpec::env_add_path`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_add_path: Option<Vectorized<String>>,

    /// Same as `ManifestSpec::env_set`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_set: Option<HashMap<String, String>>,

    /// Same as `ManifestSpec::extract_dir`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract_dir: Option<Vectorized<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<Vectorized<HashString>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub installer: Option<Installer>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::package::manifest::manifest_parse::deserialize_hook_script"
    )]
    pub post_install: Option<Vectorized<String>>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::package::manifest::manifest_parse::deserialize_hook_script"
    )]
    pub post_uninstall: Option<Vectorized<String>>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::package::manifest::manifest_parse::deserialize_hook_script"
    )]
    pub pre_install: Option<Vectorized<String>>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::package::manifest::manifest_parse::deserialize_hook_script"
    )]
    pub pre_uninstall: Option<Vectorized<String>>,

    /// Same as `ManifestSpec::shortcuts`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shortcuts: Option<Vec<Vec<String>>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub uninstaller: Option<Uninstaller>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Vectorized<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AutoupdateArchitecture {
    #[serde(rename = "32bit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ia32: Option<AutoupdateArchSpec>,
    #[serde(rename = "64bit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amd64: Option<AutoupdateArchSpec>,
    #[serde(rename = "arm64")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aarch64: Option<AutoupdateArchSpec>,
}

#[derive(Clone, Debug, Serialize)]
pub enum HashString {
    /// Empty hash — the manifest explicitly does not verify this URL.
    ///
    /// Scoop uses `""` in real-world manifests for URLs without a fixed
    /// checksum (e.g. `wget`'s `cacert.pem`); a missing hash only produces a
    /// warning during `check_hash` in the original implementation.
    #[serde(rename = "")]
    Empty,
    Md5(String),
    Sha1(String),
    Sha256(String),
    Sha512(String),
}

#[derive(Clone, Debug, Serialize)]
pub struct HashExtraction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub find: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub regex: Option<String>,

    #[serde(alias = "jp")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jsonpath: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub xpath: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<HashExtractionMode>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AutoupdateArchSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract_dir: Option<Vectorized<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<Vectorized<HashExtraction>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Vectorized<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum HashExtractionMode {
    #[serde(rename = "download")]
    Download,
    #[serde(rename = "extract")]
    Extract,
    #[serde(rename = "json")]
    Json,
    #[serde(rename = "xpath")]
    Xpath,
    #[serde(rename = "rdf")]
    Rdf,
    #[serde(rename = "metalink")]
    Metalink,
    #[serde(rename = "fosshub")]
    Fosshub,
    #[serde(rename = "sourceforge")]
    Sourceforge,
}

impl<T: Serialize> Serialize for Vectorized<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.0.len() {
            // Serialize an empty vector as `[]` (not `null`) to preserve the
            // JSON semantics of the manifest field.
            0 => {
                let seq = serializer.serialize_seq(Some(0))?;
                seq.end()
            }
            1 => serializer.serialize_some(&self.0[0]),
            _ => serializer.collect_seq(self.0.iter()),
        }
    }
}

/// Collapse a list of strings into a single JSON value the way Scoop
/// manifests do: one element becomes a bare string, multiple elements
/// become an array.
///
/// Shared by `checkver`/`checkhashes` when rewriting manifest fields
/// (url / hash) — mirrors [`Vectorized`]'s serialization semantics.
pub(crate) fn json_str_array(items: &[String]) -> serde_json::Value {
    if items.len() == 1 {
        serde_json::Value::String(items[0].clone())
    } else {
        serde_json::Value::Array(
            items
                .iter()
                .map(|s| serde_json::Value::String(s.clone()))
                .collect(),
        )
    }
}

////////////////////////////////////////////////////////////////////////////////
//  Implementations for types
////////////////////////////////////////////////////////////////////////////////

/// Macro to generate architecture-specific fields.
macro_rules! arch_specific_field {
    ($self:ident, $field:ident) => {{
        let mut ret = $self.inner.$field.as_ref();

        if let Some(arch) = $self.inner.architecture.as_ref() {
            // Architecture is selected at runtime from the host OS (Scoop's
            // `Get-DefaultArchitecture`, honouring the `default_architecture`
            // config override), plus the ARM64 fallback of Scoop's
            // `Get-SupportedArchitecture`: on ARM64 hosts whose manifest has
            // no `arm64` field, Windows 11 → 64bit, Windows 10 → 32bit.
            let current = crate::internal::arch::Arch::supported(
                crate::internal::arch::Arch::current(),
                arch.aarch64.is_none(),
            );
            match current {
                crate::internal::arch::Arch::Ia32 => {
                    if let Some(ia32) = &arch.ia32 {
                        let $field = ia32.$field.as_ref();
                        if $field.is_some() {
                            ret = $field;
                        }
                    }
                }
                crate::internal::arch::Arch::Amd64 => {
                    if let Some(amd64) = &arch.amd64 {
                        let $field = amd64.$field.as_ref();
                        if $field.is_some() {
                            ret = $field;
                        }
                    }
                }
                crate::internal::arch::Arch::Aarch64 => {
                    if let Some(aarch64) = &arch.aarch64 {
                        let $field = aarch64.$field.as_ref();
                        if $field.is_some() {
                            ret = $field;
                        }
                    }
                }
            }
        }
        ret
    }};
}

/// Generate an accessor method that reads a field via `arch_specific_field!`
/// and applies `devectorize()`.
macro_rules! arch_accessor {
    // Standard pattern — wrapped in Option
    ($(#[$doc:meta])* fn $name:ident() -> Option<$ret:ty>) => {
        $(#[$doc])*
        pub fn $name(&self) -> Option<$ret> {
            let ret = arch_specific_field!(self, $name);
            ret.map(|v| v.devectorize())
        }
    };
    // With `unwrap_or_default` — return type is not Option-wrapped
    ($(#[$doc:meta])* fn $name:ident() -> $ret:ty, unwrap_or_default) => {
        $(#[$doc])*
        pub fn $name(&self) -> $ret {
            let ret = arch_specific_field!(self, $name);
            ret.map(|v| v.devectorize()).unwrap_or_default()
        }
    };
}

/// Empty placeholder for manifests that omit the `license` field.
static EMPTY_LICENSE: std::sync::LazyLock<License> = std::sync::LazyLock::new(License::default);

/// Parse manifest JSON tolerantly, mirroring the tolerance of upstream
/// Scoop's PowerShell 7 `ConvertFrom-Json` (and `hok formatjson`):
///
/// - a leading UTF-8 BOM (`\u{FEFF}`) is stripped (notepad "UTF-8 with
///   BOM" manifests; upstream reads with `Get-Content -Encoding UTF8`)
/// - strict `serde_json` is tried first — the fast path for well-formed
///   bucket manifests, keeping the parse bottleneck fast — and JSON5
///   (comments, trailing commas, single-quoted strings) is used only as
///   a fallback for the rare hand-edited file
///
/// When both fail the strict error is returned: it points at the real
/// problem (e.g. a type mismatch), unlike a JSON5 parse error.
fn parse_manifest_spec(json: &str) -> Fallible<ManifestSpec> {
    let cleaned = json.trim_start_matches('\u{FEFF}');
    match serde_json::from_str::<ManifestSpec>(cleaned) {
        Ok(inner) => Ok(inner),
        Err(strict_err) => json5::from_str::<ManifestSpec>(cleaned)
            .map_err(|_| crate::Error::Custom(strict_err.to_string())),
    }
}

impl Manifest {
    /// Create a [`Manifest`] representation of a manfest JSON file with the
    /// given path.
    ///
    /// ## Errors
    ///
    /// If the process fails to read the file, this method will return a
    /// [`std::io::Error`].
    ///
    /// It returns a `serde_json::Error` when the JSON deserialization fails.
    pub fn parse<P: AsRef<Path>>(path: P) -> Fallible<Manifest> {
        let path = path.as_ref();

        // Read the entire manifest JSON file into memory firstly and then
        // deserialize it as this way is *a lot* faster than reading via
        // `serde_json::from_reader`.
        //
        // Discussion in https://github.com/serde-rs/json/issues/160
        let mut bytes = Vec::new();
        File::open(path)?.read_to_end(&mut bytes)?;

        // Parsing manifest files is the key bottleneck of the entire
        // project. We use `serde_json` because it's well documented and easy
        // to integrate. But I believe there should be an alternative to
        // `serde_json` which can parse JSON files much *faster*. Perhaps
        // `simd_json` can be the one. See https://github.com/serde-rs/json-benchmark
        let text = std::str::from_utf8(&bytes).map_err(|e| {
            crate::Error::Custom(format!(
                "failed to parse manifest {}: not valid UTF-8: {e}",
                path.display()
            ))
        })?;
        let inner: ManifestSpec = parse_manifest_spec(text).inspect_err(|e| {
            warn!("failed to parse manifest {} (err: {})", path.display(), e);
        })?;
        let path = internal::path::normalize_path(path);

        // SHA256 of the manifest file itself (kept for cache validation).
        let mut checksum = crate::internal::hash::ChecksumBuilder::new()
            .sha256()
            .build();
        checksum.consume(&bytes);
        let hash = checksum.finalize();

        Ok(Manifest { path, inner, hash })
    }

    /// Return the file path of this manifest.
    #[inline]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Create a Manifest from a JSON string, using the given name as identifier.
    ///
    /// This is used when loading manifests from the SQLite cache, where no
    /// file path is available.
    pub fn from_json(name: &str, json: &str) -> Fallible<Manifest> {
        let inner: ManifestSpec = parse_manifest_spec(json)?;
        let path = PathBuf::from(name);

        // SHA256 of the manifest JSON, consistent with `parse()`.
        let mut checksum = crate::internal::hash::ChecksumBuilder::new()
            .sha256()
            .build();
        checksum.consume(json.as_bytes());
        let hash = checksum.finalize();

        Ok(Manifest { path, inner, hash })
    }

    /// Return the `version` of this manifest.
    #[inline]
    pub fn version(&self) -> &str {
        // A manifest without `version` parses fine (upstream never
        // validates it); callers get an empty string and `formatjson`
        // warns about the missing field.
        self.inner.version.as_deref().unwrap_or("")
    }

    /// Return the `description` of this manifest.
    #[inline]
    pub fn description(&self) -> Option<&str> {
        self.inner.description.as_deref()
    }

    /// Return the `homepage` of this manifest.
    #[inline]
    pub fn homepage(&self) -> &str {
        self.inner.homepage.as_deref().unwrap_or("")
    }

    /// Return the `license` of this manifest.
    #[inline]
    pub fn license(&self) -> &License {
        self.inner.license.as_ref().unwrap_or(&EMPTY_LICENSE)
    }

    // #[inline]
    // pub fn manifest_hash(&self) -> &str {
    //     &self.hash
    // }

    /// Return the `depends` of this manifest.
    ///
    /// This method returns the explicit dependencies defined in the manifest,
    /// while [`dependencies`] returns all dependencies including the implicit
    /// ones.
    ///
    /// # Note
    ///
    /// The format of a value in the `depends` field can be either `name` or
    /// `bucket/name`, for example: `7zip` or `main/7zip`.
    ///
    /// [`dependencies`]: #method.dependencies
    #[inline]
    pub fn depends(&self) -> Option<Vec<&str>> {
        self.inner.depends.as_ref().map(|v| v.devectorize())
    }

    #[inline]
    pub fn architecture(&self) -> Option<&Architecture> {
        self.inner.architecture.as_ref()
    }

    arch_accessor! {
        /// Get `bin` field of this manifest.
        fn bin() -> Option<Vec<Vec<&str>>>
    }

    #[inline]
    pub fn checkver(&self) -> Option<&Checkver> {
        self.inner.checkver.as_ref()
    }

    /// Returns `autoupdate` defined in this manifest.
    #[inline]
    pub fn autoupdate(&self) -> Option<&Autoupdate> {
        self.inner.autoupdate.as_ref()
    }

    /// Returns `cookie` defined in this manifest.
    #[inline]
    pub fn cookie(&self) -> Option<&HashMap<String, String>> {
        self.inner.cookie.as_ref()
    }

    arch_accessor! {
        /// Returns `env_add_path` defined in this manifest.
        fn env_add_path() -> Option<Vec<&str>>
    }

    /// Returns `env_set` defined in this manifest.
    pub fn env_set(&self) -> Option<&HashMap<String, String>> {
        arch_specific_field!(self, env_set)
    }

    arch_accessor! {
        /// Returns `extract_dir` defined in this manifest.
        fn extract_dir() -> Option<Vec<&str>>
    }

    /// Returns `extract_to` defined in this manifest.
    #[inline]
    pub fn extract_to(&self) -> Option<Vec<&str>> {
        self.inner.extract_to.as_ref().map(|v| v.devectorize())
    }

    #[inline]
    pub fn innosetup(&self) -> bool {
        self.inner.innosetup.unwrap_or(false)
    }

    #[inline]
    pub fn suggest(&self) -> Option<&HashMap<String, Vectorized<String>>> {
        self.inner.suggest.as_ref()
    }

    arch_accessor! { fn pre_install() -> Option<Vec<&str>> }

    arch_accessor! { fn post_install() -> Option<Vec<&str>> }

    arch_accessor! { fn pre_uninstall() -> Option<Vec<&str>> }

    arch_accessor! { fn post_uninstall() -> Option<Vec<&str>> }

    /// Returns `notes` defined in this manifest.
    pub fn notes(&self) -> Option<Vec<&str>> {
        self.inner.notes.as_ref().map(|v| v.devectorize())
    }

    pub fn installer(&self) -> Option<&Installer> {
        arch_specific_field!(self, installer)
    }

    pub fn uninstaller(&self) -> Option<&Uninstaller> {
        arch_specific_field!(self, uninstaller)
    }

    /// Returns `persist` defined in this manifest.
    #[inline]
    pub fn persist(&self) -> Option<Vec<Vec<&str>>> {
        self.inner.persist.as_ref().map(|v| v.devectorize())
    }

    /// Returns `psmodule` defined in this manifest.
    #[inline]
    pub fn psmodule(&self) -> Option<&Psmodule> {
        self.inner.psmodule.as_ref()
    }

    pub fn shortcuts(&self) -> Option<Vec<Vec<&str>>> {
        let ret = arch_specific_field!(self, shortcuts);
        ret.map(|v| {
            v.iter()
                .map(|v| v.iter().map(|s| s.as_str()).collect())
                .collect()
        })
    }

    arch_accessor! {
        /// Extract download urls from this manifest:
        ///
        /// - For `amd64` return "64bit" urls if available else noarch urls;
        /// - For `ia32` return "32bit" urls if available else noarch urls;
        /// - For `aarch64` return "arm64" urls if available else noarch urls.
        fn url() -> Vec<&str>, unwrap_or_default
    }

    /// Collect ALL URLs from this manifest (noarch + all architectures).
    ///
    /// Unlike `url()` which only returns URLs for the current platform,
    /// this method returns URLs from noarch, 64bit, 32bit, and arm64 — in
    /// the same fixed order as Scoop's `bin/checkhashes.ps1`
    /// (64bit → 32bit → arm64). Used by checkurls to validate all download
    /// URLs.
    pub fn all_urls(&self) -> Vec<&str> {
        let mut urls: Vec<&str> = Vec::new();
        // Add noarch URLs
        if let Some(ref u) = self.inner.url {
            for s in u.devectorize() {
                urls.push(s);
            }
        }
        // Add architecture-specific URLs (64bit → 32bit → arm64)
        if let Some(ref arch) = self.inner.architecture {
            for spec in [&arch.amd64, &arch.ia32, &arch.aarch64]
                .into_iter()
                .flatten()
            {
                if let Some(ref u) = spec.url {
                    for s in u.devectorize() {
                        urls.push(s);
                    }
                }
            }
        }
        urls
    }

    /// Collect ALL hashes from this manifest (noarch + all architectures).
    ///
    /// Unlike `hash()` which only returns hashes for the current platform,
    /// this method returns hashes from noarch, 64bit, 32bit, and arm64 — in
    /// the same fixed order as Scoop's `bin/checkhashes.ps1`
    /// (64bit → 32bit → arm64). Used by checkhashes to validate all hashes.
    pub fn all_hashes(&self) -> Vec<&HashString> {
        let mut hashes: Vec<&HashString> = Vec::new();
        // Add noarch hashes
        if let Some(ref h) = self.inner.hash {
            for s in h.devectorize() {
                hashes.push(s);
            }
        }
        // Add architecture-specific hashes (64bit → 32bit → arm64)
        if let Some(ref arch) = self.inner.architecture {
            for spec in [&arch.amd64, &arch.ia32, &arch.aarch64]
                .into_iter()
                .flatten()
            {
                if let Some(ref h) = spec.hash {
                    for s in h.devectorize() {
                        hashes.push(s);
                    }
                }
            }
        }
        hashes
    }

    /// Collect the JSON pointer paths and hash counts for all hash segments,
    /// in the same order as `all_hashes()` / `all_urls()`.
    ///
    /// Each entry is `(json_pointer, count)`, e.g.:
    /// `("/hash", 2)` for two top-level hashes,
    /// `("/architecture/64bit/hash", 1)` for one 64bit hash.
    ///
    /// Architecture segments are emitted 64bit → 32bit → arm64, matching
    /// Scoop's `bin/checkhashes.ps1`.
    pub fn all_hash_segments(&self) -> Vec<(String, usize)> {
        let mut segments = Vec::new();
        // Top-level hashes
        if let Some(ref h) = self.inner.hash {
            let count = h.devectorize().len();
            if count > 0 {
                segments.push(("/hash".to_string(), count));
            }
        }
        // Architecture-specific hashes (64bit → 32bit → arm64)
        if let Some(ref arch) = self.inner.architecture {
            for (arch_name, spec_opt) in [
                ("64bit", &arch.amd64),
                ("32bit", &arch.ia32),
                ("arm64", &arch.aarch64),
            ] {
                if let Some(spec) = spec_opt {
                    if let Some(ref h) = spec.hash {
                        let count = h.devectorize().len();
                        if count > 0 {
                            segments.push((format!("/architecture/{arch_name}/hash"), count));
                        }
                    }
                }
            }
        }
        segments
    }

    arch_accessor! {
        /// Extract file hashes from this manifest, in following order:
        ///
        /// - For `amd64` return "64bit" hashes if available else noarch hashes;
        /// - For `ia32` return "32bit" hashes if available else noarch hashes;
        /// - For `aarch64` return "arm64" hashes if available else noarch hashes.
        fn hash() -> Vec<&HashString>, unwrap_or_default
    }

    /// Returns the dependencies of this manifest.
    ///
    /// This method returns all dependencies including the implicit ones, while
    /// [`depends`] returns the explicit dependencies defined in the `depends`
    /// field of the manifest.
    ///
    /// # Note
    ///
    /// The format of the value of a dependency can be either `name` or
    /// `bucket/name`, for example: `7zip` or `main/7zip`.
    ///
    /// [`depends`]: #method.depends
    pub(crate) fn dependencies(&self) -> Vec<String> {
        let mut deps = HashSet::new();

        if let Some(raw_depends) = self.depends() {
            deps.extend(raw_depends.into_iter().map(|s| s.to_owned()));
        }

        // Implicit "installation helpers", mirroring Scoop's
        // `Get-InstallationHelper` (lib/depends.ps1): helpers are *appended*
        // to the declared dependencies and deduplicated — existing
        // declarations are never removed. Helper names carry no bucket
        // prefix, matching the original implementation.
        //
        // Known differences from Scoop, noted for future alignment:
        // - Scoop gates `7zip` on config `USE_EXTERNAL_7ZIP` and `lessmsi` on
        //   `USE_LESSMSI` (default off); Hok has no such config yet, so
        //   `lessmsi` is always appended when triggered.
        // - The archive-URL check is an approximation of Scoop's
        //   `Test-7zipRequirement` pattern.
        let urls = self.url();
        let hook_scripts = [
            self.pre_install(),
            self.post_install(),
            self.installer().map(|i| i.script()).unwrap_or_default(),
        ];
        let script = hook_scripts
            .into_iter()
            .flatten() // Option<Vec<&str>> → Vec<&str>
            .flatten() // Vec<&str> → &str
            .collect::<Vec<_>>()
            .join("\r\n");

        let url_is_archive = urls.iter().any(|u| REGEX_ARCHIVE_7Z.is_match(u));
        let url_is_msi = urls
            .iter()
            .any(|u| u.to_ascii_lowercase().ends_with(".msi"));

        if url_is_archive || script.contains("Expand-7zipArchive") {
            deps.insert("7zip".to_owned());
        }
        if url_is_msi || script.contains("Expand-MsiArchive") {
            deps.insert("lessmsi".to_owned());
        }
        if self.innosetup() || script.contains("Expand-InnoArchive") {
            deps.insert("innounp".to_owned());
        }
        if script.contains("Expand-DarkArchive") {
            deps.insert("dark".to_owned());
        }

        deps.into_iter().collect()
    }

    /// Get shims defined in this manifest.
    ///
    /// # Note
    ///
    /// While [`bin()`][1] method returns the raw `bin` field of the manifest,
    /// this method returns the shim names defined in the `bin` field.
    ///
    /// [1]: #method.bin
    pub(crate) fn shims(&self) -> Option<Vec<&str>> {
        if let Some(shim_defs) = self.bin() {
            let mut shims = Vec::with_capacity(shim_defs.len());
            for def in shim_defs {
                match def.len() {
                    0 => {
                        warn!(
                            "invalid shim definition found in manifest {}",
                            self.path().display()
                        );
                        continue;
                    }
                    1 => shims.push(def[0]),
                    _ => shims.push(def[1]),
                }
            }
            Some(shims)
        } else {
            None
        }
    }
}

impl HashString {
    /// Create a [`HashString`] representation.
    pub fn new(raw: &str) -> Fallible<HashString> {
        // An empty hash means "no verification" in Scoop (see `check_hash` in
        // lib/download.ps1 — a missing hash only warns). Real-world manifests
        // (e.g. ScoopInstaller/Main `wget.json`) use `""` for URLs without a
        // fixed checksum, so accept it instead of failing to parse the whole
        // manifest.
        if raw.is_empty() {
            return Ok(HashString::Empty);
        }

        if !REGEX_HASH.is_match(raw) {
            let msg = format!("invalid hash string: {}", raw);
            return Err(crate::Error::Custom(msg));
        }

        let (algo, hash) = raw.split_once(':').unwrap_or(("sha256", raw));
        let hash = hash.to_lowercase();
        match algo {
            "md5" => Ok(HashString::Md5(hash)),
            "sha1" => Ok(HashString::Sha1(hash)),
            "sha256" => Ok(HashString::Sha256(hash)),
            "sha512" => Ok(HashString::Sha512(hash)),
            _ => Err(crate::Error::Custom(format!(
                "unsupported hash algorithm: {}",
                algo
            ))),
        }
    }

    /// Return the hash algorithm.
    ///
    /// # Returns
    ///
    /// - `md5`
    /// - `sha1`
    /// - `sha256`
    /// - `sha512`
    ///
    /// For [`HashString::Empty`] (no verification) the value is unused;
    /// `sha256` is returned as a conservative default.
    pub fn algorithm(&self) -> &str {
        match self {
            HashString::Empty => "sha256",
            HashString::Md5(_) => "md5",
            HashString::Sha1(_) => "sha1",
            HashString::Sha256(_) => "sha256",
            HashString::Sha512(_) => "sha512",
        }
    }

    /// Return the hash value.
    ///
    /// For [`HashString::Empty`] this returns an empty string, so callers can
    /// uniformly treat an empty value as "skip verification".
    pub fn value(&self) -> &str {
        match self {
            HashString::Empty => "",
            HashString::Md5(s) => s,
            HashString::Sha1(s) => s,
            HashString::Sha256(s) => s,
            HashString::Sha512(s) => s,
        }
    }
}

impl fmt::Display for HashString {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            HashString::Empty => String::new(),
            HashString::Md5(s) => format!("md5:{}", s),
            HashString::Sha1(s) => format!("sha1:{}", s),
            HashString::Sha256(s) => format!("sha256:{}", s),
            HashString::Sha512(s) => format!("sha512:{}", s),
        };

        write!(f, "{}", s)
    }
}

impl Installer {
    #[inline]
    pub fn args(&self) -> Option<Vec<&str>> {
        self.args.as_ref().map(|v| v.devectorize())
    }

    #[inline]
    pub fn file(&self) -> Option<&str> {
        self.file.as_deref()
    }

    #[inline]
    pub fn script(&self) -> Option<Vec<&str>> {
        self.script.as_ref().map(|v| v.devectorize())
    }

    #[inline]
    pub fn keep(&self) -> bool {
        self.keep.unwrap_or(false)
    }
}

impl Psmodule {
    /// Return the `name` of the PowerShell module.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Uninstaller {
    #[inline]
    pub fn args(&self) -> Option<Vec<&str>> {
        self.args.as_ref().map(|v| v.devectorize())
    }

    #[inline]
    pub fn file(&self) -> Option<&str> {
        self.file.as_deref()
    }

    #[inline]
    pub fn keep(&self) -> bool {
        self.keep.unwrap_or(false)
    }

    #[inline]
    pub fn script(&self) -> Option<Vec<&str>> {
        self.script.as_ref().map(|v| v.devectorize())
    }
}

impl Vectorized<HashString> {
    pub fn devectorize(&self) -> Vec<&HashString> {
        self.0.iter().collect()
    }
}

impl Vectorized<String> {
    pub fn devectorize(&self) -> Vec<&str> {
        self.0.iter().map(|s| s.as_str()).collect()
    }
}

impl Vectorized<Vectorized<String>> {
    pub fn devectorize(&self) -> Vec<Vec<&str>> {
        self.0.iter().map(|v| v.devectorize()).collect()
    }
}

impl From<Vectorized<String>> for Vec<String> {
    fn from(veced: Vectorized<String>) -> Self {
        veced.0
    }
}

impl From<Vectorized<Vectorized<String>>> for Vec<Vec<String>> {
    fn from(veced: Vectorized<Vectorized<String>>) -> Self {
        veced.0.into_iter().map(|v| v.0).collect()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InstallInfo {
    architecture: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    bucket: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

impl InstallInfo {
    pub fn parse<P: AsRef<Path>>(path: P) -> Fallible<InstallInfo> {
        let path = path.as_ref();
        let mut bytes = Vec::new();
        File::open(path)?.read_to_end(&mut bytes)?;

        let info = serde_json::from_slice(&bytes).inspect_err(|e| {
            warn!(
                "failed to parse install_info {} (err: {})",
                path.display(),
                e
            );
        })?;

        Ok(info)
    }

    #[inline]
    pub fn bucket(&self) -> Option<&str> {
        self.bucket.as_deref()
    }

    #[inline]
    pub fn arch(&self) -> &str {
        &self.architecture
    }

    #[inline]
    pub fn is_held(&self) -> bool {
        self.hold.unwrap_or(false)
    }

    #[inline]
    pub fn set_held(&mut self, flag: bool) {
        self.hold = Some(flag);
    }

    #[inline]
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_from(json: &str) -> Manifest {
        Manifest::from_json("test-pkg", json).unwrap()
    }

    // ── json_str_array ──────────────────────────────────────────────────────

    #[test]
    fn json_str_array_single_element_is_bare_string() {
        let v = json_str_array(&["one".to_string()]);
        assert_eq!(v, serde_json::Value::String("one".to_string()));
    }

    #[test]
    fn json_str_array_multiple_elements_is_array() {
        let v = json_str_array(&["a".to_string(), "b".to_string()]);
        assert_eq!(v, serde_json::json!(["a", "b"]));
    }

    #[test]
    fn json_str_array_empty_is_empty_array() {
        let v = json_str_array(&[]);
        assert_eq!(v, serde_json::json!([]));
    }

    #[test]
    fn test_hashstring_empty_is_valid() {
        let empty = HashString::new("").unwrap();
        assert!(matches!(empty, HashString::Empty));
        assert_eq!(empty.value(), "");
        assert_eq!(empty.to_string(), "");
        // An empty hash in a manifest parses fine (wget-style "no verification")
        let m = manifest_from(
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "url": "https://example.com/pkg.zip",
                "hash": ""
            }"#,
        );
        assert_eq!(m.hash().len(), 1);
        assert!(m.hash()[0].value().is_empty());
    }

    #[test]
    fn test_hashstring_accepts_official_combination_lengths() {
        // Official schema `hashPattern` allows any algorithm prefix with any
        // digest length (32/40/64/128) — not bound per algorithm.
        let md5_long = format!("md5:{}", "a".repeat(64));
        assert!(matches!(
            HashString::new(&md5_long).unwrap(),
            HashString::Md5(_)
        ));
        let sha1_short = format!("sha1:{}", "b".repeat(32));
        assert!(matches!(
            HashString::new(&sha1_short).unwrap(),
            HashString::Sha1(_)
        ));
        let sha512_40 = format!("sha512:{}", "c".repeat(40));
        assert!(matches!(
            HashString::new(&sha512_40).unwrap(),
            HashString::Sha512(_)
        ));
        // Bare 64-hex stays sha256
        let bare = "d".repeat(64);
        assert!(matches!(
            HashString::new(&bare).unwrap(),
            HashString::Sha256(_)
        ));
        // Invalid digests are still rejected
        assert!(HashString::new("md5:xyz").is_err());
        assert!(HashString::new("sha1:").is_err());
    }


    // ── hook script fields (deserialize_hook_script) ────────────────────────

    #[test]
    fn test_hook_script_string_form_parses() {
        let m = manifest_from(
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "url": "https://example.com/pkg.zip",
                "post_install": "echo done"
            }"#,
        );
        assert_eq!(m.post_install(), Some(vec!["echo done"]));
    }

    #[test]
    fn test_hook_script_array_form_parses() {
        let m = manifest_from(
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "url": "https://example.com/pkg.zip",
                "pre_install": ["a", "b"]
            }"#,
        );
        assert_eq!(m.pre_install(), Some(vec!["a", "b"]));
    }

    #[test]
    fn test_hook_script_object_form_parses() {
        // tim: `"post_install": {"script": [...]}` — a non-schema object form
        // that official checkver (ConvertFrom-Json, no schema validation)
        // tolerates; hok must normalize it instead of rejecting the whole
        // manifest at parse time.
        let m = manifest_from(
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "url": "https://example.com/pkg.zip",
                "post_install": { "script": ["a", "b"] }
            }"#,
        );
        assert_eq!(m.post_install(), Some(vec!["a", "b"]));
    }

    #[test]
    fn test_checkver_sourceforge_object_without_path_parses() {
        // Official schema allows `checkver.sourceforge: {"project": ...}`
        // without `path` (both optional); hok must not require `path`.
        let m = manifest_from(
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "url": "https://example.com/pkg.zip",
                "checkver": { "sourceforge": { "project": "foo" } }
            }"#,
        );
        let cv = m.checkver().expect("checkver");
        let sf = cv.sourceforge.as_ref().expect("sourceforge");
        assert_eq!(sf.project.as_deref(), Some("foo"));
        assert_eq!(sf.path, None);
    }

    #[test]
    fn test_checkver_sourceforge_string_form_parses() {
        // Official checkver.ps1 regex `(?<project>[\w-]*)(/(?<path>.*))?`:
        // "foo/bar" → project=foo, path=bar; "fooproj" (no '/') → project
        // only, path absent.
        let m = manifest_from(
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "url": "https://example.com/pkg.zip",
                "checkver": { "sourceforge": "foo/bar" }
            }"#,
        );
        let cv = m.checkver().expect("checkver");
        let sf = cv.sourceforge.as_ref().expect("sourceforge");
        assert_eq!(sf.project.as_deref(), Some("foo"));
        assert_eq!(sf.path.as_deref(), Some("bar"));

        let m2 = manifest_from(
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "url": "https://example.com/pkg.zip",
                "checkver": { "sourceforge": "fooproj" }
            }"#,
        );
        let cv2 = m2.checkver().expect("checkver");
        let sf2 = cv2.sourceforge.as_ref().expect("sourceforge");
        assert_eq!(sf2.project.as_deref(), Some("fooproj"));
        assert_eq!(sf2.path, None);
    }

    #[test]
    fn test_dependencies_preserve_explicit_and_append_helpers() {
        // Explicit `main/7zip` must be preserved (never removed), and the
        // script trigger appends the bucket-less `7zip` helper.
        let m = manifest_from(
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "depends": ["main/7zip"],
                "url": "https://example.com/pkg.exe",
                "hash": "9f67fe001e008b1419b442818ce48746e0e20c8cb28977cc7cbc04d774f20b8a",
                "post_install": "Expand-7zipArchive foo.7z $dir"
            }"#,
        );
        let deps = m.dependencies();
        assert!(
            deps.contains(&"main/7zip".to_owned()),
            "explicit dep preserved"
        );
        assert!(
            deps.contains(&"7zip".to_owned()),
            "helper appended bucket-less"
        );
        // Deduplication: exactly one `7zip`
        assert_eq!(deps.iter().filter(|d| *d == "7zip").count(), 1);
    }

    #[test]
    fn test_dependencies_archive_url_triggers_7zip() {
        // An archive URL alone triggers the 7zip helper, even without scripts.
        let m = manifest_from(
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "url": "https://example.com/pkg.7z",
                "hash": "a927ce340e91aea1f1dcb86937bcd6cfadfd550986b1bcc8ae2edbe23844277a"
            }"#,
        );
        assert!(m.dependencies().contains(&"7zip".to_owned()));
    }

    #[test]
    fn test_dependencies_msi_url_triggers_lessmsi() {
        let m = manifest_from(
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "url": "https://example.com/pkg.msi",
                "hash": "297dfcca1435c5e6b31e3e1eedac0c61f1f24f9d27d9795e9d9e586378e12f94"
            }"#,
        );
        assert!(m.dependencies().contains(&"lessmsi".to_owned()));
    }

    #[test]
    fn test_dependencies_innosetup_and_dark() {
        // innosetup: true triggers `innounp`; script triggers `dark`.
        let m = manifest_from(
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "innosetup": true,
                "url": "https://example.com/pkg.exe",
                "hash": "d26b6c2b94cc18657c3327b6bbab07a2d8c43a7dbd51ba2f2996b6157f0b26a2",
                "pre_install": "Expand-DarkArchive appx $dir"
            }"#,
        );
        let deps = m.dependencies();
        assert!(
            deps.contains(&"innounp".to_owned()),
            "innosetup → innounp (bucket-less)"
        );
        assert!(
            deps.contains(&"dark".to_owned()),
            "Expand-DarkArchive → dark"
        );
        assert!(
            !deps.iter().any(|d| d.starts_with("main/")),
            "helpers carry no bucket prefix"
        );
    }

    #[test]
    fn test_dependencies_no_false_positives() {
        // Plain .exe URL and no helper scripts → no implicit helpers.
        let m = manifest_from(
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "url": "https://example.com/pkg.exe",
                "hash": "9ae8b12d46852aa08a97d46767d3aca42271b9307c25ec2d4f27c34536244e27"
            }"#,
        );
        assert!(m.dependencies().is_empty());
    }

    #[test]
    fn missing_required_fields_are_tolerated() {
        // Upstream never validates version/homepage/license; a manifest
        // missing them must parse instead of failing the whole manifest.
        let m = manifest_from(
            r#"{
                "url": "https://example.com/pkg.zip",
                "hash": "9f67fe001e008b1419b442818ce48746e0e20c8cb28977cc7cbc04d774f20b8a"
            }"#,
        );
        assert_eq!(m.version(), "");
        assert_eq!(m.homepage(), "");
        assert_eq!(m.license().identifier(), "");
        assert_eq!(m.license().url(), None);
    }

    #[test]
    fn autoupdate_hash_string_form_parses() {
        // `autoupdate.hash` may be a plain string (e.g. "mode:json") —
        // upstream's HashHelper accepts objects, strings and arrays.
        let m = manifest_from(
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "autoupdate": {
                    "url": "https://example.com/$version.zip",
                    "hash": "mode:json"
                }
            }"#,
        );
        // Parsing succeeded is the assertion.
        assert_eq!(m.version(), "1.0.0");
    }

    #[test]
    fn bin_object_form_normalizes_to_tuple() {
        // Third-party object form `{"file", "name", "args"}` normalizes to
        // the (file, name, args...) tuple the shim layer understands.
        let m = manifest_from(
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "bin": [{"file": "foo.exe", "name": "bar", "args": ["-x", "--y"]}]
            }"#,
        );
        let bins = m.bin().unwrap();
        assert_eq!(bins.len(), 1);
        assert_eq!(bins[0], vec!["foo.exe", "bar", "-x", "--y"]);
    }

    #[test]
    fn bin_invalid_object_dropped_not_fatal() {
        // A `bin` item that cannot be understood (object without `file`)
        // is dropped instead of invalidating the whole manifest.
        let m = manifest_from(
            r#"{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "bin": [{"bogus": 1}, "good.exe"]
            }"#,
        );
        let bins = m.bin().unwrap();
        assert_eq!(bins.len(), 1);
        assert_eq!(bins[0], vec!["good.exe"]);
    }

    #[test]
    fn bom_prefixed_manifest_parses() {
        // Notepad-style "UTF-8 with BOM" manifests (upstream reads with
        // `Get-Content -Encoding UTF8`, stripping the BOM).
        let m = manifest_from(&format!(
            "\u{FEFF}{}",
            r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#
        ));
        assert_eq!(m.version(), "1.0.0");
    }

    #[test]
    fn json5_manifest_parses_as_fallback() {
        // Comments and trailing commas (hand-edited manifests) parse via
        // the JSON5 fallback, like PowerShell 7's ConvertFrom-Json.
        let m = manifest_from(
            r#"{
                // a hand-edited comment
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "url": ["https://example.com/a.zip", "https://example.com/b.zip",],
            }"#,
        );
        assert_eq!(m.version(), "1.0.0");
        assert_eq!(m.url().len(), 2);
    }
}
