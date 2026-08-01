use clap::Parser;
use libscoop::{package::checkurls, Session};
use std::path::PathBuf;

use crate::{output, Result};

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

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let dir = &args.dir;
    if !dir.is_dir() {
        output::err(rust_i18n::t!("cmd.checkurls_err_dir", path = dir.display()));
        return Ok(());
    }

    // Scoop-style header: [U]RLs [O]kay [F]ailed
    // The ps1 prints: '[' + 'U' (cyan) + ']RLs | [' + 'O' (green) + ']kay |  | [' + 'F' (red) + ']ailed |  |  |'
    use crossterm::style::Stylize;
    eprintln!(
        "[{}] RLs   [{}] [{}]",
        "U".cyan(),
        "O".green(),
        "F".red(),
    );
    eprintln!();

    let report = checkurls::check_urls(session, dir, &args.app, args.timeout, args.skip_valid)?;

    for result in &report.results {
        eprint!("[{}]", result.total_urls.to_string().cyan());
        eprint!("[{}]",
            if result.ok_count == result.total_urls {
                result.ok_count.to_string().green()
            } else if result.ok_count == 0 {
                result.ok_count.to_string().red()
            } else {
                result.ok_count.to_string().yellow()
            }
        );
        eprint!("[{}]",
            if result.failed_count == 0 {
                result.failed_count.to_string().green()
            } else {
                result.failed_count.to_string().red()
            }
        );
        eprintln!(" {}", result.name);

        for err in &result.errors {
            eprintln!(
                "        > {} ({})",
                err.message.clone().dark_red(),
                err.url
            );
        }
    }

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
use crate::cmd::shared_args::Cmd;

impl Cmd for Args {
    type Args = Self;

    #[inline]
    fn execute(args: Self::Args, session: &Session) -> Result<()> {
        execute(args, session)
    }
}
