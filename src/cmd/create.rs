use clap::Parser;
use crossterm::style::Stylize;
use libscoop::{operation, Session};
use scoop_hash::ChecksumBuilder;
use std::io::{Read, Write};
use std::path::PathBuf;

use crate::Result;

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
        eprintln!("URL is required.");
        return Ok(());
    }

    println!("  {} Creating manifest for: {}", "•".blue(), url);

    // Extract filename from URL
    let filename = url.rsplit('/').next()
        .and_then(|s| s.split('?').next())
        .unwrap_or("download");
    let name = filename.rsplit('.').skip(1).next()
        .or_else(|| filename.split('.').next())
        .unwrap_or("app");

    // Detect archive type
    let is_archive = filename.ends_with(".zip") || filename.ends_with(".7z")
        || filename.ends_with(".tar.gz") || filename.ends_with(".tgz")
        || filename.ends_with(".tar.xz") || filename.ends_with(".tar.bz2")
        || filename.ends_with(".tar") || filename.ends_with(".gz")
        || filename.ends_with(".bz2") || filename.ends_with(".xz")
        || filename.ends_with(".zst") || filename.ends_with(".rar");

    // Download to temp and compute hash
    let tmp_dir = std::env::temp_dir().join("hok-create");
    std::fs::create_dir_all(&tmp_dir)?;
    let dest = tmp_dir.join(filename);

    print!("  {} Downloading ... ", "•".blue());
    std::io::stdout().flush()?;
    operation::download_file(session, url, &dest)
        .map_err(|e| anyhow::anyhow!("download failed: {}", e))?;
    println!("{}", "done".green());

    print!("  {} Computing hash ... ", "•".blue());
    std::io::stdout().flush()?;
    let hash = compute_file_hash(&dest)?;
    println!("{}", "done".green());

    // Generate manifest
    let version = "0.0.0".to_string();
    let homepage = "https://example.com".to_string();
    let description = format!("{} description", name);

    let manifest = serde_json::json!({
        "version": version,
        "description": description,
        "homepage": homepage,
        "license": "Unknown",
        "url": url,
        "hash": hash,
    });

    let manifest = if is_archive {
        manifest
    } else {
        let mut m = manifest.as_object().unwrap().clone();
        let bin_name = name.to_string();
        m.insert("bin".to_string(), serde_json::json!([bin_name]));
        serde_json::Value::Object(m)
    };

    let output = serde_json::to_string_pretty(&manifest)?;

    match &args.output {
        Some(path) => {
            std::fs::write(path, output.as_bytes())?;
            println!("\n  {} Manifest saved to: {}", "✓".green(), path.display());
        }
        None => {
            println!("\n{}", output);
        }
    }

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp_dir);

    Ok(())
}

fn compute_file_hash(path: &std::path::Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| anyhow::anyhow!("open: {}", e))?;
    let builder = ChecksumBuilder::new();
    let mut hasher = builder.build();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)
            .map_err(|e| anyhow::anyhow!("read: {}", e))?;
        if n == 0 { break; }
        hasher.consume(&buf[..n]);
    }
    Ok(hasher.finalize())
}
