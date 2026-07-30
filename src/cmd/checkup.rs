use clap::Parser;
use libscoop::Session;

use crate::{output, Result};

/// Check for potential problems with installed packages
#[derive(Debug, Parser)]
pub struct Args {}

pub fn execute(_: Args, session: &Session) -> Result<()> {
    let config = session.config();
    let apps_dir = config.root_path().join("apps");
    let mut issues = 0u32;

    if !apps_dir.exists() {
        output::warn(rust_i18n::t!("cmd.no_apps_found"));
        return Ok(());
    }

    for entry in std::fs::read_dir(&apps_dir)?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "scoop" { continue; }

        let app_dir = entry.path();
        let current = app_dir.join("current");

        // Check that 'current' symlink exists and points somewhere
        if !current.exists() {
            output::named(name.as_ref(), rust_i18n::t!("cmd.no_current_symlink"));
            issues += 1;
            continue;
        }

        // Verify install.json and manifest.json exist
        let install_json = current.join("install.json");
        let manifest_json = current.join("manifest.json");

        if !install_json.exists() {
            output::named(name.as_ref(), rust_i18n::t!("cmd.missing_install_json"));
            issues += 1;
        }
        if !manifest_json.exists() {
            output::named(name.as_ref(), rust_i18n::t!("cmd.missing_manifest_json"));
            issues += 1;
        }
    }

    if issues == 0 {
        output::info(rust_i18n::t!("cmd.no_issues"));
    } else {
        output::warn(format!("{issues} issue(s) found."));
        output::status(rust_i18n::t!("cmd.run_reset"));
    }

    Ok(())
}
use crate::cmd::shared_args::Cmd;

impl Cmd for Args {
    type Args = Self;

    #[inline]
    fn execute(args: Self::Args, session: &Session) -> Result<()> {
        execute(args, session)
    }
}
