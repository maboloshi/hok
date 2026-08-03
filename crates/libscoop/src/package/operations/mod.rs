//! Pure file-operation primitives for the package sync pipeline.
//!
//! Contains only filesystem primitives — symlink (de)registration, archive
//! extraction, downloaded-file copying, persist-dir handling, and shortcut
//! creation/removal. **No transaction logic lives here**: orchestration,
//! confirmation, and event sequencing stay in [`super::sync`].
//!
//! # Design
//!
//! - Every function here performs a single, self-contained filesystem
//!   operation and reports success/failure via [`Fallible`][1].
//! - Event emission inside these primitives (extract progress, persist
//!   purge) is kept local to the operation it describes; no cross-operation
//!   sequencing or rollback is performed here.
//! - [`super::sync`] composes these primitives into the install / upgrade /
//!   uninstall / reset pipelines.
//!
//! [1]: crate::error::Fallible

// ─── Submodules ────────────────────────────────────────────────────────────

mod extract;
mod link;
mod persist;
mod shortcut;

// ─── Re-exports ─────────────────────────────────────────────────────────────

pub use extract::*;
pub use link::*;
pub use persist::*;
pub use shortcut::*;
