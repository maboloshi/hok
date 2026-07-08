use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::constant::REGEX_ARCHIVE_7Z;
use crate::error::Fallible;
use crate::Error;

/// Detect archive format from filename extension.
fn detect_format(filename: &str) -> Option<&'static str> {
    if filename.ends_with(".7z") {
        return Some("7z");
    }
    if filename.ends_with(".zip") || filename.ends_with(".nupkg") {
        return Some("zip");
    }
    if filename.ends_with(".tar") {
        return Some("tar");
    }
    if filename.ends_with(".tgz") || filename.ends_with(".tar.gz") || filename.ends_with(".gz") {
        return Some("gz");
    }
    if filename.ends_with(".tar.bz2") || filename.ends_with(".bz2") || filename.ends_with(".bz") {
        return Some("bz2");
    }
    if filename.ends_with(".tar.xz") || filename.ends_with(".xz") || filename.ends_with(".lzma") {
        return Some("xz");
    }
    if filename.ends_with(".rar") {
        return Some("rar");
    }
    if filename.ends_with(".lzh") {
        return Some("lzh");
    }
    if filename.ends_with(".zst") {
        return Some("zst");
    }
    if filename.ends_with(".iso") {
        return Some("iso");
    }
    if REGEX_ARCHIVE_7Z.is_match(filename) {
        // unmatched ... but still a recognized archive
        return Some("unknown");
    }
    None
}

/// Check whether the format requires falling back to the external 7z.exe.
fn needs_fallback(fmt: &str) -> bool {
    matches!(fmt, "iso" | "unknown")
}

/// Extract an archive file to the destination directory.
///
/// * `cache_path` — Path to the downloaded archive file.
/// * `dest_dir` — Directory to extract into.
/// * `extract_dir` — If set, only extract files under this subdirectory and
///   strip the prefix. e.g. `extract_dir = ["dir1"]` means files from
///   `dir1/sub/a.txt` → `dest_dir/sub/a.txt`.
/// * `extract_to` — If set, extract all files into this subdirectory of
///   `dest_dir`. e.g. `extract_to = ["sub"]` means all files go to
///   `dest_dir/sub/...`.
/// * `innosetup` — Whether the package is an Inno Setup installer.
pub fn extract(
    cache_path: &Path,
    dest_dir: &Path,
    extract_dir: Option<&[&str]>,
    extract_to: Option<&[&str]>,
    innosetup: bool,
) -> Fallible<()> {
    // Compute effective extract_to path
    let effective_dest = match extract_to {
        Some(subdirs) if !subdirs.is_empty() => dest_dir.join(&subdirs[0]),
        _ => dest_dir.to_path_buf(),
    };
    crate::internal::fs::ensure_dir(&effective_dest)?;

    if innosetup {
        return extract_innosetup(cache_path, &effective_dest, extract_dir);
    }

    let filename = cache_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let fmt = detect_format(&filename)
        .ok_or_else(|| Error::ExtractionFailed(format!("unknown archive format: {}", filename)))?;

    // Fallback for unsupported formats (ISO, etc.)
    if needs_fallback(fmt) {
        return extract_with_7z_exe(cache_path, &effective_dest);
    }

    match fmt {
        "7z" => extract_7z(cache_path, &effective_dest, extract_dir),
        "zip" => extract_zip(cache_path, &effective_dest, extract_dir),
        "tar" => extract_tar(cache_path, &effective_dest, extract_dir, None),
        "gz" => extract_tar(cache_path, &effective_dest, extract_dir, Some(Compression::Gzip)),
        "bz2" => extract_tar(cache_path, &effective_dest, extract_dir, Some(Compression::Bzip2)),
        "xz" => extract_tar(cache_path, &effective_dest, extract_dir, Some(Compression::Xz)),
        "zst" => extract_tar(cache_path, &effective_dest, extract_dir, Some(Compression::Zstd)),
        "rar" | "lzh" => extract_with_unarc(cache_path, &effective_dest, extract_dir, fmt),
        "iso" | "unknown" => unreachable!(), // handled by needs_fallback
        _ => Err(Error::ExtractionFailed(format!("unsupported format: {}", fmt))),
    }
}

// ─── Compression enum for tar filters ────────────────────────────────

enum Compression {
    Gzip,
    Bzip2,
    Xz,
    Zstd,
}

// ─── 7z extraction via sevenz-rust2 ──────────────────────────────────

fn extract_7z(src: &Path, dest: &Path, filter: Option<&[&str]>) -> Fallible<()> {
    use sevenz_rust2::{ArchiveReader, Password};

    let mut reader = ArchiveReader::open(src, Password::empty())
        .map_err(|e| Error::ExtractionFailed(format!("cannot open 7z archive: {}", e)))?;

    let entries: Vec<String> = reader
        .archive()
        .files
        .iter()
        .filter(|e| !e.is_directory())
        .filter(|e| {
            filter
                .map(|f| f.iter().any(|d| e.name().starts_with(d)))
                .unwrap_or(true)
        })
        .map(|e| e.name().to_string())
        .collect();

    for name in &entries {
        let data = reader
            .read_file(name)
            .map_err(|e| Error::ExtractionFailed(format!("failed to read '{}': {}", name, e)))?;
        let target = strip_dir(name, filter).unwrap_or_else(|| name.to_string());
        let target_path = dest.join(&target);
        if let Some(parent) = target_path.parent() {
            crate::internal::fs::ensure_dir(parent)?;
        }
        std::fs::write(&target_path, &data)?;
    }
    Ok(())
}

// ─── Zip extraction via zip crate ────────────────────────────────────

fn extract_zip(src: &Path, dest: &Path, filter: Option<&[&str]>) -> Fallible<()> {
    use std::fs::File;
    use zip::ZipArchive;

    let file = File::open(src)?;
    let mut archive =
        ZipArchive::new(file).map_err(|e| Error::ExtractionFailed(format!("zip error: {}", e)))?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| {
            Error::ExtractionFailed(format!("zip read error: {}", e))
        })?;
        let name = entry.name().to_string();
        if name.ends_with('/') {
            continue;
        }
        if let Some(f) = filter {
            if !f.iter().any(|d| name.starts_with(d)) {
                continue;
            }
        }
        let target = strip_dir(&name, filter).unwrap_or(name);
        let target_path = dest.join(&target);
        if let Some(parent) = target_path.parent() {
            crate::internal::fs::ensure_dir(parent)?;
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        std::fs::write(&target_path, &data)?;
    }
    Ok(())
}

// ─── Tar extraction via tar crate ────────────────────────────────────

fn extract_tar(
    src: &Path,
    dest: &Path,
    filter: Option<&[&str]>,
    compression: Option<Compression>,
) -> Fallible<()> {
    use tar::Archive as TarArchive;

    let file = std::fs::File::open(src)?;
    let reader: Box<dyn Read + Send> = match compression {
        Some(Compression::Gzip) => Box::new(flate2::read::GzDecoder::new(file)),
        Some(Compression::Bzip2) => Box::new(bzip2::read::BzDecoder::new(file)),
        Some(Compression::Xz) => {
            let mut data = Vec::new();
            lzma_rs::xz_decompress(&mut std::io::BufReader::new(file), &mut data)
                .map_err(|e| Error::ExtractionFailed(format!("xz decompress error: {}", e)))?;
            Box::new(std::io::Cursor::new(data))
        }
        Some(Compression::Zstd) => Box::new(zstd::Decoder::new(file)?),
        None => Box::new(file),
    };

    if filter.is_some() {
        let mut archive = TarArchive::new(reader);
        let mut entries = archive.entries()?;
        while let Some(entry) = entries.next() {
            let mut entry: tar::Entry<'_, Box<dyn Read + Send>> = entry?;
            let path = entry.path()?.to_string_lossy().to_string();
            if let Some(f) = filter {
                if !f.iter().any(|d| path.starts_with(d)) {
                    continue;
                }
            }
            let target = strip_dir(&path, filter).unwrap_or(path);
            let target_path = dest.join(&target);
            if let Some(parent) = target_path.parent() {
                crate::internal::fs::ensure_dir(parent)?;
            }
            entry.unpack(dest)?;
        }
    } else {
        let mut archive = TarArchive::new(reader);
        archive.unpack(dest)?;
    }
    Ok(())
}

// ─── RAR / LZH extraction via unarc-rs unified API ───────────────────

fn extract_with_unarc(
    src: &Path,
    dest: &Path,
    filter: Option<&[&str]>,
    _fmt: &str,
) -> Fallible<()> {
    use unarc_rs::unified::ArchiveFormat as UnarcFormat;

    let mut archive = UnarcFormat::open_path(src)
        .map_err(|e| Error::ExtractionFailed(format!("cannot open archive: {}", e)))?;

    while let Some(entry) = archive
        .next_entry()
        .map_err(|e| Error::ExtractionFailed(format!("archive entry error: {}", e)))?
    {
        let name = entry.name().to_string();
        if let Some(f) = filter {
            if !f.iter().any(|d| name.starts_with(d)) {
                archive
                    .skip(&entry)
                    .map_err(|e| Error::ExtractionFailed(format!("skip error: {}", e)))?;
                continue;
            }
        }
        let target = strip_dir(&name, filter).unwrap_or(name);
        let target_path = dest.join(&target);
        if let Some(parent) = target_path.parent() {
            crate::internal::fs::ensure_dir(parent)?;
        }

        let mut out = std::fs::File::create(&target_path)?;
        archive
            .read_to(&entry, &mut out)
            .map_err(|e| Error::ExtractionFailed(format!("extract error: {}", e)))?;
    }
    Ok(())
}

// ─── Fallback: call external 7z.exe for ISO ─────────────────────────

fn extract_with_7z_exe(src: &Path, dest: &Path) -> Fallible<()> {
    let status = Command::new("7z.exe")
        .arg("x")
        .arg(src)
        .arg(format!("-o{}", dest.display()))
        .arg("-y")
        .arg("-bb0")
        .status()
        .map_err(|e| {
            Error::ExtractionFailed(format!(
                "failed to launch 7z.exe (is 7-Zip installed?): {}",
                e
            ))
        })?;

    if !status.success() {
        return Err(Error::ExtractionFailed(format!(
            "7z.exe exited with code {:?}",
            status.code()
        )));
    }
    Ok(())
}

// ─── Inno Setup extraction via innospect ─────────────────────────────

#[cfg(windows)]
fn extract_innosetup(src: &Path, dest: &Path, filter: Option<&[&str]>) -> Fallible<()> {
    let data = std::fs::read(src)
        .map_err(|e| Error::ExtractionFailed(format!("cannot read {}: {}", src.display(), e)))?;

    let installer = innospect::InnoInstaller::from_bytes(&data)
        .map_err(|e| Error::ExtractionFailed(format!("innospect parse error: {}", e)))?;

    for result in installer.extract_files() {
        let (file_entry, bytes) = result
            .map_err(|e| Error::ExtractionFailed(format!("innospect extract error: {}", e)))?;

        // destination is the Inno Setup install path, e.g. `{app}\bin\file.exe`.
        // We strip `{app}\` prefix since the extraction root IS the app dir.
        let name = file_entry.destination.as_str();
        if name.is_empty() {
            continue;
        }
        // Strip Inno Setup constants from the destination path
        let name = strip_innopath(name);
        if name.is_empty() {
            continue;
        }
        if let Some(f) = filter {
            if !f.iter().any(|d| name.starts_with(d)) {
                continue;
            }
        }
        let target = strip_dir(name, filter).unwrap_or_else(|| name.to_string());
        let target_path = dest.join(&target);
        if let Some(parent) = target_path.parent() {
            crate::internal::fs::ensure_dir(parent)?;
        }
        std::fs::write(&target_path, &bytes).map_err(|e| {
            Error::ExtractionFailed(format!("cannot write {}: {}", target_path.display(), e))
        })?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn extract_innosetup(_src: &Path, _dest: &Path, _filter: Option<&[&str]>) -> Fallible<()> {
    Err(Error::ExtractionFailed(
        "Inno Setup extraction is only supported on Windows".into(),
    ))
}

// ─── Helpers ─────────────────────────────────────────────────────────

/// Strip common Inno Setup path constants from a destination path.
///
/// `{app}` → stripped (the app directory IS our extraction root)
/// `{group}` → stripped (start menu group, no meaning in Scoop)
/// `{sys}` → kept as-is (system directory, unusual but possible)
fn strip_innopath(path: &str) -> &str {
    if let Some(rest) = path
        .strip_prefix("{app}")
        .or_else(|| path.strip_prefix("{group}"))
    {
        rest.trim_start_matches('\\').trim_start_matches('/')
    } else if let Some(rest) = path.strip_prefix("{autopf}") {
        rest.trim_start_matches('\\').trim_start_matches('/')
    } else if let Some(rest) = path.strip_prefix("{commonpf}") {
        rest.trim_start_matches('\\').trim_start_matches('/')
    } else {
        path
    }
}

/// Strip the extract_dir prefix from a path inside the archive.
fn strip_dir(path: &str, filter: Option<&[&str]>) -> Option<String> {
    let filter = filter?;
    for prefix in filter {
        let prefix = if prefix.ends_with('/') {
            prefix.to_string()
        } else {
            format!("{}/", prefix)
        };
        if let Some(rest) = path.strip_prefix(&prefix) {
            return Some(rest.to_string());
        }
        // also try without trailing slash
        let trimmed = prefix.trim_end_matches('/');
        if let Some(rest) = path.strip_prefix(trimmed).and_then(|r| {
            if r.is_empty() || r.starts_with('/') || r.starts_with('\\') {
                Some(r.trim_start_matches('/').trim_start_matches('\\'))
            } else {
                None
            }
        }) {
            return Some(rest.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_detect_format() {
        assert_eq!(detect_format("foo.7z"), Some("7z"));
        assert_eq!(detect_format("foo.zip"), Some("zip"));
        assert_eq!(detect_format("foo.nupkg"), Some("zip"));
        assert_eq!(detect_format("foo.tar"), Some("tar"));
        assert_eq!(detect_format("foo.tar.gz"), Some("gz"));
        assert_eq!(detect_format("foo.tgz"), Some("gz"));
        assert_eq!(detect_format("foo.tar.bz2"), Some("bz2"));
        assert_eq!(detect_format("foo.tar.xz"), Some("xz"));
        assert_eq!(detect_format("foo.rar"), Some("rar"));
        assert_eq!(detect_format("foo.lzh"), Some("lzh"));
        assert_eq!(detect_format("foo.iso"), Some("iso"));
        assert_eq!(detect_format("foo.zst"), Some("zst"));
        assert_eq!(detect_format("foo.exe"), None);
        assert_eq!(detect_format(""), None);
    }

    #[test]
    fn test_strip_dir() {
        let filter = vec!["dir1"];
        assert_eq!(
            strip_dir("dir1/sub/a.txt", Some(&filter)),
            Some("sub/a.txt".into())
        );

        let filter2 = vec!["dir1/"];
        assert_eq!(
            strip_dir("dir1/sub/a.txt", Some(&filter2)),
            Some("sub/a.txt".into())
        );

        let filter3: Vec<&str> = vec![];
        assert_eq!(strip_dir("dir1/sub/a.txt", Some(&filter3)), None);
    }

    // ─── Integration tests ────────────────────────────────────────

    fn tmpdir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("hok_test_{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn create_test_zip(path: &std::path::Path) {
        use zip::write::FileOptions;
        use zip::CompressionMethod;
        use zip::ZipWriter;

        let file = std::fs::File::create(path).unwrap();
        let mut zip = ZipWriter::new(file);
        let opts: FileOptions<'_, ()> = FileOptions::default().compression_method(CompressionMethod::Stored);

        zip.add_directory("root/", opts).unwrap();
        zip.start_file("root/hello.txt", opts).unwrap();
        zip.write_all(b"Hello, World!").unwrap();
        zip.add_directory("root/sub/", opts).unwrap();
        zip.start_file("root/sub/deep.txt", opts).unwrap();
        zip.write_all(b"Deep content").unwrap();
        zip.finish().unwrap();
    }

    fn create_test_tar_gz(path: &std::path::Path) {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use tar::Builder;

        let stagedir = tmpdir("tar_staging");
        std::fs::create_dir_all(stagedir.join("root/sub")).unwrap();
        std::fs::write(stagedir.join("root/hello.txt"), b"Hello, World!").unwrap();
        std::fs::write(stagedir.join("root/sub/deep.txt"), b"Deep content").unwrap();

        let file = std::fs::File::create(path).unwrap();
        let encoder = GzEncoder::new(file, Compression::fast());
        let mut tar = Builder::new(encoder);
        tar.append_path_with_name(stagedir.join("root/hello.txt"), "root/hello.txt")
            .unwrap();
        tar.append_path_with_name(stagedir.join("root/sub/deep.txt"), "root/sub/deep.txt")
            .unwrap();

        let encoder = tar.into_inner().unwrap();
        encoder.finish().unwrap();

        let _ = std::fs::remove_dir_all(&stagedir);
    }

    #[test]
    fn test_extract_zip_basic() {
        let dir = tmpdir("extract_zip_basic");
        let archive_path = dir.join("test.zip");
        create_test_zip(&archive_path);

        let dest = dir.join("out");
        std::fs::create_dir_all(&dest).unwrap();

        extract(&archive_path, &dest, None, None, false).unwrap();

        assert!(dest.join("root/hello.txt").exists());
        assert_eq!(
            std::fs::read_to_string(dest.join("root/hello.txt")).unwrap(),
            "Hello, World!"
        );
        assert!(dest.join("root/sub/deep.txt").exists());
        assert_eq!(
            std::fs::read_to_string(dest.join("root/sub/deep.txt")).unwrap(),
            "Deep content"
        );
    }

    #[test]
    fn test_extract_zip_with_extract_dir() {
        let dir = tmpdir("extract_zip_dir");
        let archive_path = dir.join("test.zip");
        create_test_zip(&archive_path);

        let dest = dir.join("out");
        std::fs::create_dir_all(&dest).unwrap();

        let filter = vec!["root/sub"];
        extract(&archive_path, &dest, Some(&filter), None, false).unwrap();

        // hello.txt not extracted (not under root/sub)
        assert!(!dest.join("root/hello.txt").exists());
        // deep.txt extracted with prefix stripped
        assert!(dest.join("deep.txt").exists());
        assert_eq!(
            std::fs::read_to_string(dest.join("deep.txt")).unwrap(),
            "Deep content"
        );
    }

    #[test]
    fn test_extract_zip_with_extract_to() {
        let dir = tmpdir("extract_zip_to");
        let archive_path = dir.join("test.zip");
        create_test_zip(&archive_path);

        let dest = dir.join("out");
        std::fs::create_dir_all(&dest).unwrap();

        let subdir = vec!["myapp"];
        extract(&archive_path, &dest, None, Some(&subdir), false).unwrap();

        // All files go under myapp/
        assert!(dest.join("myapp/root/hello.txt").exists());
        assert!(dest.join("myapp/root/sub/deep.txt").exists());
    }

    #[test]
    fn test_extract_tar_gz_basic() {
        let dir = tmpdir("extract_tar_gz");
        let archive_path = dir.join("test.tar.gz");
        create_test_tar_gz(&archive_path);

        let dest = dir.join("out");
        std::fs::create_dir_all(&dest).unwrap();

        extract(&archive_path, &dest, None, None, false).unwrap();

        assert!(dest.join("root/hello.txt").exists());
        assert_eq!(
            std::fs::read_to_string(dest.join("root/hello.txt")).unwrap(),
            "Hello, World!"
        );
        assert!(dest.join("root/sub/deep.txt").exists());
    }

    #[test]
    fn test_extract_invalid_file() {
        let dir = tmpdir("extract_invalid");
        let archive_path = dir.join("not_an_archive.zip");
        std::fs::write(&archive_path, b"this is not a zip file").unwrap();

        let dest = dir.join("out");
        std::fs::create_dir_all(&dest).unwrap();

        let result = extract(&archive_path, &dest, None, None, false);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::ExtractionFailed(msg) => {
                assert!(msg.contains("zip error"));
            }
            _ => panic!("expected ExtractionFailed error"),
        }
    }

    #[test]
    fn test_unknown_format() {
        let dir = tmpdir("unknown_format");
        let archive_path = dir.join("data.bin");
        std::fs::write(&archive_path, b"some random data").unwrap();

        let dest = dir.join("out");
        std::fs::create_dir_all(&dest).unwrap();

        let result = extract(&archive_path, &dest, None, None, false);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::ExtractionFailed(msg) => {
                assert!(msg.contains("unknown archive format"));
            }
            _ => panic!("expected ExtractionFailed error"),
        }
    }

    #[test]
    fn test_innosetup_not_implemented() {
        let dir = tmpdir("innosetup_notimpl");
        let archive_path = dir.join("setup.exe");
        std::fs::write(&archive_path, b"dummy exe").unwrap();

        let dest = dir.join("out");
        std::fs::create_dir_all(&dest).unwrap();

        let result = extract(&archive_path, &dest, None, None, true);
        // Should fail to parse as Inno Setup (not a valid PE/Inno installer)
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::ExtractionFailed(msg) => {
                assert!(
                    msg.contains("innospect") || msg.contains("parse"),
                    "unexpected error: {msg}"
                );
            }
            _ => panic!("expected ExtractionFailed error"),
        }
    }
}
