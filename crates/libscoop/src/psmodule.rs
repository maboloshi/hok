//! PowerShell module management for package lifecycle.
//!
//! Removes PowerShell modules that were installed by a package during
//! uninstall/cleanup.
//!
//! # Design
//!
//! - **Single operation**: Currently only supports `remove()`. Module
//!   installation happens during package sync (in `sync.rs`) by copying
//!   the module from the manifest.
//! - **Event emission**: Fires `PackagePsModuleRemoveStart/Done` events.

use crate::{error::Fallible, package::Package, Event, Session};

/// Remove PowerShell module imported by a given package.
pub fn remove(session: &Session, package: &Package) -> Fallible<()> {
    assert!(package.is_installed());

    if let Some(psmodule) = package.manifest().psmodule() {
        let mut psmodule_path = session.effective_root_path().join("modules");

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackagePsModuleRemoveStart(
                psmodule.name().to_owned(),
            ));
        }

        psmodule_path.push(psmodule.name());
        let _ = std::fs::remove_dir(psmodule_path);

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackagePsModuleRemoveDone);
        }
    }
    Ok(())
}
