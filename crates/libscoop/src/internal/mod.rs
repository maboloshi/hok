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
//! Items in this module are `pub(crate)`: they are **not** part of the stable
//! public API and may change without notice. The facade (`lib.rs`) re-exports
//! only the few symbols CLI consumers need (e.g. `Arch`, `compare_versions`,
//! the `fs`/`os`/`string` helpers).

pub(crate) mod arch;
pub(crate) mod archive;
pub(crate) mod dag;
pub(crate) mod env;
pub(crate) mod fs;
pub(crate) mod git;
pub(crate) mod github;
pub(crate) mod hash;
pub(crate) mod network;
pub(crate) mod os;
pub(crate) mod path;
pub(crate) mod pe;
pub(crate) mod string;
pub(crate) mod time;
pub(crate) mod url;
pub(crate) mod version;

// `pub` (not `pub(crate)`): the facade (`lib.rs`) re-exports this to the
// public API; the `pub(crate)` module visibility already blocks direct
// external access to `internal::`.
pub use version::compare_versions;
