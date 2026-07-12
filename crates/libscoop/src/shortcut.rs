use std::sync::LazyLock;
use std::path::{Path, PathBuf};

use crate::{error::Fallible, internal, package::Package, Event, Session};

static SCOOP_SHORTCUT_DIR: LazyLock<PathBuf> = LazyLock::new(shortcut_dir);

/// Return the path to the shortcut directory.
///
/// `~\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Scoop Apps`
fn shortcut_dir() -> PathBuf {
    let mut dir = dirs::config_dir().unwrap();
    dir.push("Microsoft/Windows/Start Menu/Programs/Scoop Apps");
    internal::path::normalize_path(dir)
}

/// Add shortcut(s) for a given package.
#[cfg(windows)]
pub fn add(session: &Session, package: &Package) -> Fallible<()> {
    if let Some(shortcuts) = package.manifest().shortcuts() {
        let config = session.config();
        let apps_dir = config.root_path().join("apps");

        // Ensure shortcut dir exists
        internal::fs::ensure_dir(&*SCOOP_SHORTCUT_DIR)?;

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageShortcutAddStart);
        }

        for shortcut in shortcuts {
            let length = shortcut.len();
            assert!(length > 1);

            // Scoop shortcut format:
            //   [0] = target exe (relative to package dir)
            //   [1] = display name in start menu
            //   [2] = optional arguments
            //   [3] = optional icon path (relative to package dir)
            let target = apps_dir.join(package.name()).join("current").join(shortcut[0]);
            let target_str = target.to_string_lossy().into_owned();

            let args = shortcut.get(2).map(|s| s.to_string());
            let icon = shortcut.get(3).map(|s| apps_dir.join(package.name()).join("current").join(s).to_string_lossy().into_owned());

            let mut link_path = SCOOP_SHORTCUT_DIR.join(shortcut[1]);
            link_path.set_extension("lnk");

            create_shortcut(&target_str, &link_path, args, icon)?;

            if let Some(tx) = session.emitter() {
                let name = link_path.file_name().unwrap().to_str().unwrap().to_owned();
                let _ = tx.send(Event::PackageShortcutAddProgress(name));
            }
        }

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageShortcutAddDone);
        }
    }

    Ok(())
}

/// Create a `.lnk` shortcut file using `shortcuts-rs` (pure Rust LNK writer).
#[cfg(windows)]
fn create_shortcut(
    target: &str,
    link: &Path,
    args: Option<String>,
    icon: Option<String>,
) -> std::io::Result<()> {
    use shortcuts_rs::ShellLink;

    let link_str = link.to_string_lossy();
    // The display name is derived from the .lnk filename (minus .lnk extension)
    let name = link.file_stem().and_then(|s| s.to_str()).map(|s| s.to_owned());

    let sl = ShellLink::new(target, args, name, icon)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    sl.create_lnk(link_str.as_ref())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
}

/// Remove shortcut(s) for a given package.
#[cfg(not(windows))]
pub fn add(_session: &Session, _package: &Package) -> Fallible<()> {
    Ok(())
}

/// Remove shortcut(s) for a given package.
pub fn remove(session: &Session, package: &Package) -> Fallible<()> {
    assert!(package.is_installed());

    if let Some(shortcuts) = package.manifest().shortcuts() {
        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageShortcutRemoveStart);
        }

        for shortcut in shortcuts {
            let length = shortcut.len();
            assert!(length > 1);

            let mut path = SCOOP_SHORTCUT_DIR.join(shortcut[1]);
            path.set_extension("lnk");

            if let Some(tx) = session.emitter() {
                let shortcut_name = path.file_name().unwrap().to_str().unwrap().to_owned();
                let _ = tx.send(Event::PackageShortcutRemoveProgress(shortcut_name));
            }

            let _ = std::fs::remove_file(&path);
        }

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageShortcutRemoveDone);
        }
    }
    Ok(())
}

#[cfg(all(windows, test))]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_create_shortcut_to_exe() {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let target = Path::new(&system_root).join("System32\\cmd.exe");
        let link = std::env::temp_dir().join("hok_test_shortcut.lnk");
        let target_str = target.to_string_lossy().into_owned();

        let _ = std::fs::remove_file(&link);

        let result = create_shortcut(&target_str, &link, None, None);
        assert!(result.is_ok(), "create_shortcut failed: {:?}", result.err());
        assert!(link.exists(), ".lnk file was not created");

        let bytes = std::fs::read(&link).unwrap();
        assert_eq!(&bytes[..4], &[0x4C, 0x00, 0x00, 0x00], "not a valid LNK header");

        let _ = std::fs::remove_file(&link);
    }

    #[test]
    fn test_create_shortcut_with_args_and_icon() {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let target = Path::new(&system_root).join("System32\\cmd.exe");
        let link = std::env::temp_dir().join("hok_test_shortcut_args.lnk");
        let target_str = target.to_string_lossy().into_owned();

        let _ = std::fs::remove_file(&link);

        let result = create_shortcut(
            &target_str,
            &link,
            Some("/k echo hello".into()),
            Some(target_str.clone()),
        );
        assert!(result.is_ok(), "create_shortcut with args/icon failed: {:?}", result.err());
        assert!(link.exists(), ".lnk file was not created");

        let _ = std::fs::remove_file(&link);
    }
}
