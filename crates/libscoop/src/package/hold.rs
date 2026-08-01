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
