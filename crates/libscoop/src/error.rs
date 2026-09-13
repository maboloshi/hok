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

    /// Thrown when a bucket update is rejected because the update cannot be
    /// integrated: the histories have diverged and cannot be merged, or the
    /// branches are unrelated (e.g. the remote branch was rewritten).
    #[error("bucket ref update for '{0}' was rejected: not a fast-forward")]
    BucketUpdateNotFastForward(String),

    /// Thrown when a bucket update fails because the 3-way merge between the
    /// local branch and the fetched remote produced conflicts. Conflict
    /// markers are left in the working tree for the user to resolve.
    #[error("merge conflict while updating bucket '{0}'; resolve the conflicts and retry")]
    BucketUpdateMergeConflict(String),

    /// Thrown when a bucket update is rejected because it would overwrite
    /// local changes in the listed files (matching `git pull`).
    #[error("local changes would be overwritten by the bucket update: {0}")]
    BucketUpdateLocalChanges(String),

    /// Thrown when trying to mutate config while it is in use.
    #[error("Could not alter config because it is in use.")]
    ConfigInUse,

    /// Invalid config key error
    #[error("invalid config key '{0}'")]
    ConfigKeyInvalid(String),

    /// An upstream Scoop setting hok has no equivalent for.
    #[error("config key '{0}' is not supported by hok")]
    ConfigKeyUnsupported(String),

    /// Invalid config value error
    #[error("invalid config value '{0}'")]
    ConfigValueInvalid(String),

    /// Thrown when adding an alias whose name is already configured.
    #[error("Alias '{0}' already exists.")]
    AliasAlreadyExists(String),

    /// Thrown when removing an alias that is not configured.
    #[error("Alias '{0}' doesn't exist.")]
    AliasNotFound(String),

    /// Thrown when adding an alias whose script file is already present in the
    /// shims directory (upstream `add_alias`'s second guard).
    #[error("File '{0}' already exists in shims directory.")]
    AliasScriptExists(String),

    /// Thrown when an alias name cannot be used as a `scoop-<name>.ps1` script
    /// name (empty, whitespace, or a path separator / reserved filename char).
    #[error("invalid alias name '{0}'")]
    AliasInvalidName(String),

    /// Thrown when an alias name collides with a built-in command: clap
    /// resolves built-ins before aliases, so such an alias is unreachable.
    #[error("Alias '{0}' conflicts with a built-in command.")]
    AliasReservedName(String),

    /// Thrown when trying to set the user agent twice.
    #[error("User agent already set")]
    UserAgentAlreadySet,

    /// Thrown when trying to set the output sink twice.
    #[error("Output sink already set")]
    OutputAlreadySet,

    /// Thrown when trying to inject an infra implementation (network / git /
    /// archive) into a session more than once.
    #[error("Infra implementation already set")]
    InfraAlreadySet,

    /// Thrown when trying to inject a cancellation token into a session more
    /// than once.
    #[error("Cancellation token already set")]
    CancellationTokenAlreadySet,

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

    /// Thrown when `shim add` names a target that does not exist. Upstream
    /// refuses to create such a shim and exits 3
    /// (`libexec/scoop-shim.ps1:114-123`); the CLI maps this variant to that
    /// code.
    #[error("command path does not exist: {0}")]
    ShimTargetMissing(String),

    /// Thrown when the user aborts an interactive confirmation prompt.
    /// Not a failure: callers exit 0 but skip success messages.
    #[error("operation aborted by user")]
    UserAborted,

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
    Hash(#[from] crate::internal::hash::HashError),

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
            Error::BucketUpdateMergeConflict(_) => "error.bucket_update_merge_conflict",
            Error::BucketUpdateLocalChanges(_) => "error.bucket_update_local_changes",
            Error::ConfigInUse => "error.config_in_use",
            Error::ConfigKeyInvalid(_) => "error.config_key_invalid",
            Error::ConfigKeyUnsupported(_) => "error.config_key_unsupported",
            Error::ConfigValueInvalid(_) => "error.config_value_invalid",
            Error::AliasAlreadyExists(_) => "error.alias_already_exists",
            Error::AliasNotFound(_) => "error.alias_not_found",
            Error::AliasScriptExists(_) => "error.alias_script_exists",
            Error::AliasInvalidName(_) => "error.alias_invalid_name",
            Error::AliasReservedName(_) => "error.alias_reserved_name",
            Error::UserAgentAlreadySet => "error.user_agent_already_set",
            Error::OutputAlreadySet => "error.output_already_set",
            Error::InfraAlreadySet => "error.infra_already_set",
            Error::CancellationTokenAlreadySet => "error.cancellation_token_already_set",
            Error::HashMismatch(_) => "error.hash_mismatch",
            Error::InvalidCacheFile { .. } => "error.invalid_cache_file",
            Error::InvalidAnswer => "error.invalid_answer",
            Error::PackageNotFound(_) => "error.package_not_found",
            Error::PackageCascadeRemoveHold(_) => "error.package_cascade_remove_hold",
            Error::PackageDependentFound(_) => "error.package_dependent_found",
            Error::PackageMultipleCandidates(_) => "error.package_multiple_candidates",
            Error::PackageHoldNotInstalled(_) => "error.package_hold_not_installed",
            Error::PackageHoldBrokenInstall(_) => "error.package_hold_broken_install",
            Error::ShimTargetMissing(_) => "error.shim_target_missing",
            Error::UserAborted => "error.user_aborted",
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
        assert_eq!(
            Error::BucketUpdateMergeConflict("refs/heads/main".into()).error_key(),
            "error.bucket_update_merge_conflict"
        );
        assert_eq!(
            Error::BucketUpdateLocalChanges("a.txt".into()).error_key(),
            "error.bucket_update_local_changes"
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
            Error::ConfigKeyUnsupported("x".into()).error_key(),
            "error.config_key_unsupported"
        );
        assert_eq!(
            Error::ConfigValueInvalid("x".into()).error_key(),
            "error.config_value_invalid"
        );
    }

    #[test]
    fn test_error_key_alias() {
        assert_eq!(
            Error::AliasAlreadyExists("rm".into()).error_key(),
            "error.alias_already_exists"
        );
        assert_eq!(
            Error::AliasNotFound("rm".into()).error_key(),
            "error.alias_not_found"
        );
        assert_eq!(
            Error::AliasScriptExists("scoop-rm.ps1".into()).error_key(),
            "error.alias_script_exists"
        );
        assert_eq!(
            Error::AliasInvalidName("a b".into()).error_key(),
            "error.alias_invalid_name"
        );
        assert_eq!(
            Error::AliasReservedName("list".into()).error_key(),
            "error.alias_reserved_name"
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
        assert_eq!(
            Error::OutputAlreadySet.error_key(),
            "error.output_already_set"
        );
        assert_eq!(
            Error::InfraAlreadySet.error_key(),
            "error.infra_already_set"
        );
    }
}
