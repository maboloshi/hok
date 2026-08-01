//! Persistent data directory handling primitives.

use tracing::debug;

use crate::{error::Fallible, internal, package::Package, Event, Session};

/// Link persistent data directories for an installed package.
pub fn persist_link(session: &Session, pkg: &Package) -> Fallible<()> {
    crate::persist::link(session, pkg)
}

/// Unlink persistent data directories of an uninstalled package.
pub fn persist_unlink(session: &Session, pkg: &Package) -> Fallible<()> {
    crate::persist::unlink(session, pkg)
}

/// Purge the persistent data directory of `pkg_name`.
pub fn persist_purge(session: &Session, pkg_name: &str) -> Fallible<()> {
    debug!("remove: {} - purging persist data", pkg_name);
    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackagePersistPurgeStart);
    }
    let persist_dir = session.config().root_path().join("persist").join(pkg_name);
    internal::fs::remove_dir(persist_dir)?;
    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackagePersistPurgeDone);
    }
    Ok(())
}
