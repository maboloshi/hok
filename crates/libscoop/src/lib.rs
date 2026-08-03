//! This crate aims to provide a full-featured, practical, and efficient Rust
//! reimplementation of [Scoop], the Windows command-line installer. It is a
//! library crate providing the core functionality of interacting with Scoop,
//! and is not intended to be used directly by end users. Developers who wish
//! to implement a Scoop frontend or make use of Scoop's functionality in their
//! own applications may use this crate. For end users, they may take a glance
//! at [Hok], a reference implementation built on top of this crate, which
//! provides a command-line interface similar to Scoop.
//!
//! # Overview
//!
//! The primary type in this crate is a [`Session`], which is an entry point to
//! this crate. A session instance is basically a handle to the global state of
//! libscoop. Most of the functions exposed by this crate take a session as
//! their first argument.
//!
//! ## Examples
//!
//! Initialize a Scoop session, get the configuration associated with the
//! session, and print the root path of Scoop to stdout:
//!
//! ```rust
//! use libscoop::Session;
//! let session = Session::new();
//! let config = session.config();
//! println!("{}", config.root_path().display());
//! ```
//!
//! [Scoop]: https://scoop.sh/
//! [Hok]: https://github.com/chawyehsu/hok
#[macro_use]
extern crate serde;

rust_i18n::i18n!("../../locales");

pub mod bucket;
pub mod cache;
pub mod config;
mod constant;
mod env;
mod error;
mod event;
mod handler;
pub mod internal;
pub mod network;
pub mod package;
mod persist;
mod psmodule;
mod session;
mod shim;
mod shortcut;
#[cfg(test)]
mod test_utils;

pub use error::Error;
pub use event::Event;
pub use handler::EventHandler;
pub use internal::compare_versions;
pub use package::manifest::{Checkver, Manifest};
pub use package::{QueryOption, SyncOption};
pub use session::Session;

/// Public filesystem helpers (thin facade over [`internal::fs`]).
pub mod fs {
    pub use crate::internal::fs::{read_to_string, walkdir_files, write, write_json};
}

/// Public OS / process helpers (thin facade over [`internal::os`]).
pub mod os {
    pub use crate::internal::os::{is_program_available, run_program};
    pub use crate::internal::string::encode_wide;
}

/// Public string helpers (thin facade over [`internal::string`]).
pub mod string {
    pub use crate::internal::string::{glob_to_regex, matches_any_glob};
}
