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

/// A nonzero process exit code a command handler may request.
///
/// Most failures exit with code 1; a handful of Scoop commands are defined to
/// exit with a specific code (`which` returns 2 when the command is not
/// found, VirusTotal combines 2/4/8/16). The handler prints its own message and
/// returns `Err(ExitCode(n).into())`; the CLI runner propagates `n` to the
/// process exit code without rendering an additional error line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitCode(pub i32);

impl std::fmt::Display for ExitCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exit code {}", self.0)
    }
}

impl std::error::Error for ExitCode {}

fn translate_error(err: &libscoop::Error) -> String {
    match err {
        libscoop::Error::BucketNotFound(name) => {
            hok_i18n::t!("error.bucket_not_found", name = name).to_string()
        }
        libscoop::Error::BucketAlreadyExists(name) => {
            hok_i18n::t!("error.bucket_already_exists", name = name).to_string()
        }
        libscoop::Error::BucketInvalidGitUrl(repo) => {
            hok_i18n::t!("error.bucket_invalid_git_url", repo = repo).to_string()
        }
        libscoop::Error::BucketRepoAlreadyExists(bucket, repo) => hok_i18n::t!(
            "error.bucket_repo_already_exists",
            bucket = bucket,
            repo = repo
        )
        .to_string(),
        libscoop::Error::PackageNotFound(name) => {
            hok_i18n::t!("error.package_not_found", name = name).to_string()
        }
        libscoop::Error::ConfigKeyInvalid(key) => {
            hok_i18n::t!("error.config_key_invalid", key = key).to_string()
        }
        libscoop::Error::ConfigKeyUnsupported(key) => {
            hok_i18n::t!("error.config_key_unsupported", key = key).to_string()
        }
        libscoop::Error::ConfigValueInvalid(value) => {
            hok_i18n::t!("error.config_value_invalid", value = value).to_string()
        }
        libscoop::Error::AliasReservedName(name) => {
            hok_i18n::t!("error.alias_reserved_name", name = name).to_string()
        }
        libscoop::Error::ShimTargetMissing(path) => {
            hok_i18n::t!("error.shim_target_missing", path = path).to_string()
        }
        libscoop::Error::ExtractionFailed(reason) => {
            hok_i18n::t!("error.extraction_failed", reason = reason).to_string()
        }
        _ => err.to_string(),
    }
}

fn report(err: &anyhow::Error) {
    // A handler that signalled an explicit exit code already printed its own
    // message; nothing more to render.
    if err.downcast_ref::<ExitCode>().is_some() {
        return;
    }
    if let Some(libscoop_err) = err.downcast_ref::<libscoop::Error>() {
        output::err(translate_error(libscoop_err));
    } else {
        output::err(err.to_string());
    }
    if let Some(cause) = err.source() {
        eprintln!("\n{}", hok_i18n::t!("error.caused_by"));
        for (i, e) in std::iter::successors(Some(cause), |e| e.source()).enumerate() {
            eprintln!("   {}: {}", i, e);
        }
    }
}

pub async fn create_app() -> i32 {
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

    // The multi-thread runtime is provided by #[tokio::main] (rt-multi-thread
    // + macros) in bin/hok.rs, so the CLI runs as an async entry point with
    // no manual Runtime/block_on — concurrent commands (checkver/checkup/
    // download, later install/upgrade) interleave their I/O on worker threads.
    match cmd::start(token).await {
        Ok(()) => 0,
        // A handler signalled a documented nonzero exit code (e.g. `which`
        // exits 2 on not-found); it printed its own message, so exit with that
        // code and report nothing here.
        Err(err) => {
            if let Some(code) = err.downcast_ref::<ExitCode>() {
                return code.0;
            }
            report(&err);
            1
        }
    }
}
