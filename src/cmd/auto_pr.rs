//! CLI wrapper for the CI auto-PR command.
//!
//! This file is intentionally thin: it parses CLI arguments, converts them
//! to plain-data types, and delegates all business logic to
//! [`libscoop::package::auto_pr::run_auto_pr`].

use clap::Parser;
use libscoop::package::checkver::{CheckverReport, ReportSeverity};
use libscoop::Session;
use std::path::PathBuf;

use crate::{output, Result};

/// Auto-update manifests and create pull-requests via GitHub API (CI mode)
///
/// This command runs in CI environments (e.g., GitHub Actions) and uses the
/// GitHub API exclusively — no git/hub binary required.
#[derive(Debug, Parser)]
#[clap(name = "ci-auto-pr", arg_required_else_help = true)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = "bucket")]
    pub(crate) dir: PathBuf,

    /// Upstream repository with target branch (<user>/<repo>:<branch>)
    #[arg(short = 'u', long)]
    pub(crate) upstream: Option<String>,

    /// Push updates directly to origin branch
    #[arg(short = 'p', long)]
    pub(crate) push: bool,

    /// Create pull-requests for each update
    #[arg(short = 'r', long)]
    pub(crate) request: bool,

    /// Origin (local) branch name
    #[arg(short = 'o', long, default_value = "master")]
    pub(crate) origin_branch: String,

    /// Commit message format (<app> and <version> are replaced)
    #[arg(short = 'm', long, default_value = "<app>: Update to version <version>")]
    pub(crate) message: String,

    /// Skip manifests that are already up-to-date
    #[arg(short = 's', long = "skip-updated")]
    pub(crate) skip_updated: bool,

    /// Force update on these special snowflakes (comma-separated)
    #[arg(long)]
    pub(crate) special: Option<String>,

    /// GitHub Token (default from GITHUB_TOKEN env var)
    #[arg(short = 'T', long)]
    pub(crate) token: Option<String>,

    /// Show per-manifest checkver details (versions found, updates, errors)
    #[arg(short = 'v', long, action = clap::ArgAction::Count)]
    pub(crate) verbose: u8,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let token = args
        .token
        .clone()
        .unwrap_or_else(|| std::env::var("GITHUB_TOKEN").unwrap_or_default());

    let (upstream_repo_nwo, upstream_branch) = if args.request {
        let upstream = args.upstream.as_deref().unwrap_or("");
        let parts: Vec<&str> = upstream.splitn(2, ':').collect();
        (
            parts[0].to_string(),
            parts.get(1).unwrap_or(&"master").to_string(),
        )
    } else {
        (String::new(), String::new())
    };

    let special: Vec<String> = args
        .special
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let config = libscoop::package::auto_pr::AutoPrConfig {
        token,
        dir: args.dir,
        push: args.push,
        request: args.request,
        upstream_repo_nwo,
        upstream_branch,
        origin_branch: args.origin_branch,
        message: args.message,
        skip_updated: args.skip_updated,
        special,
    };

    // Presentation is a CLI concern: with `-v`, stream the per-manifest
    // checkver reports through the same renderer as the standalone
    // `checkver` command; otherwise pass no callback and auto-pr keeps its
    // compact progress/ok output.
    let on_report: Option<&mut dyn FnMut(&CheckverReport)> = if args.verbose > 0 {
        Some(&mut render_checkver_report)
    } else {
        None
    };

    libscoop::package::auto_pr::run_auto_pr(config, session, on_report)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Render a per-manifest checkver report, mirroring the standalone
/// `checkver` command (`src/cmd/checkver.rs`): status lines for version
/// changes, done/warn/err for outcomes and failures.
fn render_checkver_report(report: &CheckverReport) {
    if let Some(msg) = &report.message {
        match report.severity {
            ReportSeverity::Warn => output::warn(format!("{}: {}", report.stem, msg)),
            _ => output::err(format!("{}: {}", report.stem, msg)),
        }
        return;
    }

    let Some(ver) = &report.new_version else {
        return;
    };

    if ver == &report.current {
        output::status(format!("  {} ({})", report.stem, ver));
        if report.updated {
            output::done(rust_i18n::t!("cmd.checkver_updated_to", ver = ver));
        }
    } else {
        output::status(format!(
            "  {} ({} -> {})",
            report.stem, report.current, ver
        ));
        if report.autoupdate_available {
            output::status("    autoupdate available");
        }
        if report.updated {
            output::done(rust_i18n::t!("cmd.checkver_updated_to", ver = ver));
        }
    }
}
