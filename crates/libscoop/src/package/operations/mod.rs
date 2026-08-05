//! Pure operation primitives for the package sync pipeline.
//!
//! Contains only self-contained primitives — symlink (de)registration, archive
//! extraction, downloaded-file copying, and PowerShell script execution with
//! installer-variable expansion. **No transaction logic lives here**:
//! orchestration, confirmation, and event sequencing stay in [`super::sync`].
//! Domain services ([`super::super::persist`], [`super::super::shortcut`],
//! [`super::super::shim`], [`super::super::env`]) are called directly by the
//! pipeline, not re-wrapped here.
//!
//! # Design
//!
//! - Every function here performs a single, self-contained filesystem
//!   operation and reports success/failure via [`Fallible`][1].
//! - Event emission inside these primitives (extract progress) is kept
//!   local to the operation it describes; no cross-operation sequencing
//!   or rollback is performed here.
//! - [`super::sync`] composes these primitives into the install / upgrade /
//!   uninstall / reset pipelines.
//!
//! [1]: crate::error::Fallible

// ─── Submodules ────────────────────────────────────────────────────────────

mod extract;
mod link;
mod script;

// ─── Re-exports ─────────────────────────────────────────────────────────────

pub use extract::*;
pub use link::*;
pub use script::*;
