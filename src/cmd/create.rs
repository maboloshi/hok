//! Create a package manifest.

use clap::Parser;
use libscoop::{Session, fs, package::create};
use std::path::PathBuf;

use crate::{Result, output};

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

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let url = args.url.trim();
    if url.is_empty() {
        return Err(anyhow::anyhow!(rust_i18n::t!("cmd.create_url_required")));
    }

    output::info(rust_i18n::t!("cmd.create_creating", url = url));

    output::progress(rust_i18n::t!("cmd.downloading"), "");
    let manifest = create::create_manifest(session, url)
        .await
        .map_err(|e| anyhow::anyhow!(rust_i18n::t!(
            "cmd.create_failed",
            e = e
        )))?;
    output::ok();

    let output_json = serde_json::to_string_pretty(&manifest)?;

    match &args.output {
        Some(path) => {
            fs::write(path, output_json.as_bytes())?;
            output::done(rust_i18n::t!(
                "cmd.create_manifest_saved",
                path = path.display()
            ));
        }
        None => {
            println!("\n{}", output_json);
        }
    }

    Ok(())
}
