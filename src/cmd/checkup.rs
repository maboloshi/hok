//! Check for potential problems with the environment.

use clap::Parser;
use libscoop::{
    Session,
    package::diagnostic::{self, DiagnosticIssue, DiagnosticReport, Helper},
};

use crate::{Result, output, output::style};

/// Check for potential problems
#[derive(Debug, Parser)]
pub struct Args {}

pub async fn execute(_: Args, session: &Session) -> Result<()> {
    let report = diagnostic::diagnose(session);
    render(&report);

    // Upstream `scoop checkup` always exits 0, even when it reports problems
    // (`libexec/scoop-checkup.ps1:58`).
    Ok(())
}

/// Render the diagnostic the way upstream does: the Defender performance notes
/// first (upstream prints them as it checks), then one warning per problem,
/// then the summary.
fn render(report: &DiagnosticReport) {
    for issue in &report.performance {
        if let DiagnosticIssue::DefenderPerformance { path, .. } = issue {
            output::info(hok_i18n::t!("cmd.checkup_defender"));
            output::info(hok_i18n::t!("cmd.checkup_defender_consider"));
            output::info(hok_i18n::t!(
                "cmd.checkup_defender_exclude",
                path = path.as_str()
            ));
            output::info(hok_i18n::t!("cmd.checkup_defender_sudo"));
        }
    }

    for issue in &report.issues {
        match issue {
            DiagnosticIssue::MainBucketMissing => {
                output::warn(hok_i18n::t!("cmd.checkup_main_bucket"));
                output::info(hok_i18n::t!("cmd.checkup_main_bucket_hint"));
            }
            DiagnosticIssue::LongPathsUnsupported => {
                output::warn(hok_i18n::t!("cmd.checkup_long_paths_unsupported"));
            }
            DiagnosticIssue::LongPathsDisabled => {
                output::warn(hok_i18n::t!("cmd.checkup_long_paths_disabled"));
                output::info(hok_i18n::t!("cmd.checkup_long_paths_enable"));
                output::info(hok_i18n::t!("cmd.checkup_long_paths_set"));
                output::info(hok_i18n::t!("cmd.checkup_defender_sudo"));
            }
            DiagnosticIssue::DeveloperModeDisabled => {
                output::warn(hok_i18n::t!("cmd.checkup_developer_mode"));
                output::info(hok_i18n::t!("cmd.checkup_developer_mode_read"));
                output::info(hok_i18n::t!("cmd.checkup_developer_mode_link"));
            }
            DiagnosticIssue::MissingHelper(helper) => {
                let message = match helper {
                    Helper::SevenZip => hok_i18n::t!("cmd.checkup_missing_7zip"),
                    Helper::Innounp => hok_i18n::t!("cmd.checkup_missing_innounp"),
                    Helper::Dark => hok_i18n::t!("cmd.checkup_missing_dark"),
                };
                output::warn(message);
            }
            DiagnosticIssue::NotNtfs { global: true, .. } => {
                output::err(hok_i18n::t!("cmd.checkup_ntfs_global"));
            }
            DiagnosticIssue::NotNtfs { global: false, .. } => {
                output::err(hok_i18n::t!("cmd.checkup_ntfs_root"));
            }
            // Rendered in the performance loop above; never a "problem".
            DiagnosticIssue::DefenderPerformance { .. } => {}
        }
    }

    if !report.issues.is_empty() {
        output::warn(hok_i18n::t!(
            "cmd.checkup_potential_problems",
            count = report.issues.len()
        ));
    } else if !report.performance.is_empty() {
        output::info(hok_i18n::t!(
            "cmd.checkup_performance_problems",
            count = report.performance.len()
        ));
        output::warn(hok_i18n::t!("cmd.checkup_security_over_performance"));
    } else {
        println!(
            "{}",
            style::success(hok_i18n::t!("cmd.checkup_no_problems"))
        );
    }
}
