//! Environment diagnostic checks (upstream `libexec/scoop-checkup.ps1` +
//! `lib/diagnostic.ps1`).
//!
//! Upstream `scoop checkup` inspects the *environment* rather than the
//! installed packages: administrator/Defender performance, the `main` bucket,
//! long-path support, Windows Developer Mode, the implicit installation
//! helpers, and the NTFS requirement for both Scoop roots.
//!
//! # Design
//!
//! - **Pure decision layer**: [`evaluate`] is a side-effect-free function over
//!   an [`EnvFacts`] snapshot, so every branch is unit-testable offline.
//! - **Thin OS gathering**: [`diagnose`] fills the snapshot from the session
//!   and the `internal::os` probes, then delegates to [`evaluate`].
//! - **Unknown means pass**: every probe that cannot read its subject is
//!   treated as "no problem" (upstream skips the check when the corresponding
//!   PowerShell cmdlet is unavailable).

use crate::{Session, bucket, internal};

use super::resolve;

/// An implicit installation helper upstream expects to be present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Helper {
    /// `7zip` — required to unpack most archives.
    SevenZip,
    /// `innounp` — required to unpack Inno Setup installers.
    Innounp,
    /// `dark` — required to unpack WiX Toolset installers.
    Dark,
}

/// A problem the environment diagnostic found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticIssue {
    /// Defender realtime scanning may slow down installs; `path` is the root
    /// that should be excluded. `global` selects the global root.
    DefenderPerformance { global: bool, path: String },
    /// The `main` bucket is not added.
    MainBucketMissing,
    /// Windows predates configurable long-path support.
    LongPathsUnsupported,
    /// `LongPathsEnabled` is off.
    LongPathsDisabled,
    /// Windows Developer Mode is off.
    DeveloperModeDisabled,
    /// An installation helper is missing.
    MissingHelper(Helper),
    /// The local (`global = false`) or global (`global = true`) root is not on
    /// an NTFS volume.
    NotNtfs { global: bool, path: String },
}

/// Outcome of the environment diagnostic, split the way upstream counts it.
///
/// `performance` maps to upstream's `$defenderIssues` and `issues` to its
/// `$issues`: the summary reports the two separately.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DiagnosticReport {
    /// Defender performance concerns.
    pub performance: Vec<DiagnosticIssue>,
    /// Potential problems.
    pub issues: Vec<DiagnosticIssue>,
}

impl DiagnosticReport {
    /// Whether the diagnostic found nothing at all.
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty() && self.performance.is_empty()
    }
}

/// A snapshot of every environment fact the diagnostic inspects.
#[derive(Debug, Clone)]
pub struct EnvFacts {
    /// The current process has administrator privileges.
    pub is_admin: bool,
    /// The current account name (`USERNAME`).
    pub user_name: String,
    /// Names of the added buckets.
    pub buckets: Vec<String>,
    /// Windows major version (0 when unreadable).
    pub os_major: u32,
    /// Windows build number (0 when unreadable).
    pub os_build: u32,
    /// `LongPathsEnabled` is set to 1.
    pub long_paths_enabled: bool,
    /// `AllowDevelopmentWithoutDevLicense` is set to 1.
    pub developer_mode_enabled: bool,
    /// The `7zip` helper is installed.
    pub helper_7zip: bool,
    /// The `innounp` helper is installed.
    pub helper_innounp: bool,
    /// The `dark` helper is installed.
    pub helper_dark: bool,
    /// The local Scoop root, as a display string.
    pub root_path: String,
    /// The global Scoop root, as a display string.
    pub global_path: String,
    /// The local root is on an NTFS volume.
    pub root_ntfs: bool,
    /// The global root is on an NTFS volume.
    pub global_ntfs: bool,
    /// Defender facts, or `None` when Defender could not be inspected.
    pub defender: Option<DefenderFacts>,
}

/// The Defender facts behind the performance check (`Get-MpPreference`).
#[derive(Debug, Clone)]
pub struct DefenderFacts {
    /// Realtime monitoring is explicitly disabled.
    pub realtime_disabled: bool,
    /// The `WinDefend` service is running.
    pub service_running: bool,
    /// Paths already excluded from Defender scanning.
    pub exclusions: Vec<String>,
}

/// Gather the environment facts from the OS and evaluate them.
pub fn diagnose(session: &Session) -> DiagnosticReport {
    let buckets = bucket::list(session)
        .map(|buckets| {
            buckets
                .iter()
                .map(|bucket| bucket.name().to_owned())
                .collect()
        })
        .unwrap_or_default();

    let root_path = session.config().root_path().to_path_buf();
    let global_path = session.config().global_path().to_path_buf();
    let (os_major, os_build) = internal::os::windows_version();

    let facts = EnvFacts {
        is_admin: session.is_admin(),
        user_name: std::env::var("USERNAME").unwrap_or_default(),
        buckets,
        os_major,
        os_build,
        long_paths_enabled: internal::os::registry_dword(
            r"SYSTEM\CurrentControlSet\Control\FileSystem",
            "LongPathsEnabled",
        ) == Some(1),
        developer_mode_enabled: internal::os::registry_dword(
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock",
            "AllowDevelopmentWithoutDevLicense",
        ) == Some(1),
        helper_7zip: resolve::helper_installed(session, "7zip"),
        helper_innounp: resolve::helper_installed(session, "innounp"),
        helper_dark: resolve::helper_installed(session, "dark"),
        root_path: root_path.to_string_lossy().into_owned(),
        global_path: global_path.to_string_lossy().into_owned(),
        root_ntfs: internal::os::volume_is_ntfs(&root_path).unwrap_or(true),
        global_ntfs: internal::os::volume_is_ntfs(&global_path).unwrap_or(true),
        defender: defender_facts(),
    };

    evaluate(&facts)
}

/// Read the Defender facts.
///
/// `None` stands for "Defender cannot be inspected" — upstream skips the whole
/// Defender check when `Get-MpPreference` is unavailable, which is exactly the
/// case where the Defender policy key is absent.
fn defender_facts() -> Option<DefenderFacts> {
    const POLICY_KEY: &str = r"SOFTWARE\Microsoft\Windows Defender";
    if !internal::os::registry_key_exists(POLICY_KEY) {
        return None;
    }

    Some(DefenderFacts {
        realtime_disabled: internal::os::registry_dword(
            r"SOFTWARE\Microsoft\Windows Defender\Real-Time Protection",
            "DisableRealtimeMonitoring",
        ) == Some(1),
        // An unqueryable service is "not running" here, matching upstream's
        // fall-through when `Get-Service -Name WinDefend` finds nothing.
        service_running: internal::os::is_service_running("WinDefend").unwrap_or(false),
        exclusions: internal::os::registry_subkeys(
            r"SOFTWARE\Microsoft\Windows Defender\Exclusions\Paths",
        )
        .unwrap_or_default(),
    })
}

/// Apply the upstream checks to a fact snapshot.
///
/// The checks and their order mirror `libexec/scoop-checkup.ps1:11-47`: the
/// Defender performance notes are collected first (upstream prints them as it
/// checks), then the potential problems.
pub fn evaluate(facts: &EnvFacts) -> DiagnosticReport {
    let mut report = DiagnosticReport::default();

    // Defender performance is only meaningful as an administrator, and the
    // Windows Defender Application Guard utility account is excluded upstream.
    if facts.is_admin
        && facts.user_name != "WDAGUtilityAccount"
        && let Some(defender) = &facts.defender
    {
        for (global, path) in [(false, &facts.root_path), (true, &facts.global_path)] {
            if defender_performance_issue(defender, path) {
                report
                    .performance
                    .push(DiagnosticIssue::DefenderPerformance {
                        global,
                        path: path.clone(),
                    });
            }
        }
    }

    if !facts.buckets.iter().any(|bucket| bucket == "main") {
        report.issues.push(DiagnosticIssue::MainBucketMissing);
    }

    if facts.os_major < 10 || facts.os_build < 1607 {
        report.issues.push(DiagnosticIssue::LongPathsUnsupported);
    } else if !facts.long_paths_enabled {
        report.issues.push(DiagnosticIssue::LongPathsDisabled);
    }

    if !facts.developer_mode_enabled {
        report.issues.push(DiagnosticIssue::DeveloperModeDisabled);
    }

    // `use_external_7zip` is not modelled by hok's config, so the helper is
    // always required (upstream only skips this when that key is set).
    if !facts.helper_7zip {
        report
            .issues
            .push(DiagnosticIssue::MissingHelper(Helper::SevenZip));
    }
    if !facts.helper_innounp {
        report
            .issues
            .push(DiagnosticIssue::MissingHelper(Helper::Innounp));
    }
    if !facts.helper_dark {
        report
            .issues
            .push(DiagnosticIssue::MissingHelper(Helper::Dark));
    }

    // Upstream checks the global root first, then the local root.
    if !facts.global_ntfs {
        report.issues.push(DiagnosticIssue::NotNtfs {
            global: true,
            path: facts.global_path.clone(),
        });
    }
    if !facts.root_ntfs {
        report.issues.push(DiagnosticIssue::NotNtfs {
            global: false,
            path: facts.root_path.clone(),
        });
    }

    report
}

/// Upstream `check_windows_defender` (`lib/diagnostic.ps1:6-27`): realtime
/// scanning is only a concern while the service runs and the root is not
/// already excluded. Matching is case-insensitive, like PowerShell's
/// `-contains`.
fn defender_performance_issue(defender: &DefenderFacts, install_path: &str) -> bool {
    if defender.realtime_disabled || !defender.service_running {
        return false;
    }
    !defender
        .exclusions
        .iter()
        .any(|excluded| excluded.eq_ignore_ascii_case(install_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The baseline: a healthy environment with the `main` bucket, modern
    /// Windows, both features enabled, all helpers present and NTFS roots.
    fn healthy() -> EnvFacts {
        EnvFacts {
            is_admin: false,
            user_name: "user".to_owned(),
            buckets: vec!["main".to_owned()],
            os_major: 10,
            os_build: 22631,
            long_paths_enabled: true,
            developer_mode_enabled: true,
            helper_7zip: true,
            helper_innounp: true,
            helper_dark: true,
            root_path: r"C:\hok".to_owned(),
            global_path: r"C:\hok-global".to_owned(),
            root_ntfs: true,
            global_ntfs: true,
            defender: None,
        }
    }

    #[test]
    fn healthy_environment_reports_nothing() {
        assert!(evaluate(&healthy()).is_clean());
    }

    #[test]
    fn missing_main_bucket_is_reported() {
        let mut facts = healthy();
        facts.buckets.clear();
        let report = evaluate(&facts);
        assert_eq!(report.issues, vec![DiagnosticIssue::MainBucketMissing]);
    }

    #[test]
    fn old_windows_reports_unsupported_long_paths_not_disabled() {
        let mut facts = healthy();
        facts.os_major = 6;
        facts.os_build = 7601;
        facts.long_paths_enabled = false;
        let report = evaluate(&facts);
        assert!(
            report
                .issues
                .contains(&DiagnosticIssue::LongPathsUnsupported)
        );
        assert!(!report.issues.contains(&DiagnosticIssue::LongPathsDisabled));
    }

    #[test]
    fn modern_windows_with_long_paths_off_reports_disabled() {
        let mut facts = healthy();
        facts.long_paths_enabled = false;
        let report = evaluate(&facts);
        assert!(report.issues.contains(&DiagnosticIssue::LongPathsDisabled));
        assert!(
            !report
                .issues
                .contains(&DiagnosticIssue::LongPathsUnsupported)
        );
    }

    #[test]
    fn developer_mode_off_is_reported() {
        let mut facts = healthy();
        facts.developer_mode_enabled = false;
        let report = evaluate(&facts);
        assert_eq!(report.issues, vec![DiagnosticIssue::DeveloperModeDisabled]);
    }

    #[test]
    fn each_missing_helper_is_reported_in_order() {
        let mut facts = healthy();
        facts.helper_7zip = false;
        facts.helper_innounp = false;
        facts.helper_dark = false;
        let report = evaluate(&facts);
        assert_eq!(
            report.issues,
            vec![
                DiagnosticIssue::MissingHelper(Helper::SevenZip),
                DiagnosticIssue::MissingHelper(Helper::Innounp),
                DiagnosticIssue::MissingHelper(Helper::Dark),
            ]
        );
    }

    #[test]
    fn non_ntfs_roots_are_reported_global_first() {
        let mut facts = healthy();
        facts.root_ntfs = false;
        facts.global_ntfs = false;
        let report = evaluate(&facts);
        assert_eq!(
            report.issues,
            vec![
                DiagnosticIssue::NotNtfs {
                    global: true,
                    path: r"C:\hok-global".to_owned(),
                },
                DiagnosticIssue::NotNtfs {
                    global: false,
                    path: r"C:\hok".to_owned(),
                },
            ]
        );
    }

    /// Defender performance is checked for both roots, but only as admin and
    /// only when realtime scanning runs without an exclusion.
    #[test]
    fn defender_performance_needs_admin_running_service_and_missing_exclusion() {
        let mut facts = healthy();
        facts.is_admin = true;
        facts.defender = Some(DefenderFacts {
            realtime_disabled: false,
            service_running: true,
            exclusions: vec![r"C:\HOK".to_owned()],
        });
        let report = evaluate(&facts);
        // Local root excluded case-insensitively, global root not: one note.
        assert_eq!(
            report.performance,
            vec![DiagnosticIssue::DefenderPerformance {
                global: true,
                path: r"C:\hok-global".to_owned(),
            }]
        );
        assert!(report.issues.is_empty());

        // Not an administrator: Defender is skipped entirely.
        facts.is_admin = false;
        assert!(evaluate(&facts).performance.is_empty());
    }

    #[test]
    fn defender_realtime_disabled_or_service_stopped_is_not_a_problem() {
        let mut facts = healthy();
        facts.is_admin = true;
        facts.defender = Some(DefenderFacts {
            realtime_disabled: true,
            service_running: true,
            exclusions: vec![],
        });
        assert!(evaluate(&facts).performance.is_empty());

        facts.defender = Some(DefenderFacts {
            realtime_disabled: false,
            service_running: false,
            exclusions: vec![],
        });
        assert!(evaluate(&facts).performance.is_empty());
    }

    #[test]
    fn wdag_utility_account_skips_defender() {
        let mut facts = healthy();
        facts.is_admin = true;
        facts.user_name = "WDAGUtilityAccount".to_owned();
        facts.defender = Some(DefenderFacts {
            realtime_disabled: false,
            service_running: true,
            exclusions: vec![],
        });
        assert!(evaluate(&facts).performance.is_empty());
    }
}
