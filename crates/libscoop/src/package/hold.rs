//! Hold / unhold installed packages.
//!
//! Toggles the `held` flag in the package's `install.json`, preventing
//! accidental upgrades — mirroring Scoop's `hold` / `unhold` commands.

use crate::{error::Fallible, internal, package::InstallInfo, Error, Session};

/// Hold or unhold a package.
///
/// # Errors
///
/// This method will return an error if the package is not installed.
///
/// A [`PackageHoldBrokenInstall`][1] error will be returned if the install is
/// broken (`install.json` is missing or broken).
///
/// I/O errors will be returned if failed to write the `install.json` file.
/// Serde errors will be returned if the install info cannot be serialized.
///
/// [1]: crate::Error::PackageHoldBrokenInstall
pub fn hold(session: &Session, name: &str, flag: bool) -> Fallible<()> {
    let mut path = session.effective_root_path();
    path.push("apps");
    path.push(name);

    if !path.exists() {
        return Err(Error::PackageHoldNotInstalled(name.to_owned()));
    }

    path.push("current");
    path.push("install.json");

    if let Ok(mut install_info) = InstallInfo::parse(&path) {
        install_info.set_held(flag);
        internal::fs::write_json(path, install_info)
    } else {
        Err(Error::PackageHoldBrokenInstall(name.to_owned()))
    }
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

    /// Create `apps/<name>/current/install.json` with the given raw content.
    fn write_install_json(root: &std::path::Path, name: &str, content: &str) {
        let current = root.join("apps").join(name).join("current");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("install.json"), content).unwrap();
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
        let info = InstallInfo::parse(root.0.join("apps/app/current/install.json")).unwrap();
        assert!(info.is_held(), "held flag should be set");

        // unhold
        hold(&session, "app", false).unwrap();
        let info = InstallInfo::parse(root.0.join("apps/app/current/install.json")).unwrap();
        assert!(!info.is_held(), "held flag should be cleared");
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

        let info = InstallInfo::parse(root.0.join("apps/app/current/install.json")).unwrap();
        assert_eq!(info.arch(), "32bit");
        assert_eq!(info.bucket(), Some("main"));
        assert!(info.is_held());
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

    #[test]
    fn hold_corrupted_install_json_errors() {
        let (session, root) = setup("broken_corrupt");
        write_install_json(&root.0, "app", "not-json{");

        let err = hold(&session, "app", true).unwrap_err();

        assert!(matches!(err, Error::PackageHoldBrokenInstall(name) if name == "app"));
    }
}
