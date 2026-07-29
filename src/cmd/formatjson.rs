use clap::Parser;
use regex::Regex;
use serde::Serialize;
use std::path::PathBuf;

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

/// Convert a simple glob pattern (supporting `*` and `?`) to a regex.
fn glob_to_regex(pattern: &str) -> String {
    let escaped = regex::escape(pattern);
    let re_str = escaped
        .replace(r"\*", ".*")
        .replace(r"\?", ".");
    format!("^{re_str}$")
}

/// Check whether `name` (the file stem) matches the given app pattern.
fn app_matches(name: &str, pattern: &str) -> bool {
    if pattern.contains('*') || pattern.contains('?') {
        // Has wildcards → convert glob to regex and match
        let re_str = glob_to_regex(pattern);
        Regex::new(&re_str).map_or(false, |re| re.is_match(name))
    } else {
        // No wildcards → exact stem match (same as PS -Filter "app.json")
        name == pattern
    }
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
        // Apply app filter on the file stem
        if !patterns.is_empty() {
            let name = path.file_stem().unwrap().to_string_lossy();
            if !patterns.iter().any(|p| app_matches(&name, p)) {
                continue;
            }
        }

        // Read, validate, and reformat
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let cleaned = content.trim_start_matches('\u{FEFF}');

        let value: serde_json::Value = match json5::from_str(cleaned) {
            Ok(v) => v,
            Err(e) => {
                output::err(format!("{}: parse error: {}", path.display(), e));
                continue;
            }
        };

        // Serialize with 4-space indent (Scoop convention), CRLF (Windows).
        let mut buf = Vec::new();
        let fmt = serde_json::ser::PrettyFormatter::with_indent(b"    ");
        let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
        value
            .serialize(&mut ser)
            .map_err(|e| anyhow::anyhow!("serialize error: {}", e))?;
        let mut formatted = String::from_utf8(buf)
            .map_err(|e| anyhow::anyhow!("utf8 error: {}", e))?;
        formatted = formatted.replace('\n', "\r\n");
        if !formatted.ends_with("\r\n") {
            formatted.push_str("\r\n");
        }

        // Only write if the content changed
        if formatted != content {
            std::fs::write(path, formatted.as_bytes())?;
            output::done(format!("{}", path.display()));
            count += 1;
        }
    }

    if count == 0 {
        output::info(rust_i18n::t!("cmd.formatjson_none"));
    } else {
        output::info(rust_i18n::t!("cmd.formatjson_count", count = count));
    }

    Ok(())
}
