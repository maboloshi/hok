//! Hash library for libscoop.
//!
//! Provides a unified API (`ChecksumBuilder`) for MD5, SHA1, SHA256, SHA512.
//! Uses the RustCrypto crates (`md-5`, `sha1`, `sha2`) for the actual hash
//! implementations — battle-tested, pure Rust, no C deps.

use std::error::Error as StdError;
use std::io::Read;
use std::path::Path;

mod rustcrypto;
use rustcrypto::{Digest, Md5, Sha1, Sha256, Sha512};

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
    ($hasher:ty) => {
        impl Hasher for $hasher {
            fn hash_type(&self) -> String {
                stringify!($hasher).to_string()
            }

            fn update(&mut self, data: &[u8]) {
                self.inner.update(data);
            }

            fn sum(self: Box<Self>) -> String {
                use std::fmt::Write;
                let digest = self.inner.finalize();
                let mut hex = String::with_capacity(digest.len() * 2);
                for byte in digest.iter() {
                    let _ = write!(hex, "{:02x}", byte);
                }
                hex
            }
        }
    };
}

impl_hasher_for!(Md5);
impl_hasher_for!(Sha1);
impl_hasher_for!(Sha256);
impl_hasher_for!(Sha512);

#[derive(Debug)]
pub struct Error;

impl StdError for Error {}

impl core::fmt::Display for Error {
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
    ($(#[$doc:meta])* fn $name:ident($ctor:ident)) => {
        $(#[$doc])*
        pub fn $name(self) -> ChecksumBuilder {
            let algo: Box<dyn Hasher> = Box::new($ctor::new());
            self.set_algo(algo)
        }
    };
}

impl ChecksumBuilder {
    /// Creates a new ChecksumBuilder instance.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use scoop_hash::ChecksumBuilder;
    /// let mut md5 = ChecksumBuilder::new().md5().build();
    /// md5.consume(b"hello world");
    /// assert!(md5.check("5eb63bbbe01eeed093cb22bb8f5acdc3"));
    /// ```
    pub fn new() -> ChecksumBuilder {
        ChecksumBuilder {
            hasher: Box::new(Sha256::new()),
        }
    }

    /// Use the specified hash algorithm.
    ///
    /// # Errors
    ///
    /// Returns an error if the specified algorithm is not supported.
    pub fn algo(self, algo: &str) -> Result<ChecksumBuilder, Error> {
        match algo {
            "md5" => Ok(self.md5()),
            "sha1" => Ok(self.sha1()),
            "sha256" => Ok(self.sha256()),
            "sha512" => Ok(self.sha512()),
            _ => Err(Error),
        }
    }

    checksum_method! {
        /// Use the md5 hash algorithm.
        fn md5(Md5)
    }

    checksum_method! {
        /// Use the sha1 hash algorithm.
        fn sha1(Sha1)
    }

    checksum_method! {
        /// Use the sha256 hash algorithm.
        fn sha256(Sha256)
    }

    checksum_method! {
        /// Use the sha512 hash algorithm.
        fn sha512(Sha512)
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

    /// Checks if the result of the hash computation matches the input hash.
    pub fn check(self, input: &str) -> bool {
        input == self.finalize()
    }
}

/// Compute the hash of a file using the given algorithm.
///
/// Supported algorithms: `md5`, `sha1`, `sha256`, `sha512`.
/// Returns the hash as a lowercase hex string.
pub fn compute_file_hash(path: &Path, algo: &str) -> std::io::Result<String> {
    let builder = ChecksumBuilder::new()
        .algo(algo)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "unsupported hash algorithm"))?;
    let mut hasher = builder.build();
    let mut file = std::fs::File::open(path)?;
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 { break; }
        hasher.consume(&buf[..n]);
    }
    Ok(hasher.finalize())
}
