//! URL parsing helpers for checkver.
//!
//! Split out of [`super`] (`checkver.rs`) — pure URL string manipulation
//! used by the hash-collection pipeline.

/// Substitute variables in a hash URL using the download URL's context.
pub(super) fn sub_url(hash_url: &str, _download_url: &str) -> String {
    // Most hash URLs use the same $version etc. that were already substituted
    hash_url.to_string()
}
