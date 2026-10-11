//! Hold / unhold installed packages.
//!
//! Toggles the `held` flag in the package's install record
//! (`scoop-install.json`, else the legacy `install.json`), preventing
//! accidental upgrades — mirroring Scoop's `hold` / `unhold` commands.

use crate::{
    Error, Session,
    error::Fallible,
    internal,
    package::install_state::{self, parse_install_info},
};

/// Result of a hold/unhold toggle.
///
/// Mirrors Scoop's per-package branches in `scoop-hold.ps1` / `scoop-unhold.ps1`:
/// an already-held package reports "already held" instead of rewriting state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldResult {
    /// The `held` flag was actually changed and persisted.
    Changed,
    /// The package already had the requested state; nothing was written.
    Unchanged,
}

/// Hold or unhold a package.
///
/// # Errors
///
/// This method will return an error if the package is not installed.
///
/// A [`PackageHoldBrokenInstall`][1] error will be returned if the install is
/// broken (no install record, or it cannot be parsed).
///
/// I/O errors will be returned if failed to write the install record file.
/// Serde errors will be returned if the install info cannot be serialized.
///
/// [1]: crate::Error::PackageHoldBrokenInstall
pub fn hold(session: &Session, name: &str, flag: bool) -> Fallible<HoldResult> {
    let app_dir = session.app_dir(name);
    if !app_dir.exists() {
        return Err(Error::PackageHoldNotInstalled(name.to_owned()));
    }

    // Upstream scoop-hold.ps1/scoop-unhold.ps1 write
    // `versiondir $app (NO_JUNCTION ? Select-CurrentVersion : 'current')` —
    // i.e. the resolved version dir when there is no junction to write
    // through. The install record therefore lives outside `current` under
    // NO_JUNCTION.
    let Some(meta_dir) = session.app_current_dir(name) else {
        return Err(Error::PackageHoldBrokenInstall(name.to_owned()));
    };

    // Read through the new-first resolver, but write the file upstream's
    // `save_install_info` writes (`scoop-install.json`, lib/manifest.ps1:141-147)
    // — so holding a legacy install converts it, exactly as Scoop >= 0.6.0 does.
    let Some(read_path) = install_state::install_info_path(&meta_dir) else {
        return Err(Error::PackageHoldBrokenInstall(name.to_owned()));
    };
    let Ok(mut install_info) = parse_install_info(&read_path) else {
        return Err(Error::PackageHoldBrokenInstall(name.to_owned()));
    };
    if install_info.is_held() == flag {
        return Ok(HoldResult::Unchanged);
    }
    install_info.set_held(flag);
    internal::fs::write_json(
        meta_dir.join(install_state::SCOOP_INSTALL_JSON),
        install_info,
    )?;
    Ok(HoldResult::Changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a temp root with a session rooted at it, plus a drop guard.
    fn setup(test_name: &str) -> (Session, PathBufGuard) {
        let root = crate::test_utils::tmpdir(&format!("hold_{}", test_name));
        let session = crate::test_utils::test_session(&root);
        (session, PathBufGuard(root))
    }

    struct PathBufGuard(std::path::PathBuf);
    impl Drop for PathBufGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Create `apps/<name>/current/scoop-install.json` with the given raw
    /// content (the name hok and Scoop >= 0.6.0 write).
    fn write_install_json(root: &std::path::Path, name: &str, content: &str) {
        write_named_install_json(root, name, "scoop-install.json", content);
    }

    /// [`write_install_json`] with an explicit file name.
    fn write_named_install_json(root: &std::path::Path, name: &str, file: &str, content: &str) {
        let current = root.join("apps").join(name).join("current");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join(file), content).unwrap();
    }

    #[test]
    fn hold_sets_and_clears_flag() {
        let (session, root) = setup("roundtrip");
        write_install_json(
            &root.0,
            "app",
            r#"{"architecture": "64bit", "bucket": "main"}"#,
        );

        // hold
        hold(&session, "app", true).unwrap();
        let info = parse_install_info(&root.0.join("apps/app/current/scoop-install.json")).unwrap();
        assert!(info.is_held(), "held flag should be set");

        // unhold
        hold(&session, "app", false).unwrap();
        let info = parse_install_info(&root.0.join("apps/app/current/scoop-install.json")).unwrap();
        assert!(!info.is_held(), "held flag should be cleared");
    }

    /// A legacy-only install (only `install.json`) is read through the old
    /// name and, like upstream `save_install_info`, re-written under the new
    /// `scoop-install.json` — other fields survive the conversion.
    #[test]
    fn hold_legacy_install_reads_old_and_writes_new() {
        let (session, root) = setup("legacy_conversion");
        write_named_install_json(
            &root.0,
            "app",
            "install.json",
            r#"{"architecture": "32bit", "bucket": "main"}"#,
        );

        assert_eq!(hold(&session, "app", true).unwrap(), HoldResult::Changed);

        let current = root.0.join("apps/app/current");
        let info = parse_install_info(&current.join("scoop-install.json")).unwrap();
        assert_eq!(info.arch(), "32bit", "other fields kept");
        assert_eq!(info.bucket(), Some("main"));
        assert!(info.is_held());
        assert!(
            current.join("install.json").exists(),
            "the legacy file is not deleted"
        );
    }

    #[test]
    fn hold_keeps_other_install_fields() {
        let (session, root) = setup("keeps_fields");
        write_install_json(
            &root.0,
            "app",
            r#"{"architecture": "32bit", "bucket": "main"}"#,
        );

        hold(&session, "app", true).unwrap();

        let info = parse_install_info(&root.0.join("apps/app/current/scoop-install.json")).unwrap();
        assert_eq!(info.arch(), "32bit");
        assert_eq!(info.bucket(), Some("main"));
        assert!(info.is_held());
    }

    #[test]
    fn hold_idempotent_reports_unchanged() {
        let (session, root) = setup("idempotent");
        write_install_json(&root.0, "app", r#"{"architecture": "64bit"}"#);

        assert_eq!(hold(&session, "app", true).unwrap(), HoldResult::Changed);
        assert_eq!(hold(&session, "app", true).unwrap(), HoldResult::Unchanged);
        assert_eq!(hold(&session, "app", false).unwrap(), HoldResult::Changed);
        assert_eq!(hold(&session, "app", false).unwrap(), HoldResult::Unchanged);
    }

    #[test]
    fn hold_not_installed_errors() {
        let (session, _root) = setup("not_installed");

        let err = hold(&session, "ghost", true).unwrap_err();

        assert!(matches!(err, Error::PackageHoldNotInstalled(name) if name == "ghost"));
    }

    #[test]
    fn hold_missing_install_json_errors() {
        let (session, root) = setup("broken_missing");
        std::fs::create_dir_all(root.0.join("apps/app/current")).unwrap();

        let err = hold(&session, "app", true).unwrap_err();

        assert!(matches!(err, Error::PackageHoldBrokenInstall(name) if name == "app"));
    }

    /// Under NO_JUNCTION there is no `current` junction, so the install record
    /// lives in the version dir — upstream writes
    /// `versiondir $app (Select-CurrentVersion)` there. Hardcoding `current`
    /// made `hold`/`unhold` fail on every no-junction install. The record is
    /// rewritten under the new name there too.
    #[test]
    fn hold_no_junction_writes_version_dir_install_record() {
        let root = crate::test_utils::tmpdir("hold_no_junction");
        let session = crate::test_utils::test_session_with_junctions(&root, true);
        let vdir = root.join("apps").join("app").join("1.2.3");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(
            vdir.join("install.json"),
            r#"{"architecture": "64bit", "bucket": "main"}"#,
        )
        .unwrap();
        std::fs::write(vdir.join("manifest.json"), r#"{"version": "1.2.3"}"#).unwrap();

        assert_eq!(hold(&session, "app", true).unwrap(), HoldResult::Changed);

        let info = parse_install_info(&vdir.join("scoop-install.json")).unwrap();
        assert!(info.is_held(), "held flag must land in the version dir");
        assert!(
            !root.join("apps/app/current").exists(),
            "no junction should be created"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A no-junction app whose version dir carries no install record cannot be
    /// held — reported as a broken install, not as "not installed".
    #[test]
    fn hold_no_junction_missing_metadata_is_broken() {
        let root = crate::test_utils::tmpdir("hold_no_junction_broken");
        let session = crate::test_utils::test_session_with_junctions(&root, true);
        std::fs::create_dir_all(root.join("apps").join("app").join("1.2.3")).unwrap();

        let err = hold(&session, "app", true).unwrap_err();

        assert!(matches!(err, Error::PackageHoldBrokenInstall(name) if name == "app"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn hold_corrupted_install_json_errors() {
        let (session, root) = setup("broken_corrupt");
        write_install_json(&root.0, "app", "not-json{");

        let err = hold(&session, "app", true).unwrap_err();

        assert!(matches!(err, Error::PackageHoldBrokenInstall(name) if name == "app"));
    }
}
