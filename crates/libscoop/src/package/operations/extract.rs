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

            // Extract the target filename directly from the URL (query
            // parameters stripped; the `#/rename.ext` Scoop fragment kept).
            let stripped = internal::url::strip_url_query(url);
            let target_name = stripped.rsplit('/').next().unwrap_or(f);

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
                    | "iso"
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
        let stripped = internal::url::strip_url_query(url);
        let target_name = stripped.rsplit('/').next().unwrap_or(filename);

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

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use crate::package::{Manifest, Package};
    use crate::test_utils::tmpdir;
    use crate::Session;

    use super::*;

    // ─── Helpers ────────────────────────────────────────────────────

    /// Like `test_utils::test_session`, but pins `cache_path` to
    /// `<root>/cache` — the global default cache dir would otherwise be
    /// shared by concurrently running tests.
    fn session_with_cache(root: &Path) -> Session {
        let config_path = root.join("config.json");
        let root_escaped = root.to_string_lossy().replace('\\', "\\\\");
        let cache_escaped = root.join("cache").to_string_lossy().replace('\\', "\\\\");
        std::fs::write(
            &config_path,
            format!(
                r#"{{"root_path": "{}", "cache_path": "{}"}}"#,
                root_escaped, cache_escaped
            ),
        )
        .unwrap();
        Session::new_with(&config_path).unwrap()
    }

    /// Build a manifest whose `url` field is the given JSON fragment.
    fn manifest_with_urls(urls_json: &str, extra: &str) -> Manifest {
        let json = format!(
            r#"{{
                "version": "1.0.0",
                "homepage": "https://example.com",
                "license": "MIT",
                "url": {urls_json},
                "hash": "0000000000000000000000000000000000000000000000000000000000000000"
                {extra}
            }}"#
        );
        Manifest::from_json("test-pkg", &json).unwrap()
    }

    /// Build a manifest with a single download URL.
    fn manifest(url: &str) -> Manifest {
        manifest_with_urls(&format!(r#""{url}""#), "")
    }

    /// Create a small real zip archive containing `app/hello.txt` and
    /// `app/sub/deep.txt` (mirrors `internal::archive::tests`).
    fn create_zip(path: &Path) {
        use zip::write::FileOptions;
        use zip::CompressionMethod;
        use zip::ZipWriter;

        let file = std::fs::File::create(path).unwrap();
        let mut zip = ZipWriter::new(file);
        let opts: FileOptions<'_, ()> =
            FileOptions::default().compression_method(CompressionMethod::Stored);

        zip.start_file("app/hello.txt", opts).unwrap();
        zip.write_all(b"Hello, World!").unwrap();
        zip.start_file("app/sub/deep.txt", opts).unwrap();
        zip.write_all(b"Deep content").unwrap();
        zip.finish().unwrap();
    }

    /// Place a fake downloaded file into the session cache dir under the
    /// name `download_filenames()[idx]` produces, so `extract_archives` /
    /// `copy_downloaded_files` find it via `session.config().cache_path()`.
    fn stage_cache_file(session: &Session, pkg: &Package, idx: usize, content: &[u8]) -> PathBuf {
        let filename = &pkg.download_filenames()[idx];
        let config = session.config();
        let cache_dir = config.cache_path();
        std::fs::create_dir_all(cache_dir).unwrap();
        let path = cache_dir.join(filename);
        std::fs::write(&path, content).unwrap();
        path
    }

    // ─── extract_archives ───────────────────────────────────────────

    #[test]
    fn test_extract_archives_zip_only() {
        let root = tmpdir("extract_archives_zip");
        let session = session_with_cache(&root);
        let pkg = Package::from("test-pkg", "main", manifest("https://example.com/pkg.zip"));

        let staged = stage_cache_file(&session, &pkg, 0, &[]);
        create_zip(&staged);

        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        let archives = extract_archives(&session, &pkg, &working_dir).unwrap();
        assert_eq!(archives, vec![0]);
        assert!(working_dir.join("app/hello.txt").exists());
        assert_eq!(
            std::fs::read_to_string(working_dir.join("app/hello.txt")).unwrap(),
            "Hello, World!"
        );
        assert!(working_dir.join("app/sub/deep.txt").exists());
    }

    #[test]
    fn test_extract_archives_mixed_zip_and_plain() {
        let root = tmpdir("extract_archives_mixed");
        let session = session_with_cache(&root);
        let pkg = Package::from(
            "test-pkg",
            "main",
            manifest_with_urls(
                r#"["https://example.com/pkg.zip", "https://example.com/setup.exe"]"#,
                "",
            ),
        );

        let staged_zip = stage_cache_file(&session, &pkg, 0, &[]);
        create_zip(&staged_zip);
        stage_cache_file(&session, &pkg, 1, b"exe content");

        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        // Only the .zip URL is treated as an archive.
        let archives = extract_archives(&session, &pkg, &working_dir).unwrap();
        assert_eq!(archives, vec![0]);
        assert!(working_dir.join("app/hello.txt").exists());

        // The plain file is copied, not extracted.
        copy_downloaded_files(&session, &pkg, &working_dir, &archives).unwrap();
        assert_eq!(
            std::fs::read_to_string(working_dir.join("setup.exe")).unwrap(),
            "exe content"
        );
    }

    #[test]
    fn test_extract_archives_iso_detected_as_archive() {
        // `.iso` must be recognised as an archive (aligned with
        // `internal::archive::detect_format`, which extracts via 7z). The
        // actual extraction is not exercised here: it requires the external
        // `7z` executable, so the cache file is deliberately not staged —
        // `extract_archives` skips missing files but still reports the index.
        let root = tmpdir("extract_archives_iso");
        let session = session_with_cache(&root);
        let pkg = Package::from("test-pkg", "main", manifest("https://example.com/disk.iso"));

        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        let archives = extract_archives(&session, &pkg, &working_dir).unwrap();
        assert_eq!(archives, vec![0]);
    }

    #[test]
    fn test_extract_archives_unknown_extension_copied() {
        let root = tmpdir("extract_archives_unknown");
        let session = session_with_cache(&root);
        let pkg = Package::from("test-pkg", "main", manifest("https://example.com/file.bin"));

        stage_cache_file(&session, &pkg, 0, b"binary");

        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        let archives = extract_archives(&session, &pkg, &working_dir).unwrap();
        assert!(archives.is_empty());

        copy_downloaded_files(&session, &pkg, &working_dir, &archives).unwrap();
        assert_eq!(
            std::fs::read_to_string(working_dir.join("file.bin")).unwrap(),
            "binary"
        );
    }

    #[test]
    fn test_extract_archives_url_with_query_detected() {
        // Query parameters must be stripped before extension detection, so
        // `pkg.zip?download=1` is still recognised as a zip archive. The
        // cache filename derived from the URL must also stay `?`-free (the
        // old behaviour produced one with `?` in it, illegal on Windows).
        let root = tmpdir("extract_archives_query");
        let session = session_with_cache(&root);
        let pkg = Package::from(
            "test-pkg",
            "main",
            manifest("https://example.com/pkg.zip?download=1"),
        );

        let staged = stage_cache_file(&session, &pkg, 0, &[]);
        create_zip(&staged);
        assert!(!staged.to_string_lossy().contains('?'));

        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        let archives = extract_archives(&session, &pkg, &working_dir).unwrap();
        assert_eq!(archives, vec![0]);
        assert!(working_dir.join("app/hello.txt").exists());
    }

    #[test]
    fn test_extract_archives_missing_cache_file_skipped() {
        // The archive index is still reported, but a missing cache file is
        // silently skipped (matching Scoop behaviour for partial downloads).
        let root = tmpdir("extract_archives_missing");
        let session = session_with_cache(&root);
        let pkg = Package::from("test-pkg", "main", manifest("https://example.com/pkg.zip"));

        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        let archives = extract_archives(&session, &pkg, &working_dir).unwrap();
        assert_eq!(archives, vec![0]);
        assert!(!working_dir.join("app/hello.txt").exists());
    }

    #[test]
    fn test_extract_archives_with_extract_dir() {
        let root = tmpdir("extract_archives_extract_dir");
        let session = session_with_cache(&root);
        let pkg = Package::from(
            "test-pkg",
            "main",
            manifest_with_urls(
                r#""https://example.com/pkg.zip""#,
                r#","extract_dir": "app""#,
            ),
        );

        let staged = stage_cache_file(&session, &pkg, 0, &[]);
        create_zip(&staged);

        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        extract_archives(&session, &pkg, &working_dir).unwrap();

        // Prefix `app/` stripped: files land directly in working_dir.
        assert!(working_dir.join("hello.txt").exists());
        assert!(!working_dir.join("app/hello.txt").exists());
        assert!(working_dir.join("sub/deep.txt").exists());
    }

    #[test]
    fn test_extract_archives_with_extract_to() {
        let root = tmpdir("extract_archives_extract_to");
        let session = session_with_cache(&root);
        let pkg = Package::from(
            "test-pkg",
            "main",
            manifest_with_urls(
                r#""https://example.com/pkg.zip""#,
                r#","extract_to": "myapp""#,
            ),
        );

        let staged = stage_cache_file(&session, &pkg, 0, &[]);
        create_zip(&staged);

        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        extract_archives(&session, &pkg, &working_dir).unwrap();

        // Everything lands under working_dir/myapp/.
        assert!(working_dir.join("myapp/app/hello.txt").exists());
    }

    // ─── copy_downloaded_files ──────────────────────────────────────

    #[test]
    fn test_copy_downloaded_files_skips_archives() {
        let root = tmpdir("copy_skips_archives");
        let session = session_with_cache(&root);
        let pkg = Package::from(
            "test-pkg",
            "main",
            manifest_with_urls(
                r#"["https://example.com/a.zip", "https://example.com/b.exe"]"#,
                "",
            ),
        );

        stage_cache_file(&session, &pkg, 0, &[]);
        stage_cache_file(&session, &pkg, 1, b"b content");

        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        // Index 0 is an archive: it must NOT be copied as a file.
        copy_downloaded_files(&session, &pkg, &working_dir, &[0]).unwrap();
        assert!(!working_dir.join("a.zip").exists());
        assert_eq!(
            std::fs::read_to_string(working_dir.join("b.exe")).unwrap(),
            "b content"
        );
    }

    #[test]
    fn test_copy_downloaded_files_overwrites_existing() {
        let root = tmpdir("copy_overwrites");
        let session = session_with_cache(&root);
        let pkg = Package::from("test-pkg", "main", manifest("https://example.com/b.exe"));

        stage_cache_file(&session, &pkg, 0, b"new content");

        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();
        std::fs::write(working_dir.join("b.exe"), b"stale content").unwrap();

        copy_downloaded_files(&session, &pkg, &working_dir, &[]).unwrap();
        assert_eq!(
            std::fs::read_to_string(working_dir.join("b.exe")).unwrap(),
            "new content"
        );
    }

    #[test]
    fn test_copy_downloaded_files_missing_source_skipped() {
        let root = tmpdir("copy_missing_source");
        let session = session_with_cache(&root);
        let pkg = Package::from(
            "test-pkg",
            "main",
            manifest("https://example.com/missing.exe"),
        );

        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        // No file staged in the cache: silently skipped, no error.
        copy_downloaded_files(&session, &pkg, &working_dir, &[]).unwrap();
        assert!(!working_dir.join("missing.exe").exists());
    }

    #[test]
    fn test_copy_downloaded_files_strips_query_from_target() {
        let root = tmpdir("copy_strips_query");
        let session = session_with_cache(&root);
        let pkg = Package::from(
            "test-pkg",
            "main",
            manifest("https://example.com/file.exe?download=1"),
        );

        stage_cache_file(&session, &pkg, 0, b"exe content");

        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();

        copy_downloaded_files(&session, &pkg, &working_dir, &[]).unwrap();
        // The query string must not leak into the destination filename.
        assert!(working_dir.join("file.exe").exists());
        assert!(!working_dir.join("file.exe?download=1").exists());
        assert_eq!(
            std::fs::read_to_string(working_dir.join("file.exe")).unwrap(),
            "exe content"
        );
    }

    // ─── extract_markers ────────────────────────────────────────────

    fn write_markers(working_dir: &Path, lines: &[&str]) {
        let marker_path = working_dir.join("hok_extract_markers.txt");
        std::fs::write(&marker_path, lines.join("\n")).unwrap();
    }

    #[test]
    fn test_extract_markers_zip() {
        let root = tmpdir("markers_zip");
        let session = session_with_cache(&root);
        let archive = root.join("pkg.zip");
        create_zip(&archive);
        let dest = root.join("dest");

        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();
        write_markers(
            &working_dir,
            &[&format!("zip|{}|{}", archive.display(), dest.display())],
        );

        extract_markers(&session, &working_dir);
        assert!(dest.join("app/hello.txt").exists());
    }

    #[test]
    fn test_extract_markers_malformed_lines_skipped() {
        let root = tmpdir("markers_malformed");
        let session = session_with_cache(&root);
        let archive = root.join("pkg.zip");
        create_zip(&archive);
        let dest = root.join("dest");

        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();
        // A line with fewer than 3 parts must be skipped without panicking.
        write_markers(
            &working_dir,
            &[
                "zip|only-two-parts",
                &format!("zip|{}|{}", archive.display(), dest.display()),
            ],
        );

        extract_markers(&session, &working_dir);
        assert!(dest.join("app/hello.txt").exists());
    }

    #[test]
    fn test_extract_markers_missing_source_skipped() {
        let root = tmpdir("markers_missing_source");
        let session = session_with_cache(&root);
        let missing = root.join("does-not-exist.zip");
        let dest = root.join("dest");

        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();
        write_markers(
            &working_dir,
            &[&format!("zip|{}|{}", missing.display(), dest.display())],
        );

        // No panic, nothing extracted, extraction does not abort.
        extract_markers(&session, &working_dir);
        assert!(!dest.exists());
    }

    #[test]
    fn test_extract_markers_bad_archive_continues() {
        let root = tmpdir("markers_bad_archive");
        let session = session_with_cache(&root);
        // A file that exists but is not a valid archive.
        let bad = root.join("bad.zip");
        std::fs::write(&bad, b"this is not a zip").unwrap();
        let good = root.join("good.zip");
        create_zip(&good);
        let dest = root.join("dest");

        let working_dir = root.join("work");
        std::fs::create_dir_all(&working_dir).unwrap();
        write_markers(
            &working_dir,
            &[
                &format!("zip|{}|{}", bad.display(), root.join("bad-dest").display()),
                &format!("zip|{}|{}", good.display(), dest.display()),
            ],
        );

        // A failing marker must not abort processing of the remaining lines.
        extract_markers(&session, &working_dir);
        assert!(dest.join("app/hello.txt").exists());
    }
}
