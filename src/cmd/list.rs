use clap::{ArgAction, Parser};
use libscoop::{operation, QueryOption, Session};

use crate::{output, Result};

/// List installed package(s)
#[derive(Debug, Parser)]
pub struct Args {
    /// The query string (regex supported by default)
    #[arg(action = ArgAction::Append)]
    query: Vec<String>,
    /// Turn regex off and use explicit matching
    #[arg(short = 'e', long, action = ArgAction::SetTrue)]
    explicit: bool,
    /// List upgradable package(s)
    #[arg(short = 'u', long, action = ArgAction::SetTrue)]
    upgradable: bool,
    /// List held package(s)
    #[arg(short = 'H', long, action = ArgAction::SetTrue)]
    held: bool,
    /// Show all installed versions (not just current)
    #[arg(short = 'V', long, action = ArgAction::SetTrue)]
    versions: bool,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let queries = args.query.iter().map(|s| s.as_str()).collect::<Vec<_>>();
    let mut options = vec![];

    if args.explicit {
        options.push(QueryOption::Explicit);
    }

    if args.upgradable {
        options.push(QueryOption::Upgradable);
    }

    if args.versions {
        return list_with_versions(&queries, &options, session);
    }

    match operation::package_query(session, queries, options, true) {
        Err(e) => Err(e.into()),
        Ok(packages) => {
            for pkg in packages {
                let mut output = String::new();
                output.push_str(
                    format!("{}/{} {}", pkg.name(), pkg.bucket(), pkg.version()).as_str(),
                );

                let held = pkg.is_held();
                if args.held && !held {
                    continue;
                }

                let upgradable = pkg.upgradable_version();
                if args.upgradable && upgradable.is_some() {
                    output.push_str(format!(" -> {}", upgradable.unwrap()).as_str());
                }

                if held {
                    output.push_str(" [held]");
                }

                output::status(&output);
            }
            Ok(())
        }
    }
}

/// List packages with all installed versions shown.
fn list_with_versions(queries: &[&str], options: &[QueryOption], session: &Session) -> Result<()> {
    let root_path = session.config().root_path().to_owned();
    let apps_dir = root_path.join("apps");

    // Get current packages for name/bucket info
    let pkgs = operation::package_query(session, queries.to_vec(), options.to_vec(), true)
        .unwrap_or_default();

    for pkg in &pkgs {
        output::named(pkg.name(), format!("/{}", pkg.bucket()));

        let app_dir = apps_dir.join(pkg.name());
        if !app_dir.exists() {
            continue;
        }

        // Read all version directories under apps/{name}/
        let current_target = std::fs::read_link(app_dir.join("current")).ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()));

        let mut versions: Vec<_> = std::fs::read_dir(&app_dir)
            .map(|entries| entries.flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|name| name != "current")
                .collect::<Vec<_>>())
            .unwrap_or_default();

        versions.sort_by(|a, b| {
            let a_ver = a.trim_start_matches(|c| c == 'v' || c == 'V');
            let b_ver = b.trim_start_matches(|c| c == 'v' || c == 'V');
            // Simple numeric sort — descending (newest first)
            let a_parts: Vec<u64> = a_ver.split('.').filter_map(|s| s.parse().ok()).collect();
            let b_parts: Vec<u64> = b_ver.split('.').filter_map(|s| s.parse().ok()).collect();
            for (a_n, b_n) in a_parts.iter().zip(b_parts.iter()) {
                match a_n.cmp(b_n) {
                    std::cmp::Ordering::Equal => continue,
                    other => return other.reverse(),
                }
            }
            a_parts.len().cmp(&b_parts.len()).reverse()
        });

        for ver in &versions {
            let is_current = current_target.as_deref() == Some(ver.as_str());
            if is_current {
                output::named(ver.as_str(), "(current)");
            } else {
                output::status(ver);
            }
        }

        if versions.is_empty() {
            output::named("(no versions)", "(broken install)");
        }
    }

    Ok(())
}
