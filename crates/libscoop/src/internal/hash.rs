//! Hash utilities — MD5, SHA1, SHA256, SHA512.
//!
//! Provides a unified API ([`ChecksumBuilder`]) for MD5, SHA1, SHA256, SHA512
//! using the RustCrypto crates (`md-5`, `sha1`, `sha2`) — battle-tested,
//! pure Rust, no C deps.
//!
//! This module was merged from the former standalone `scoop-hash` crate and
//! is not part of the stable API (see [`crate::internal`]).

use std::error::Error as StdError;
use std::io::Read;
use std::path::Path;

use md5::Md5;
use sha1::Sha1;
use sha2::{Sha256, Sha512};

trait Hasher {
    fn hash_type(&self) -> String;
    fn update(&mut self, data: &[u8]);
    fn sum(self: Box<Self>) -> String;
}

impl core::fmt::Debug for dyn Hasher {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Hasher {{ hash_type: {} }}", self.hash_type())
    }
}

macro_rules! impl_hasher_for {
    ($hasher:ty, $digest:path, $name:literal) => {
        impl Hasher for $hasher {
            fn hash_type(&self) -> String {
                $name.to_string()
            }

            fn update(&mut self, data: &[u8]) {
                <$hasher as $digest>::update(self, data);
            }

            fn sum(self: Box<Self>) -> String {
                use std::fmt::Write;
                let digest = <$hasher as $digest>::finalize(*self);
                let mut hex = String::with_capacity(digest.len() * 2);
                for byte in digest.iter() {
                    let _ = write!(hex, "{:02x}", byte);
                }
                hex
            }
        }
    };
}

impl_hasher_for!(Md5, md5::Digest, "md5");
impl_hasher_for!(Sha1, sha1::Digest, "sha1");
impl_hasher_for!(Sha256, sha2::Digest, "sha256");
impl_hasher_for!(Sha512, sha2::Digest, "sha512");

#[derive(Debug)]
pub struct HashError;

impl StdError for HashError {}

impl core::fmt::Display for HashError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "unsupported hash algorithm")
    }
}

/// ChecksumBuilder is used to create a Checksum instance.
pub struct ChecksumBuilder {
    hasher: Box<dyn Hasher>,
}

impl Default for ChecksumBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Macro to generate a `ChecksumBuilder::*` method for a given hash algorithm.
macro_rules! checksum_method {
    ($(#[$doc:meta])* fn $name:ident($ctor:ident, $digest:path)) => {
        $(#[$doc])*
        pub fn $name(self) -> ChecksumBuilder {
            let algo: Box<dyn Hasher> = Box::new(<$ctor as $digest>::new());
            self.set_algo(algo)
        }
    };
}

impl ChecksumBuilder {
    /// Creates a new ChecksumBuilder instance.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // `internal` is pub(crate); shown here for crate-internal consumers.
    /// use libscoop::internal::hash::ChecksumBuilder;
    /// let mut md5 = ChecksumBuilder::new().md5().build();
    /// md5.consume(b"hello world");
    /// assert_eq!(md5.finalize(), "5eb63bbbe01eeed093cb22bb8f5acdc3");
    /// ```
    pub fn new() -> ChecksumBuilder {
        ChecksumBuilder {
            hasher: Box::new(<Sha256 as sha2::Digest>::new()),
        }
    }

    /// Use the specified hash algorithm.
    ///
    /// # Errors
    ///
    /// Returns an error if the specified algorithm is not supported.
    pub fn algo(self, algo: &str) -> Result<ChecksumBuilder, HashError> {
        match algo {
            "md5" => Ok(self.md5()),
            "sha1" => Ok(self.sha1()),
            "sha256" => Ok(self.sha256()),
            "sha512" => Ok(self.sha512()),
            _ => Err(HashError),
        }
    }

    checksum_method! {
        /// Use the md5 hash algorithm.
        fn md5(Md5, md5::Digest)
    }

    checksum_method! {
        /// Use the sha1 hash algorithm.
        fn sha1(Sha1, sha1::Digest)
    }

    checksum_method! {
        /// Use the sha256 hash algorithm.
        fn sha256(Sha256, sha2::Digest)
    }

    checksum_method! {
        /// Use the sha512 hash algorithm.
        fn sha512(Sha512, sha2::Digest)
    }

    fn set_algo(mut self, algo: Box<dyn Hasher>) -> ChecksumBuilder {
        self.hasher = algo;
        self
    }

    /// Build the Checksum instance for use.
    pub fn build(self) -> Checksum {
        Checksum {
            hasher: self.hasher,
        }
    }
}

/// Checksum is a wrapper around a hash algorithm.
#[derive(Debug)]
pub struct Checksum {
    hasher: Box<dyn Hasher>,
}

impl Checksum {
    /// Consumes the provided data.
    pub fn consume(&mut self, data: &[u8]) {
        self.hasher.update(data);
    }

    /// Gets the result of the hash computation as a hex string.
    ///
    /// Note that the Checksum instance is consumed after getting the result.
    pub fn finalize(self) -> String {
        self.hasher.sum()
    }
}

/// Compute the hash of a file using the given algorithm.
///
/// Supported algorithms: `md5`, `sha1`, `sha256`, `sha512`.
/// Returns the hash as a lowercase hex string.
pub fn compute_file_hash(path: &Path, algo: &str) -> std::io::Result<String> {
    let builder = ChecksumBuilder::new().algo(algo).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "unsupported hash algorithm",
        )
    })?;
    let mut hasher = builder.build();
    let mut file = std::fs::File::open(path)?;
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.consume(&buf[..n]);
    }
    Ok(hasher.finalize())
}

/// Format a raw hash value with the Scoop-compatible algorithm prefix.
///
/// `sha256` hashes are written bare; `md5`/`sha1`/`sha512` get an
/// `algo:` prefix (matching Scoop's `format_hash`). Unknown algorithms
/// are returned bare.
///
/// This is the single source of truth for hash prefix formatting, shared
/// by `checkhashes` (algo-name based) and `checkver_hash` (length based).
pub fn format_hash_value(algo: &str, hash: &str) -> String {
    match algo {
        "md5" => format!("md5:{hash}"),
        "sha1" => format!("sha1:{hash}"),
        "sha512" => format!("sha512:{hash}"),
        _ => hash.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── format_hash_value ───────────────────────────────────────────────────

    #[test]
    fn format_sha256_returns_bare_value() {
        assert_eq!(format_hash_value("sha256", "abc123"), "abc123");
    }

    #[test]
    fn format_md5_adds_prefix() {
        assert_eq!(format_hash_value("md5", "deadbeef"), "md5:deadbeef");
    }

    #[test]
    fn format_sha1_adds_prefix() {
        assert_eq!(format_hash_value("sha1", "aabbcc"), "sha1:aabbcc");
    }

    #[test]
    fn format_sha512_adds_prefix() {
        assert_eq!(format_hash_value("sha512", "longvalue"), "sha512:longvalue");
    }

    #[test]
    fn format_unknown_algo_returns_bare() {
        assert_eq!(format_hash_value("crc32", "deadcode"), "deadcode");
    }
}
