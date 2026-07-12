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
        output::warn("No apps directory found.");
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
            output::named(name.as_ref(), "no 'current' symlink");
            issues += 1;
            continue;
        }

        // Verify install.json and manifest.json exist
        let install_json = current.join("install.json");
        let manifest_json = current.join("manifest.json");

        if !install_json.exists() {
            output::named(name.as_ref(), "missing install.json");
            issues += 1;
        }
        if !manifest_json.exists() {
            output::named(name.as_ref(), "missing manifest.json");
            issues += 1;
        }
    }

    if issues == 0 {
        output::info("No issues found.");
    } else {
        output::warn(format!("{issues} issue(s) found."));
        output::status("run 'hok reset <app>' to fix");
    }

    Ok(())
}
