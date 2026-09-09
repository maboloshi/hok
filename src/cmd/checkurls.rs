//! Check for broken download URLs in package manifests.

use clap::Parser;
use libscoop::{Session, package::checkurls};
use std::path::PathBuf;

use crate::cmd::shared_args::validate_dir;
use crate::output::style;
use crate::{Result, output};

/// Check manifest URLs for validity
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = ".")]
    dir: PathBuf,

    /// Specific app(s) to check (supports wildcards, default: all)
    #[arg(default_value = "*")]
    app: Vec<String>,

    /// Request timeout in seconds (Scoop default: 5)
    #[arg(short = 't', long, default_value = "5")]
    timeout: u64,

    /// Only show invalid URLs (suppress valid ones)
    #[arg(short = 's', long, action = clap::ArgAction::SetTrue)]
    skip_valid: bool,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    validate_dir(&args.dir)?;
    let dir = &args.dir;

    // Scoop-style header: [U]RLs [O]kay [F]ailed. The report (header + one
    // row per manifest + per-error detail) is this command's whole output, so
    // it goes to stdout — stderr is reserved for actual warnings/errors.
    println!(
        "[{}] RLs   [{}] [{}]",
        style::accent("U"),
        style::success("O"),
        style::danger("F")
    );

    // Stream results: each manifest is reported as soon as its URL check
    // completes, matching Scoop's per-manifest output while checking.
    let report = checkurls::check_urls(
        session,
        dir,
        &args.app,
        args.timeout,
        args.skip_valid,
        |result| {
            print!("[{}]", style::accent(result.total_urls.to_string()));
            print!(
                "[{}]",
                if result.ok_count == result.total_urls {
                    style::success(result.ok_count.to_string())
                } else if result.ok_count == 0 {
                    style::danger(result.ok_count.to_string())
                } else {
                    style::warning(result.ok_count.to_string())
                }
            );
            print!(
                "[{}]",
                if result.failed_count == 0 {
                    style::success(result.failed_count.to_string())
                } else {
                    style::danger(result.failed_count.to_string())
                }
            );
            println!(" {}", result.name);

            for err in &result.errors {
                println!("        > {} ({})", style::danger(&err.message), err.url);
            }
        },
    )
    .await?;

    if report.total_manifests == 0 {
        output::info(rust_i18n::t!("cmd.checkurls_no_manifests"));
    } else {
        output::info(rust_i18n::t!(
            "cmd.checkurls_summary",
            total = report.total_urls,
            valid = report.total_valid,
            invalid = report.total_invalid
        ));
    }

    Ok(())
}
