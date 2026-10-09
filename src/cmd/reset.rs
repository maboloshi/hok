//! Reset a package to its current version.

use clap::Parser;
use libscoop::{EventHandler, Session, package};

use crate::Result;

/// Reset an app to resolve conflicts (reapply shims, shortcuts, post_install)
#[derive(Debug, Parser)]
pub struct Args {
    /// The app(s) to reset: `app`, `bucket/app` or `app@version`
    app: Vec<String>,
    /// Reset all installed apps (equivalent to `hok reset *`)
    #[arg(short = 'a', long, action = clap::ArgAction::SetTrue)]
    all: bool,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    // `*` is a shorthand for `-a/--all` (Scoop: `$apps -eq '*' -or $all`), and
    // like upstream it replaces the app list instead of adding to it.
    let all = args.all || args.app.iter().any(|app| app == "*");
    if args.app.is_empty() && !all {
        return Err(anyhow::anyhow!(hok_i18n::t!("cmd.reset_app_missing")));
    }

    // Every positional argument is an app (`scoop-reset.ps1:16,31`); parsing,
    // multi-app error+continue, global auto-detection and the `(k/N)` counter
    // live in the pipeline, shared with the `-a` path.
    let queries: Vec<&str> = if all {
        Vec::new()
    } else {
        args.app.iter().map(String::as_str).collect()
    };
    package::sync::reset(session, &queries, all).await?;
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
