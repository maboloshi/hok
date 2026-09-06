//! Reset a package to its current version.

use clap::Parser;
use libscoop::{EventHandler, Session, package};

use crate::{Result, output};

/// Reset an app to resolve conflicts (reapply shims, shortcuts, post_install)
#[derive(Debug, Parser)]
pub struct Args {
    /// The app name
    app: Option<String>,
    /// A specific version to reset to
    version: Option<String>,
    /// Reset all installed apps (equivalent to `hok reset *`)
    #[arg(short = 'a', long, action = clap::ArgAction::SetTrue)]
    all: bool,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    // `*` is a shorthand for `-a/--all` (Scoop: `$apps -eq '*' -or $all`)
    let all = args.all || args.app.as_deref() == Some("*");
    if args.app.is_none() && !all {
        return Err(anyhow::anyhow!(rust_i18n::t!("cmd.reset_app_missing")));
    }

    if all {
        // Reset every installed app — local first, then global (mirrors
        // scoop-reset.ps1 `installed_apps $false` + `installed_apps $true`).
        //
        // First collect the ordered attempts (skipping 'scoop' and non-admin
        // globals, which warn), so the `(k/N)` commit counter spans the whole
        // batch instead of restarting at `(1/1)` per package.
        let mut attempts: Vec<(bool, String)> = Vec::new();
        for global in [false, true] {
            session.set_global(global);
            // `Broken` is included so a partial install shows up and is
            // either repaired (if a version resolves) or reported as broken,
            // mirroring Scoop's `installed_apps` (which lists every app dir).
            let pkgs =
                package::query::query_installed(session, &["*"], &[libscoop::QueryOption::Broken])?;
            for pkg in &pkgs {
                if pkg.name() == "scoop" {
                    // Official scoop-reset.ps1 skips 'scoop' itself.
                    continue;
                }
                // Official warns and skips global apps without admin rights.
                if global && !session.is_admin() {
                    output::warn(rust_i18n::t!("cmd.reset_admin_skip", name = pkg.name()));
                    continue;
                }
                attempts.push((global, pkg.name().to_owned()));
            }
        }
        let total = attempts.len();
        for (k, (global, name)) in attempts.into_iter().enumerate() {
            // Restore the scope this package was collected under before
            // resetting it (paths resolve against the session's scope).
            session.set_global(global);
            // Official scoop-reset.ps1 iterates apps independently
            // (ForEach-Object with `return`): a failing app is reported
            // and skipped, the remaining apps keep resetting.
            if let Err(e) = package::sync::reset(session, &name, None, k + 1, total).await {
                output::err(rust_i18n::t!("cmd.reset_failed", name = name, e = e));
                continue;
            }
        }
        // Leave the session back in the default (user-level) state.
        session.set_global(false);
        drain_steps(session);
        return Ok(());
    }

    let name = args.app.unwrap();
    let version = args.version.as_deref();
    package::sync::reset(session, &name, version, 1, 1).await?;
    drain_steps(session);
    Ok(())
}

/// `reset` runs synchronously (no async event loop — unlike install/update it
/// needs no progress bars or prompts), so the lifecycle `PackageStep` events it
/// emitted sit in the event bus until now. Drain them into the default handler
/// so verbose step output (`-v`) renders for reset just like the other verbs.
fn drain_steps(session: &Session) {
    let Some(mut rx) = session.event_bus().receiver() else {
        return;
    };
    let mut handler = crate::scoop_handler::ScoopHandler::new();
    while let Ok(event) = rx.try_recv() {
        handler.handle(&event);
    }
    handler.on_finished();
}
