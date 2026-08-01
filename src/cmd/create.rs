use clap::Parser;
use libscoop::{fs, package::create, Session};
use std::path::PathBuf;

use crate::{output, Result};

/// Create a manifest from a download URL
///
/// Downloads the file, computes its hash, and generates a manifest skeleton.
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// Download URL
    url: String,

    /// Optional output file (default: stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,
}

pub fn execute(args: Args, session: &Session) -> Result<()> {
    let url = args.url.trim();
    if url.is_empty() {
        output::err(rust_i18n::t!("cmd.create_url_required"));
        return Ok(());
    }

    output::info(format!("Creating manifest for: {url}"));

    output::progress(rust_i18n::t!("cmd.downloading"), "");
    let manifest = create::create_manifest(session, url)
        .map_err(|e| anyhow::anyhow!("create manifest failed: {e}"))?;
    output::ok();

    let output_json = serde_json::to_string_pretty(&manifest)?;

    match &args.output {
        Some(path) => {
            fs::write(path, output_json.as_bytes())?;
            output::done(rust_i18n::t!("cmd.create_manifest_saved", path = path.display()));
        }
        None => {
            println!("\n{}", output_json);
        }
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
