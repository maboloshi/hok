//! Installed-app status aggregation for the `status` command.
//!
//! Mirrors upstream `scoop status` (libexec/scoop-status.ps1) + `app_status`
//! (lib/core.ps1): for every installed app it reports which dimensions need
//! attention across a shared four-axis model:
//!
//! - **Axis 1 (install integrity)** - `broken` (Ok / Broken);
//! - **Axis 2 (source state)** - `SourceState`, collapsed by the shared
//!   `classify` so `status` and `list` can never drift into two predicates;
//! - **Axis 3 (update state)** - `outdated` (Current / Outdated); a source
//!   that cannot be compared (Deprecated / Orphaned / BucketMissing /
//!   UnknownBroken / Isolated) is *Unknown* and is surfaced through the info
//!   words rather than silently left blank;
//! - **Axis 4 (deps + policy)** - `missing_deps`, `held`.
//!
//! Output here is data, never text: the caller (CLI) renders the table and the
//! summary footer. Both `status` and `list` derive their info column from
//! [`AppStatus::info_kinds`], so the word order cannot drift.
//!
//! The heavy lifting (installed enumeration, `current`-junction resolution,
//! version selection, upgradable detection) lives in `query::query_installed`;
//! this module builds on it and adds the source classification.

use std::collections::HashSet;

use crate::Session;
use crate::error::Fallible;
use crate::package::query::query_installed;
use crate::package::source::{SourceState, classify};
use crate::package::{Package, QueryOption};

/// A dimension that can draw an installed app to the user's attention,
/// ordered by the priority used when a row is counted once in the summary
/// footer (highest wins): Broken > source anomaly > missing deps > outdated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attention {
    /// The app is healthy on every axis.
    None,
    /// Axis 1: the install is broken/partial.
    Broken,
    /// Axis 2: the origin bucket no longer carries a live manifest for the
    /// app (Deprecated / Orphaned / BucketMissing - not UnknownBroken, which
    /// is always Broken too).
    Source,
    /// Axis 4: a `depends` entry is not installed.
    MissingDeps,
    /// Axis 3: a newer version exists in the (live) origin bucket.
    Updatable,
}

/// A single info-column word, in a fixed order shared verbatim between the
/// `status` and `list` renderers (via [`AppStatus::info_kinds`]). The CLI
/// maps each kind to a localized string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoKind {
    /// The install is broken/partial (axis 1).
    InstallFailed,
    /// The package is held (axis 4 policy).
    Held,
    /// Source is only under the bucket's `deprecated/` folder (axis 2).
    Deprecated,
    /// The live manifest was deleted from the origin bucket (axis 2).
    Orphaned,
    /// The whole origin bucket directory is gone (axis 2).
    BucketMissing,
    /// Broken install with neither a bucket nor a URL record - source
    /// genuinely unknown, and deliberately not labelled "removed" (axis 2).
    SourceUnknown,
}

/// The per-app status record produced by [`status`] and consumed by `list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppStatus {
    /// Installed app name.
    pub name: String,
    /// Source column value: the origin bucket name, `__isolated__` for
    /// URL/path installs, or empty when the source is genuinely unknown
    /// ([`SourceState::UnknownBroken`]).
    pub bucket: String,
    /// Axis 2: how this app relates to its origin source.
    pub source: SourceState,
    /// Axis 1: the install is broken/partial.
    pub broken: bool,
    /// Installed version (`None` for a broken install).
    pub installed_version: Option<String>,
    /// Axis 3 latest: newest version available from a *live* origin manifest
    /// (`Some` only when outdated; upstream leaves this column blank
    /// otherwise).
    pub latest_version: Option<String>,
    /// Axis 3: a newer version exists in the live origin bucket.
    pub outdated: bool,
    /// Axis 4: the package is held.
    pub held: bool,
    /// Axis 4: names of `depends` entries that are not installed.
    pub missing_deps: Vec<String>,
}

impl AppStatus {
    /// Build the four-axis view of one installed package given its already
    /// computed source state. `missing_deps` is left empty (it requires the
    /// whole installed-name set); [`status`] fills it in.
    pub fn from_package(pkg: &Package, source: SourceState) -> Self {
        let latest = pkg.upgradable_version().map(str::to_owned);
        // A broken install with no record should not masquerade as the
        // `__isolated__` fallback the query layer assigned to it.
        let bucket = if source == SourceState::UnknownBroken {
            String::new()
        } else {
            pkg.bucket().to_owned()
        };
        AppStatus {
            name: pkg.name().to_owned(),
            bucket,
            source,
            broken: pkg.is_install_failed(),
            installed_version: pkg.installed_version().map(str::to_owned),
            latest_version: latest.clone(),
            outdated: latest.is_some(),
            held: pkg.is_held(),
            missing_deps: Vec::new(),
        }
    }

    /// The highest-priority axis that needs the user's attention, used both
    /// to decide the default `status` filter and to count each row once in
    /// the summary footer.
    pub fn attention(&self) -> Attention {
        if self.broken {
            Attention::Broken
        } else if matches!(
            self.source,
            SourceState::Deprecated | SourceState::Orphaned | SourceState::BucketMissing
        ) {
            Attention::Source
        } else if !self.missing_deps.is_empty() {
            Attention::MissingDeps
        } else if self.outdated {
            Attention::Updatable
        } else {
            Attention::None
        }
    }

    /// Whether this app needs to be listed in Scoop's "attention" view.
    /// (Upstream only shows apps that are outdated / failed / deprecated /
    /// removed / missing deps - a held-but-otherwise-healthy app is omitted.)
    pub fn needs_attention(&self) -> bool {
        self.attention() != Attention::None
    }

    /// The info-column words for this app, in a fixed order shared verbatim
    /// by `status` and `list`. Missing-dependency names and the outdated
    /// "-> new" arrow are *not* kinds: `status` shows them in dedicated
    /// columns, `list` renders the arrow as its only update indicator.
    pub fn info_kinds(&self) -> Vec<InfoKind> {
        let mut kinds = Vec::with_capacity(2);
        if self.broken {
            kinds.push(InfoKind::InstallFailed);
        }
        if self.held {
            kinds.push(InfoKind::Held);
        }
        match self.source {
            SourceState::Deprecated => kinds.push(InfoKind::Deprecated),
            SourceState::Orphaned => kinds.push(InfoKind::Orphaned),
            SourceState::BucketMissing => kinds.push(InfoKind::BucketMissing),
            SourceState::UnknownBroken => kinds.push(InfoKind::SourceUnknown),
            SourceState::Synced | SourceState::Isolated => {}
        }
        kinds
    }
}

/// Compute the status of every installed app.
///
/// Enumerates installed apps via [`query_installed`] with `Broken` +
/// `UpgradableCheck` (so partial installs are surfaced and upgradable state
/// is filled without filtering out up-to-date apps), classifies each app's
/// source via [`classify`], then fills in the missing-dependency dimension.
///
/// # Errors
///
/// I/O errors reading the apps or buckets directory propagate.
pub fn status(session: &Session) -> Fallible<Vec<AppStatus>> {
    let packages = query_installed(
        session,
        &["*"],
        &[QueryOption::Broken, QueryOption::UpgradableCheck],
    )?;
    let sources = classify(session, &packages);

    // Installed names in the current scope, used to detect missing deps.
    let mut installed_names = HashSet::new();
    for pkg in &packages {
        installed_names.insert(pkg.name().to_lowercase());
    }

    let mut out = Vec::with_capacity(packages.len());
    for (pkg, source) in packages.iter().zip(sources) {
        let mut s = AppStatus::from_package(pkg, source);
        // Scoop's app_status checks the *explicit* depends field (not the
        // implicit installation helpers) for missing dependencies.
        s.missing_deps = pkg
            .manifest()
            .depends()
            .unwrap_or_default()
            .into_iter()
            .map(|d| crate::package::extract_name(d).to_lowercase())
            .filter(|n| !installed_names.contains(n))
            .collect::<Vec<_>>();
        out.push(s);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{mark_installed, test_session, tmpdir, write_bucket_manifest};

    const MINI: &str =
        "{\"version\": \"1.0.0\", \"homepage\": \"https://e.com\", \"license\": \"MIT\"}";
    const MINI_V2: &str =
        "{\"version\": \"2.0.0\", \"homepage\": \"https://e.com\", \"license\": \"MIT\"}";

    /// Create a broken, source-unknown install: a `current/manifest.json`
    /// (so the app dir is enumerated) with no `install.json` (so the source
    /// bucket cannot be read back -> UnknownBroken).
    fn install_broken(root: &std::path::Path, name: &str) {
        let current = root.join("apps").join(name).join("current");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("manifest.json"), MINI).unwrap();
    }

    fn install(root: &std::path::Path, name: &str, bucket: &str, version: &str) {
        mark_installed(
            root,
            name,
            bucket,
            &format!(
                "{{\"version\": \"{}\", \"homepage\": \"https://e.com\", \"license\": \"MIT\"}}",
                version
            ),
            false,
        );
    }

    #[test]
    fn empty_scope_yields_no_status() {
        let root = tmpdir("status_empty");
        let session = test_session(&root);
        assert!(status(&session).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn up_to_date_app_is_healthy() {
        let root = tmpdir("status_healthy");
        let session = test_session(&root);
        write_bucket_manifest(&root, "main", "7zip", MINI);
        install(&root, "7zip", "main", "1.0.0");
        let list = status(&session).unwrap();
        assert_eq!(list.len(), 1);
        let s = &list[0];
        assert_eq!(s.name, "7zip");
        assert_eq!(s.installed_version.as_deref(), Some("1.0.0"));
        assert_eq!(s.source, SourceState::Synced);
        assert!(!s.broken && !s.outdated && s.missing_deps.is_empty());
        assert!(!s.needs_attention());
        assert_eq!(s.info_kinds(), vec![]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn outdated_app_reports_latest() {
        let root = tmpdir("status_outdated");
        let session = test_session(&root);
        write_bucket_manifest(&root, "main", "7zip", MINI_V2);
        install(&root, "7zip", "main", "1.0.0");
        let list = status(&session).unwrap();
        let s = &list[0];
        assert!(s.outdated);
        assert_eq!(s.source, SourceState::Synced);
        assert_eq!(s.latest_version.as_deref(), Some("2.0.0"));
        assert_eq!(s.attention(), Attention::Updatable);
        assert!(s.needs_attention());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn deprecated_manifest_flagged_not_orphaned() {
        let root = tmpdir("status_deprecated");
        let session = test_session(&root);
        let dir = root.join("buckets").join("main").join("deprecated");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("legacy.json"), MINI).unwrap();
        install(&root, "legacy", "main", "1.0.0");
        let list = status(&session).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].source, SourceState::Deprecated);
        assert_eq!(list[0].info_kinds(), vec![InfoKind::Deprecated]);
        assert_eq!(list[0].attention(), Attention::Source);
        assert!(list[0].needs_attention());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn live_manifest_removed_is_orphaned() {
        let root = tmpdir("status_orphaned");
        let session = test_session(&root);
        install(&root, "gone", "main", "1.0.0");
        std::fs::create_dir_all(root.join("buckets").join("main")).unwrap();
        let list = status(&session).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].source, SourceState::Orphaned);
        assert_eq!(list[0].attention(), Attention::Source);
        assert!(list[0].needs_attention());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn whole_bucket_gone_is_bucket_missing() {
        let root = tmpdir("status_bucket_missing");
        let session = test_session(&root);
        install(&root, "gone", "main", "1.0.0");
        // No buckets/main directory at all -> the bucket itself is gone.
        let list = status(&session).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].source, SourceState::BucketMissing);
        assert_eq!(list[0].bucket, "main");
        assert_eq!(list[0].attention(), Attention::Source);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn broken_without_record_is_unknown_broken() {
        let root = tmpdir("status_unknown_broken");
        let session = test_session(&root);
        // A `current` with no install.json -> broken package with no source
        // record -> UnknownBroken, not "removed".
        install_broken(&root, "fiddler");
        let list = status(&session).unwrap();
        assert_eq!(list.len(), 1);
        let s = &list[0];
        assert_eq!(s.name, "fiddler");
        assert!(s.broken);
        assert_eq!(s.source, SourceState::UnknownBroken);
        assert!(s.bucket.is_empty(), "source unknown -> no bucket claimed");
        assert_eq!(s.attention(), Attention::Broken);
        assert_eq!(
            s.info_kinds(),
            vec![InfoKind::InstallFailed, InfoKind::SourceUnknown]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_dep_reported() {
        let root = tmpdir("status_missing_dep");
        let session = test_session(&root);
        let deps = "{\"version\": \"1.0.0\", \"homepage\": \"https://e.com\", \"license\": \"MIT\", \"depends\": [\"helper\"]}";
        write_bucket_manifest(&root, "main", "a", deps);
        mark_installed(&root, "a", "main", deps, false);
        let list = status(&session).unwrap();
        let s = &list[0];
        assert_eq!(s.missing_deps, vec!["helper"]);
        assert_eq!(s.attention(), Attention::MissingDeps);
        assert!(s.needs_attention());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn satisfied_dep_not_reported() {
        let root = tmpdir("status_satisfied_dep");
        let session = test_session(&root);
        let deps = "{\"version\": \"1.0.0\", \"homepage\": \"https://e.com\", \"license\": \"MIT\", \"depends\": [\"helper\"]}";
        write_bucket_manifest(&root, "main", "a", deps);
        mark_installed(&root, "a", "main", deps, false);
        install(&root, "helper", "main", "1.0.0");
        let list = status(&session).unwrap();
        let a = list.iter().find(|s| s.name == "a").unwrap();
        assert!(a.missing_deps.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn isolated_install_not_removed_or_deprecated() {
        let root = tmpdir("status_isolated");
        let session = test_session(&root);
        let current = root.join("apps").join("portable").join("current");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("manifest.json"), MINI).unwrap();
        std::fs::write(
            current.join("install.json"),
            "{\"architecture\": \"64bit\", \"url\": \"https://e.com/portable.json\"}",
        )
        .unwrap();
        let list = status(&session).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].source, SourceState::Isolated);
        assert!(!list[0].broken && !list[0].outdated);
        assert_eq!(list[0].attention(), Attention::None);
        assert!(!list[0].needs_attention());
        let _ = std::fs::remove_dir_all(&root);
    }

    // Reproduces the user's real status/list dataset so the shared footer
    // semantics are pinned: 8 items needing attention summing to 8.
    //
    //   WorkBuddy       Synced + outdated                 -> Updatable
    //   hp              origin bucket directory gone      -> BucketMissing
    //   GuoheView       live manifest removed            -> Orphaned
    //   Reqable         live manifest removed            -> Orphaned
    //   Fiddler/potplayer/sublime-merge/sublime-text
    //                   broken with no source record     -> UnknownBroken
    #[test]
    fn acceptance_footer_counts_sum_to_rows() {
        let root = tmpdir("status_acceptance");
        let session = test_session(&root);

        // WorkBuddy: extras bucket exists and carries a live, newer manifest.
        write_bucket_manifest(&root, "extras", "workbuddy", MINI_V2);
        install(&root, "WorkBuddy", "extras", "1.0.0");
        // GuoheView / Reqable: same live bucket, but no manifest for them.
        install(&root, "GuoheView", "extras", "1.0.0");
        install(&root, "Reqable", "extras", "1.0.0");
        // hp: recorded origin bucket "main" does not exist at all.
        install(&root, "hp", "main", "1.0.0");
        // Broken, source-unknown apps.
        for name in ["Fiddler", "potplayer", "sublime-merge", "sublime-text"] {
            install_broken(&root, name);
        }

        let list = status(&session).unwrap();
        assert_eq!(list.len(), 8);

        let by = |n: &str| {
            list.iter()
                .find(|s| s.name.eq_ignore_ascii_case(n))
                .unwrap_or_else(|| panic!("no status row for {n}"))
        };

        let wb = by("WorkBuddy");
        assert_eq!(wb.source, SourceState::Synced);
        assert!(wb.outdated);
        assert_eq!(wb.attention(), Attention::Updatable);

        assert_eq!(by("hp").source, SourceState::BucketMissing);
        assert_eq!(by("hp").attention(), Attention::Source);
        assert_eq!(by("GuoheView").source, SourceState::Orphaned);
        assert_eq!(by("Reqable").source, SourceState::Orphaned);

        for name in ["Fiddler", "potplayer", "sublime-merge", "sublime-text"] {
            let s = by(name);
            assert!(s.broken, "{name} should be broken");
            assert_eq!(s.source, SourceState::UnknownBroken);
            assert_eq!(s.attention(), Attention::Broken);
        }

        // Every row needs attention and is counted exactly once, by its
        // highest-priority category: Broken(4) > Source(3) > MissingDeps(0)
        // > Updatable(1). The four numbers sum to the eight rows.
        assert_eq!(list.iter().filter(|s| s.needs_attention()).count(), 8);
        assert_eq!(list.iter().filter(|s| s.broken).count(), 4);
        assert_eq!(
            list.iter()
                .filter(|s| s.attention() == Attention::Source)
                .count(),
            3
        );
        assert_eq!(
            list.iter()
                .filter(|s| s.attention() == Attention::MissingDeps)
                .count(),
            0
        );
        assert_eq!(
            list.iter()
                .filter(|s| s.attention() == Attention::Updatable)
                .count(),
            1
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
