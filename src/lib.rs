//! Binary crate entry point for Hok — a Scoop-compatible package manager.
//!
//! This crate ties together the CLI layer ([`cmd`]), the core library
//! (`libscoop`), the event loop ([`eventloop`]), the output subsystem
//! ([`output`] / [`cui`]), and the default event handler ([`scoop_handler`]).
//! It is the only crate that produces a binary; all business logic lives in
//! `libscoop` and `hok-shim`.
//!
//! # Design
//!
//! - **Thin orchestration layer**: This crate is deliberately shallow — it
//!   parses CLI arguments, initialises the [`Session`], and dispatches to the
//!   appropriate command module. Core data structures and operations reside in
//!   `libscoop`.
//! - **Global detail mode**: The [`DETAIL_MODE`] atomic flag controls whether
//!   extra operational detail is printed. It is set once at startup from the
//!   `--detail` CLI flag and read by [`output::detail`] throughout.
//! - **i18n-first**: Language detection happens before CLI parsing so that
//!   help text is rendered in the correct language. See [`i18n`] module.
//! - **Error reporting**: Library errors (`libscoop::Error`) are mapped to
//!   user-facing i18n messages via [`translate_error`]; the [`report`] function
//!   prints the full error chain (error + causes).
//!
//! # Entry flow
//!
//! 1. `main()` (in `bin/hok.rs`) → `create_app()`
//! 2. `create_app()` → `cmd::start()`
//! 3. `cmd::start()` — language detection → logger init → session creation →
//!    output style/color config → command dispatch
//!
//! # Adding a new command
//!
//! 1. Drop a `.rs` file into `src/cmd/` — `build.rs` auto-generates the
//!    `mod` declaration in `__cmd_reg__.rs`.
//! 2. Add a variant to the [`Command`] enum in `cmd/mod.rs`.
//! 3. Add the match arm in `cmd::start()`.
//!
//! See `cmd/mod.rs` for details.

use crossterm::{
    style::{Color, Print, SetForegroundColor},
    ExecutableCommand,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fmt::Display, io};

rust_i18n::i18n!("locales");

// ─── CLI framework infrastructure ────────────────────────────────────────

mod cmd;
mod cui;
mod eventloop;

// ─── Output & formatting ──────────────────────────────────────────────────

mod format;
mod output;

// ─── Config & initialisation ─────────────────────────────────────────────

mod i18n;
mod scoop_handler;

type Result<T> = anyhow::Result<T>;

static DETAIL_MODE: AtomicBool = AtomicBool::new(false);

pub fn set_detail(enabled: bool) {
    DETAIL_MODE.store(enabled, Ordering::Relaxed);
}

pub fn is_detail() -> bool {
    DETAIL_MODE.load(Ordering::Relaxed)
}

fn error<T: Display>(input: &T) -> io::Result<()> {
    let mut stderr = io::stderr();
    stderr
        .execute(SetForegroundColor(Color::Red))?
        .execute(Print("ERROR "))?
        .execute(SetForegroundColor(Color::Reset))?
        .execute(Print(input))?
        .execute(Print("\n"))?;
    Ok(())
}

fn translate_error(err: &libscoop::Error) -> String {
    match err {
        libscoop::Error::BucketNotFound(name) => {
            rust_i18n::t!("error.bucket_not_found", name = name).to_string()
        }
        libscoop::Error::BucketAlreadyExists(name) => {
            rust_i18n::t!("error.bucket_already_exists", name = name).to_string()
        }
        libscoop::Error::PackageNotFound(name) => {
            rust_i18n::t!("error.package_not_found", name = name).to_string()
        }
        libscoop::Error::ConfigKeyInvalid(key) => {
            rust_i18n::t!("error.config_key_invalid", key = key).to_string()
        }
        libscoop::Error::ConfigValueInvalid(value) => {
            rust_i18n::t!("error.config_value_invalid", value = value).to_string()
        }
        libscoop::Error::AppRunning(name) => {
            rust_i18n::t!("error.app_running", name = name).to_string()
        }
        libscoop::Error::ExtractionFailed(reason) => {
            rust_i18n::t!("error.extraction_failed", reason = reason).to_string()
        }
        _ => err.to_string(),
    }
}

fn report(err: &anyhow::Error) {
    if let Some(libscoop_err) = err.downcast_ref::<libscoop::Error>() {
        let msg = translate_error(libscoop_err);
        let msg_s: String = msg;
        let _ = error(&msg_s);
    } else {
        let _ = error(err);
    }
    if let Some(cause) = err.source() {
        eprintln!("\nCaused by:");
        for (i, e) in std::iter::successors(Some(cause), |e| e.source()).enumerate() {
            eprintln!("   {}: {}", i, e);
        }
    }
}

pub fn create_app() -> bool {
    // Async runtime host for the whole CLI (phase 4: operations
    // async-ification). Commands run as async tasks; a multi-thread runtime
    // lets concurrent commands (checkver/checkup/download, later install/
    // upgrade) interleave their I/O on worker threads.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build the tokio runtime");

    // Cooperative cancellation (purity P2): a Ctrl-C (SIGINT) aborts the
    // whole command through a CancellationToken shared by every long-running
    // operation (downloads, scripts, waits), so temp files, subprocesses and
    // the terminal cursor are cleaned up instead of the runtime just dying.
    let token = tokio_util::sync::CancellationToken::new();
    let cancel = token.clone();
    tokio::spawn(async move {
        // In a non-console/CI context ctrl_c() may fail to install a handler;
        // that is fine — cancellation is a best-effort nicety, not a guard.
        if tokio::signal::ctrl_c().await.is_ok() {
            cancel.cancel();
        }
    });

    runtime.block_on(cmd::start(token)).inspect_err(report).is_err()
}
