//! Shortcut (.lnk) creation & removal primitives.

use crate::{error::Fallible, package::Package, Session};

/// Create the shortcuts declared by an installed package.
pub fn shortcut_add(session: &Session, pkg: &Package) -> Fallible<()> {
    crate::shortcut::add(session, pkg)
}

/// Remove the shortcuts declared by an uninstalled package.
pub fn shortcut_remove(session: &Session, pkg: &Package) -> Fallible<()> {
    crate::shortcut::remove(session, pkg)
}
