use clap::Parser;
use std::path::PathBuf;

use libscoop::package::formatjson;
use libscoop::Session;
use crate::{output, util, Result};

/// Format manifest JSON files in a bucket directory
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = ".")]
    dir: PathBuf,

    /// Specific app(s) to format (supports wildcards, default: all)
    #[arg(default_value = "*")]
    app: Vec<String>,
}

pub fn execute(args: Args) -> Result<()> {
    let dir = &args.dir;
    if !dir.is_dir() {
        output::err(rust_i18n::t!("cmd.checkurls_err_dir", path = dir.display()));
        return Ok(());
    }

    // Determine app filter patterns
    let patterns: Vec<&str> = if args.app.is_empty() || args.app[0] == "*" {
        Vec::new()
    } else {
        args.app.iter().map(|s| s.as_str()).collect()
    };

    let entries = util::walkdir_files(dir);
    let mut count = 0u32;

    for path in &entries {
        // Apply app filter on the file stem using libscoop's glob matching
        if !patterns.is_empty() {
            let name = path.file_stem().unwrap().to_string_lossy();
            if !patterns.iter().any(|p| formatjson::app_matches(&name, p)) {
                continue;
            }
        }

        // Delegate formatting to libscoop
        match formatjson::format_manifest_file(path) {
            Ok(true) => {
                output::done(format!("{}", path.display()));
                count += 1;
            }
            Ok(false) => {}
            Err(e) => output::err(format!("{e}")),
        }
    }

    if count == 0 {
        output::info(rust_i18n::t!("cmd.formatjson_none"));
    } else {
        output::info(rust_i18n::t!("cmd.formatjson_count", count = count));
    }

    Ok(())
}
use crate::cmd::shared_args::Cmd;

impl Cmd for Args {
    type Args = Self;

    #[inline]
    fn execute(args: Self::Args, _session: &Session) -> Result<()> {
        execute(args)
    }
}
