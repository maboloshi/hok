//! Hold / unhold installed packages.
//!
//! Both commands share the same batch loop — only the `held` flag value and
//! the progress message differ. The shared logic lives in [`hold_packages`];
//! `hold` calls it with `true`, `unhold` (thin shell in `unhold.rs`) with
//! `false`.

use clap::{ArgAction, Parser};
use libscoop::{package, Error, Session};

use crate::cmd::shared_args::ensure_global;
use crate::{output, Result};

/// Hold package(s) to disable changes
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// The package(s) to be held
    #[arg(required = true, action = ArgAction::Append)]
    package: Vec<String>,
    /// Hold globally installed app (from $SCOOP_GLOBAL)
    #[arg(short = 'g', long, action = ArgAction::SetTrue)]
    global: bool,
}

/// Toggle the `held` flag for a batch of packages, reporting progress per
/// package. Shared by `hold` / `unhold`: `flag` is `true` to hold and
/// `false` to unhold; `action` is the translated progress message.
pub(super) fn hold_packages(
    session: &Session,
    names: &[String],
    flag: bool,
    action: impl AsRef<str>,
) -> Result<()> {
    for name in names {
        output::progress(&action, name);
        match package::hold::hold(session, name, flag) {
            Ok(package::hold::HoldResult::Changed) => output::ok(),
            // Already in the requested state: report like Scoop's
            // "'$app' is already held." / "'$app' is not held." and move on.
            Ok(package::hold::HoldResult::Unchanged) => {
                let msg = if flag {
                    rust_i18n::t!("cmd.already_held", name = name)
                } else {
                    rust_i18n::t!("cmd.not_held", name = name)
                };
                output::info(msg);
            }
            // Not installed / broken install.json: report (distinguishing
            // global scope like Scoop) and keep processing the remaining
            // packages — Scoop uses error+continue for both, exiting 0.
            Err(Error::PackageHoldNotInstalled(_)) => {
                let msg = if session.is_global() {
                    rust_i18n::t!("cmd.hold_not_installed_global", name = name)
                } else {
                    rust_i18n::t!("cmd.hold_not_installed", name = name)
                };
                output::err(msg);
                continue;
            }
            Err(Error::PackageHoldBrokenInstall(_)) => {
                let msg = if flag {
                    rust_i18n::t!("cmd.hold_failed", name = name)
                } else {
                    rust_i18n::t!("cmd.unhold_failed", name = name)
                };
                output::err(msg);
                continue;
            }
            // I/O or serialization failure: propagate like Scoop's uncaught
            // exception in save_install_info (aborts with a non-zero code).
            Err(err) => {
                output::err(rust_i18n::t!("cmd.hold_err"));
                return Err(err.into());
            }
        }
    }
    Ok(())
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    ensure_global(session, args.global, "hold")?;
    hold_packages(session, &args.package, true, rust_i18n::t!("cmd.holding"))
}
