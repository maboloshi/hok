//! Archive extraction & downloaded-file copying primitives.

use std::path::Path;
use tracing::debug;

use crate::{error::Fallible, internal, package::Package, Event, Session};

/// Extract the downloaded archive files of `pkg` into `working_dir`.
///
/// An index into [`Package::download_filenames`][1] is treated as an archive
/// when its URL's extension is a known archive format. Each existing archive
/// is extracted via [`internal::archive::extract`] with the manifest's
/// `extract_dir` / `extract_to` / `innosetup` settings applied.
///
/// # Returns
///
/// The indices of the archive entries, so callers can skip them when copying
/// the remaining downloaded files.
///
/// [1]: crate::package::Package::download_filenames
pub fn extract_archives(
    session: &Session,
    pkg: &Package,
    working_dir: &Path,
) -> Fallible<Vec<usize>> {
    let files = pkg.download_filenames();
    let urls = pkg.manifest().url();

    // Collect the files that need to be decompressed
    let archives: Vec<usize> = files
        .iter()
        .enumerate()
        .filter_map(|(idx, f)| {
            let url = &urls[idx];

            // Extract the target filename directly from the URL
            let target_name = url.rsplit('/').next().unwrap_or(f);

            let ext = Path::new(target_name)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if matches!(
                ext,
                "7z" | "zip"
                    | "nupkg"
                    | "rar"
                    | "lzh"
                    | "gz"
                    | "bz2"
                    | "xz"
                    | "zst"
                    | "tgz"
                    | "tar"
            ) {
                Some(idx)
            } else {
                None
            }
        })
        .collect();

    if !archives.is_empty() {
        let cache_path = session.config().cache_path().to_path_buf();
        debug!(
            "extract: {} v{} - extract ({} files)",
            pkg.name(),
            pkg.version(),
            archives.len()
        );

        for idx in archives.iter() {
            let filename = &files[*idx];
            let src = cache_path.join(filename);
            if src.exists() {
                if let Some(tx) = session.emitter() {
                    let _ = tx.send(Event::PackageExtractStart(format!(
                        "{}/{}",
                        pkg.name(),
                        filename
                    )));
                }
                let emit = session.emitter();
                internal::archive::extract(
                    &src,
                    working_dir,
                    pkg.manifest().extract_dir().as_deref(),
                    pkg.manifest().extract_to().as_deref(),
                    pkg.manifest().innosetup(),
                    &emit,
                )?;
                if let Some(tx) = session.emitter() {
                    let _ = tx.send(Event::PackageExtractDone);
                }
            }
        }
    }

    Ok(archives)
}

/// Copy the downloaded non-archive files of `pkg` into `working_dir`.
///
/// Files whose index is listed in `archives` (already extracted) are skipped.
/// The destination filename is derived from the URL (after the last `/`),
/// matching how Scoop names downloaded files.
pub fn copy_downloaded_files(
    session: &Session,
    pkg: &Package,
    working_dir: &Path,
    archives: &[usize],
) -> Fallible<()> {
    let files = pkg.download_filenames();
    let urls = pkg.manifest().url();

    debug!(
        "copy: {} v{} - copy ({} files)",
        pkg.name(),
        pkg.version(),
        files.len() - archives.len()
    );

    for (idx, filename) in files.iter().enumerate() {
        // Skip already extracted archive files
        if archives.contains(&idx) {
            continue;
        }

        let src = session.config().cache_path().join(filename);
        if !src.exists() {
            continue;
        }

        let url = &urls[idx];
        let target_name = url.rsplit('/').next().unwrap_or(filename);

        let dst = working_dir.join(target_name);
        let _ = std::fs::remove_file(&dst);
        std::fs::copy(&src, dst)?;
    }

    Ok(())
}

/// Process P2 extraction marker files left behind by package scripts.
///
/// Package PowerShell scripts may write `<format>|<source>|<dest>` lines into
/// `hok_extract_markers.txt` inside `working_dir`; each marker is extracted
/// natively by Rust. Extraction failures are logged but do not abort, matching
/// the original behaviour (the PS script has its own error handling).
pub fn extract_markers(session: &Session, working_dir: &Path) {
    let marker_path = working_dir.join("hok_extract_markers.txt");
    if let Ok(markers) = std::fs::read_to_string(&marker_path) {
        for line in markers.lines() {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() < 3 {
                continue;
            }
            let format = parts[0];
            let source = Path::new(parts[1]);
            let dest = Path::new(parts[2]);
            let innosetup = format == "innosetup";

            if source.exists() {
                let emit = session.emitter();
                if let Err(e) =
                    internal::archive::extract(source, dest, None, None, innosetup, &emit)
                {
                    // Log but don't abort — extraction errors may be handled
                    // by the PS script's own error handling
                    debug!("P2 extract failed for {}: {}", source.display(), e);
                }
            }
        }
    }
}
