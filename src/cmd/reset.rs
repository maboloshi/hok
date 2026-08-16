//! Reset a package to its current version.

use clap::Parser;
use libscoop::{package, Session};

use crate::{output, Result};

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

pub fn execute(args: Args, session: &Session) -> Result<()> {
    // `*` is a shorthand for `-a/--all` (Scoop: `$apps -eq '*' -or $all`)
    let all = args.all || args.app.as_deref() == Some("*");
    if args.app.is_none() && !all {
        return Err(anyhow::anyhow!(rust_i18n::t!("cmd.reset_app_missing")));
    }

    if all {
        // Reset every installed app — local first, then global (mirrors
        // scoop-reset.ps1 `installed_apps $false` + `installed_apps $true`).
        for global in [false, true] {
            session.set_global(global);
            let pkgs = package::query::query_installed(session, &["*"], &[])?;
            for pkg in &pkgs {
                if pkg.name() == "scoop" {
                    // Official scoop-reset.ps1 skips 'scoop' itself.
                    continue;
                }
                // Official warns and skips global apps without admin rights.
                if global && !session.is_admin() {
                    output::warn(format!(
                        "'{}' is a global app. You need admin rights to reset it. Skipping.",
                        pkg.name()
                    ));
                    continue;
                }
                // Official scoop-reset.ps1 iterates apps independently
                // (ForEach-Object with `return`): a failing app is reported
                // and skipped, the remaining apps keep resetting.
                if let Err(e) = package::sync::reset(session, pkg.name(), None) {
                    output::err(format!("Failed to reset '{}': {e}", pkg.name()));
                    continue;
                }
            }
        }
        // Leave the session back in the default (user-level) state.
        session.set_global(false);
        return Ok(());
    }

    let name = args.app.unwrap();
    let version = args.version.as_deref();
    package::sync::reset(session, &name, version)?;
    Ok(())
}
