//! Internal implementation details — not part of the public API.
//!
//! This module organises low-level utilities used across `libscoop`:
//!
//! - [`arch`] — Runtime OS architecture detection (Scoop-compatible)
//! - [`archive`] — Archive extraction (7z, zip, tar, ...)
//! - [`dag`] — Directed Acyclic Graph for dependency resolution
//! - `env` — Windows Registry-backed environment variable management
//! - [`fs`] — File-system utilities (ensure dir, remove dir, write JSON)
//! - [`git`] — Git operations via `libgit2`
//! - [`hash`] — Hash utilities (MD5/SHA1/SHA256/SHA512) via RustCrypto
//! - [`network`] — HTTP networking via `ureq`
//! - [`os`] — OS-level utilities (process info, disk space, FFI)
//! - [`path`] — Path manipulation and normalisation
//! - [`pe`] — PE executable introspection (subsystem detection)
//! - [`string`] — String utilities (encoding, glob matching)
//! - [`time`] — Scoop-compatible `last_update` timestamp codec
//! - [`version`] — Semantic version comparison
//!
//! It also exports [`compare_versions()`], a Scoop-compatible semantic
//! version comparator used across the package lifecycle.
//!
//! # Note
//!
//! Items in this module are `pub` (not `pub(crate)`) only because Rust's
//! visibility rules require it for re-exporting; they are **not** part of
//! the stable API and may change without notice.

pub mod arch;
pub mod archive;
pub mod dag;
pub mod env;
pub mod fs;
pub mod git;
pub mod github;
pub mod hash;
pub mod network;
pub mod os;
pub mod path;
pub mod pe;
pub mod string;
pub mod time;
pub mod url;
pub mod version;

pub use version::compare_versions;
