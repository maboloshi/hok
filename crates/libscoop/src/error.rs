//! Error types for the `libscoop` crate.
//!
//! Defines [`Error`] — an exhaustive, `thiserror`-derived enum covering
//! all fallible operations in the library. Also exports the [`Fallible`]
//! type alias (`Result<T, Error>`).
//!
//! # Design
//!
//! - **Thiserror**: All variants use `#[error("...")]` for human-readable
//!   messages and `#[from]` for automatic conversion from lower-level errors.
//! - **Non-exhaustive**: `Error` is `#[non_exhaustive]` so that adding new
//!   variants is not a breaking change for external consumers.
//! - **Context-rich variants**: Errors like `HashMismatch` and
//!   `ExtractionFailed` carry structured context structs rather than raw
//!   strings, enabling callers to inspect and format details.
//! - **Crate-internal helpers**: [`unknown_error()`] and [`box_error()`]
//!   are convenience constructors for wrapping arbitrary errors into the
//!   `Error` type (for cases where `#[from]` is insufficient).

use std::path::PathBuf;

use crate::{internal::dag::CyclicError, package::HashMismatchContext};

pub type Fallible<T> = Result<T, Error>;

/// Error that may occur during the lifetime of a [`Session`][1].
///
/// [1]: crate::Session
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Thrown when trying to add a bucket that already exists.
    #[error("bucket '{0}' already exists")]
    BucketAlreadyExists(String),

    /// Thrown when trying to add a bucket that is not a known bucket without
    /// specifying a remote url.
    #[error("'{0}' is not a known bucket, <repo> is required")]
    BucketAddRemoteRequired(String),

    /// Bucket not found error
    #[error("bucket '{0}' does not exist")]
    BucketNotFound(String),

    /// Thrown when a bucket update is rejected because the update is not
    /// a fast-forward (e.g. the remote branch was force-pushed).
    #[error("bucket ref update for '{0}' was rejected: not a fast-forward")]
    BucketUpdateNotFastForward(String),

    /// Thrown when trying to mutate config while it is in use.
    #[error("Could not alter config because it is in use.")]
    ConfigInUse,

    /// Invalid config key error
    #[error("invalid config key '{0}'")]
    ConfigKeyInvalid(String),

    /// Invalid config value error
    #[error("invalid config value '{0}'")]
    ConfigValueInvalid(String),

    /// Thrown when trying to set the user agent twice.
    #[error("User agent already set")]
    UserAgentAlreadySet,

    /// Hash mismatch error
    #[error("{0}")]
    HashMismatch(HashMismatchContext),

    /// Invalid cache file error
    #[error("error")]
    InvalidCacheFile { path: PathBuf },

    /// Throw when receiving an invalid answer from the frontend.
    #[error("invalid answer")]
    InvalidAnswer,

    /// Package not found error, this may occur when doing an explicit lookup
    /// for a package and no record with the given query was found.
    #[error("Could not find package named '{0}'")]
    PackageNotFound(String),

    /// Thrown when trying to do a cascading uninstall of a package that has
    /// a held dependency.
    #[error("Trying to cascade uninstall held package '{0}'")]
    PackageCascadeRemoveHold(String),

    /// Package dependent found error
    #[error("Found dependent(s):\n{}", .0.iter().map(|(d, p)| format!("'{}' requires '{}'", d, p)).collect::<Vec<_>>().join("\n"))]
    PackageDependentFound(Vec<(String, String)>),

    /// Thrown when there are multiple candidates for a package name.
    #[error("Found multiple candidates for package named '{0}'")]
    PackageMultipleCandidates(String),

    /// Thrown when trying to perform (un)hold operation on a package that is
    /// not installed.
    #[error("package '{0}' is not installed")]
    PackageHoldNotInstalled(String),

    /// Thrown when trying to perform (un)hold operation on a package of which
    /// the installation is broken.
    #[error("package '{0}' is broken")]
    PackageHoldBrokenInstall(String),

    /// A custom error.
    #[error("{0}")]
    Custom(String),

    /// Archive extraction failed.
    #[error("failed to extract archive: {0}")]
    ExtractionFailed(String),

    /// Path traversal detected in archive entry name.
    #[error("path traversal detected: {0}")]
    PathTraversalDetected(String),

    /// Cycle dependency error
    #[error(transparent)]
    CyclicDependency(#[from] CyclicError),

    /// Scoop hash error
    #[error(transparent)]
    Hash(#[from] scoop_hash::Error),

    /// Git error
    #[error(transparent)]
    Git(#[from] git2::Error),

    /// I/O error
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Regular expression error
    #[error(transparent)]
    Regex(#[from] regex::Error),

    /// SQLite error
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    /// Serde error
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

impl Error {
    /// Returns a structured i18n key for this error variant.
    ///
    /// The key can be used with `t!("error.xxx")` to look up a translated
    /// error message template. Dynamic parameters embedded in the variant
    /// must be injected separately.
    pub fn error_key(&self) -> &'static str {
        match self {
            Error::BucketAlreadyExists(_) => "error.bucket_already_exists",
            Error::BucketAddRemoteRequired(_) => "error.bucket_add_remote_required",
            Error::BucketNotFound(_) => "error.bucket_not_found",
            Error::BucketUpdateNotFastForward(_) => "error.bucket_update_not_fast_forward",
            Error::ConfigInUse => "error.config_in_use",
            Error::ConfigKeyInvalid(_) => "error.config_key_invalid",
            Error::ConfigValueInvalid(_) => "error.config_value_invalid",
            Error::UserAgentAlreadySet => "error.user_agent_already_set",
            Error::HashMismatch(_) => "error.hash_mismatch",
            Error::InvalidCacheFile { .. } => "error.invalid_cache_file",
            Error::InvalidAnswer => "error.invalid_answer",
            Error::PackageNotFound(_) => "error.package_not_found",
            Error::PackageCascadeRemoveHold(_) => "error.package_cascade_remove_hold",
            Error::PackageDependentFound(_) => "error.package_dependent_found",
            Error::PackageMultipleCandidates(_) => "error.package_multiple_candidates",
            Error::PackageHoldNotInstalled(_) => "error.package_hold_not_installed",
            Error::PackageHoldBrokenInstall(_) => "error.package_hold_broken_install",
            Error::Custom(_) => "error.custom",
            Error::ExtractionFailed(_) => "error.extraction_failed",
            Error::PathTraversalDetected(_) => "error.path_traversal_detected",
            Error::CyclicDependency(_) => "error.cyclic_dependency",
            Error::Hash(_) => "error.hash",
            Error::Git(_) => "error.git",
            Error::Io(_) => "error.io",
            Error::Regex(_) => "error.regex",
            Error::Sqlite(_) => "error.sqlite",
            Error::Serde(_) => "error.serde",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_key_bucket() {
        assert_eq!(
            Error::BucketAlreadyExists("main".into()).error_key(),
            "error.bucket_already_exists"
        );
        assert_eq!(
            Error::BucketNotFound("main".into()).error_key(),
            "error.bucket_not_found"
        );
        assert_eq!(
            Error::BucketAddRemoteRequired("main".into()).error_key(),
            "error.bucket_add_remote_required"
        );
        assert_eq!(
            Error::BucketUpdateNotFastForward("refs/heads/main".into()).error_key(),
            "error.bucket_update_not_fast_forward"
        );
    }

    #[test]
    fn test_error_key_config() {
        assert_eq!(Error::ConfigInUse.error_key(), "error.config_in_use");
        assert_eq!(
            Error::ConfigKeyInvalid("x".into()).error_key(),
            "error.config_key_invalid"
        );
        assert_eq!(
            Error::ConfigValueInvalid("x".into()).error_key(),
            "error.config_value_invalid"
        );
    }

    #[test]
    fn test_error_key_package() {
        assert_eq!(
            Error::PackageNotFound("7zip".into()).error_key(),
            "error.package_not_found"
        );
        assert_eq!(
            Error::PackageMultipleCandidates("7zip".into()).error_key(),
            "error.package_multiple_candidates"
        );
        assert_eq!(
            Error::PackageHoldNotInstalled("7zip".into()).error_key(),
            "error.package_hold_not_installed"
        );
    }

    #[test]
    fn test_error_key_technical() {
        assert_eq!(
            Error::ExtractionFailed("corrupt".into()).error_key(),
            "error.extraction_failed"
        );
        assert_eq!(
            Error::PathTraversalDetected("foo/../bar".into()).error_key(),
            "error.path_traversal_detected"
        );
        assert_eq!(
            Error::Custom("something".into()).error_key(),
            "error.custom"
        );
        assert_eq!(Error::InvalidAnswer.error_key(), "error.invalid_answer");
        assert_eq!(
            Error::UserAgentAlreadySet.error_key(),
            "error.user_agent_already_set"
        );
    }
}
