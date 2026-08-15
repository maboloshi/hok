//! Archive extraction for downloaded packages.
//!
//! Detects the archive format from the filename extension and extracts
//! it to the target directory using external tools (7z) or built-in
//! support (zip, tar).
//!
//! # Design
//!
//! - **Format detection**: `detect_format()` maps extensions (`.7z`,
//!   `.zip`, `.tar`, `.gz`, `.xz`, etc.) to archive types.
//! - **External tools**: 7z archives use the `7z` executable; other
//!   formats are handled natively.
//! - **Path traversal protection**: Extraction checks that extracted
//!   entries do not escape the target directory (via
//!   `std::path::Component::ParentDir` checks).
//! - **Event emission**: Progress events are sent via the event bus
//!   during extraction of multi-file archives.

use std::io::Read;
use std::path::{Component, Path};
use std::process::Command;

use crate::error::Fallible;
use crate::event::Event;
use crate::Error;
use flume::Sender;

/// Detect archive format from filename extension.
///
/// Single source of truth for "is this a downloadable archive?" — also used
/// by [`crate::package::operations::extract`] to decide which downloaded
/// files to decompress, so the two lists cannot drift apart.
pub(crate) fn detect_format(filename: &str) -> Option<&'static str> {
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
    None
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
    emitter: &Option<Sender<Event>>,
) -> Fallible<()> {
    // Compute effective extract_to path (join all subdirs components)
    let effective_dest = match extract_to {
        Some(subdirs) if !subdirs.is_empty() => dest_dir.join(subdirs.join("/")),
        _ => dest_dir.to_path_buf(),
    };
    crate::internal::fs::ensure_dir(&effective_dest)?;

    if innosetup {
        return extract_innosetup(cache_path, &effective_dest, extract_dir, emitter);
    }

    let filename = cache_path.file_name().unwrap_or_default().to_string_lossy();
    let fmt = detect_format(&filename)
        .ok_or_else(|| Error::ExtractionFailed(format!("unknown archive format: {}", filename)))?;

    match fmt {
        "7z" => extract_7z(cache_path, &effective_dest, extract_dir, emitter),
        "zip" => extract_zip(cache_path, &effective_dest, extract_dir, emitter),
        "tar" => extract_tar(cache_path, &effective_dest, extract_dir, None, emitter),
        "gz" => extract_tar(
            cache_path,
            &effective_dest,
            extract_dir,
            Some(Compression::Gzip),
            emitter,
        ),
        "bz2" => extract_tar(
            cache_path,
            &effective_dest,
            extract_dir,
            Some(Compression::Bzip2),
            emitter,
        ),
        "xz" => extract_tar(
            cache_path,
            &effective_dest,
            extract_dir,
            Some(Compression::Xz),
            emitter,
        ),
        "zst" => extract_tar(
            cache_path,
            &effective_dest,
            extract_dir,
            Some(Compression::Zstd),
            emitter,
        ),
        "rar" => extract_rar(cache_path, &effective_dest, extract_dir, emitter),
        "lzh" | "iso" => extract_with_7z_exe(cache_path, &effective_dest, emitter),
        _ => Err(Error::ExtractionFailed(format!(
            "unsupported format: {}",
            fmt
        ))),
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

/// Installer signatures that embed standard 7z payloads. Both markers live
/// in the PE stub (the file head), so a bounded search is sufficient.
const NSIS_MARKER: &[u8] = b"Nullsoft Install System";
const INNO_MARKER: &[u8] = b"Inno Setup";
const INSTALLER_SIGNATURE_SCAN_LIMIT: usize = 1024 * 1024; // 1 MiB

/// NSIS overlay FirstHeader magic (`flags + 0xDEADBEEF + "NullsoftInst"`).
///
/// Present in *every* NSIS installer, unlike the stub version string
/// `Nullsoft Install System` which stripped/custom stubs (e.g. AltSnap)
/// omit. 7-Zip identifies NSIS by this same signature.
const NSIS_FIRST_HEADER_MAGIC: &[u8] = b"NullsoftInst";

/// True when `data` is an NSIS installer — a format that embeds 7z archives
/// as *payloads* (`$PLUGINSDIR\app-64.7z` in electron-builder), so a raw
/// magic match would not denote a 7z SFX.
fn is_nsis_installer(data: &[u8]) -> bool {
    let head = &data[..data.len().min(INSTALLER_SIGNATURE_SCAN_LIMIT)];
    head.windows(NSIS_MARKER.len()).any(|w| w == NSIS_MARKER)
        || head
            .windows(NSIS_FIRST_HEADER_MAGIC.len())
            .any(|w| w == NSIS_FIRST_HEADER_MAGIC)
}

/// True when `data` is an NSIS or Inno Setup installer — formats that embed
/// 7z archives as *payloads* (`$PLUGINSDIR\app-64.7z` in electron-builder,
/// `{tmp}` blobs in Inno), so a raw magic match would not denote a 7z SFX.
fn is_installer_pe(data: &[u8]) -> bool {
    let head = &data[..data.len().min(INSTALLER_SIGNATURE_SCAN_LIMIT)];
    is_nsis_installer(data) || head.windows(INNO_MARKER.len()).any(|w| w == INNO_MARKER)
}

/// True when a PE file may be a 7z SFX whose archive data starts after the
/// stub — i.e. it is not a recognised NSIS / Inno Setup installer.
fn pe_embedded_sfx_searchable(data: &[u8]) -> bool {
    data.starts_with(b"MZ") && !is_installer_pe(data)
}

fn extract_7z(
    src: &Path,
    dest: &Path,
    filter: Option<&[&str]>,
    emitter: &Option<Sender<Event>>,
) -> Fallible<()> {
    use sevenz_rust2::{ArchiveReader, Password};

    // Fast path: open as standard 7z archive directly (streaming, no full file load).
    if let Ok(file) = std::fs::File::open(src) {
        if let Ok(reader) = ArchiveReader::new(file, Password::empty()) {
            return extract_7z_entries(reader, dest, filter, emitter);
        }
    }

    // PE/SFX: read into memory and search for embedded 7z data.
    // Many installers append the real 7z archive after the PE stub.
    let file_data = std::fs::read(src)
        .map_err(|e| Error::ExtractionFailed(format!("cannot read {}: {}", src.display(), e)))?;

    // Skip the embedded-7z search for NSIS / Inno Setup installers: those
    // formats legitimately embed standard 7z payloads (`$PLUGINSDIR\app-64.7z`
    // in electron-builder installers, `{tmp}` blobs in Inno). Blindly matching
    // the 7z magic would extract that inner archive directly and skip the
    // installer's directory structure — 7-Zip preserves it instead.
    if pe_embedded_sfx_searchable(&file_data) {
        const MAGIC_7Z: &[u8] = &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];
        if let Some(pos) = file_data
            .windows(MAGIC_7Z.len())
            .position(|w| w == MAGIC_7Z)
        {
            if let Ok(reader) =
                ArchiveReader::new(std::io::Cursor::new(&file_data[pos..]), Password::empty())
            {
                return extract_7z_entries(reader, dest, filter, emitter);
            }
        }
    }

    // NSIS installers: extract the installer structure (including
    // `$PLUGINSDIR\app-64.7z`) with the pure-Rust nsis crate. On any
    // parse/extract failure fall through to 7z.exe below — never a hard
    // error from the Rust path alone.
    if is_nsis_installer(&file_data) && extract_nsis(src, dest, filter, emitter).is_ok() {
        return Ok(());
    }

    // Fall back to external 7z.exe which handles 7z SFX, Inno, NSIS, etc.
    extract_with_7z_exe(src, dest, emitter)
}

/// Extract entries from an already-opened 7z ArchiveReader.
fn extract_7z_entries<R: std::io::Read + std::io::Seek>(
    mut reader: sevenz_rust2::ArchiveReader<R>,
    dest: &Path,
    filter: Option<&[&str]>,
    emitter: &Option<Sender<Event>>,
) -> Fallible<()> {
    use sevenz_rust2::ArchiveEntry;

    let entries: Vec<String> = reader
        .archive()
        .files
        .iter()
        .filter(|e: &&ArchiveEntry| !e.is_directory())
        .filter(|e| {
            filter
                .map(|f| f.iter().any(|d| e.name().starts_with(d)))
                .unwrap_or(true)
        })
        .map(|e: &ArchiveEntry| e.name().to_string())
        .collect();

    let total = entries.len();
    for (i, name) in entries.iter().enumerate() {
        if let Some(tx) = emitter {
            let _ = tx.send(Event::PackageExtractProgress(format!(
                "{} ({}/{})",
                name,
                i + 1,
                total
            )));
        }
        let data = reader
            .read_file(name)
            .map_err(|e| Error::ExtractionFailed(format!("failed to read '{name}': {e}")))?;
        let target = strip_dir(name, filter).unwrap_or_else(|| name.to_string());
        if Path::new(&target)
            .components()
            .any(|c| c == Component::ParentDir)
        {
            return Err(Error::PathTraversalDetected(format!("7z: {}", target)));
        }
        let target_path = dest.join(&target);
        if let Some(parent) = target_path.parent() {
            crate::internal::fs::ensure_dir(parent)?;
        }
        std::fs::write(&target_path, &data)?;
    }
    Ok(())
}

// ─── Zip extraction via zip crate ────────────────────────────────────

fn extract_zip(
    src: &Path,
    dest: &Path,
    filter: Option<&[&str]>,
    emitter: &Option<Sender<Event>>,
) -> Fallible<()> {
    use std::fs::File;
    use zip::ZipArchive;

    let file = File::open(src)?;
    let mut archive = ZipArchive::new(file)
        .map_err(|e| Error::ExtractionFailed(format!("zip error for {}: {}", src.display(), e)))?;
    let total = archive.len();

    for i in 0..total {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| Error::ExtractionFailed(format!("zip read error: {}", e)))?;
        let name = entry.name().to_string();
        if name.ends_with('/') {
            continue;
        }
        if let Some(f) = filter {
            if !f.iter().any(|d| name.starts_with(d)) {
                continue;
            }
        }
        if let Some(tx) = emitter {
            let _ = tx.send(Event::PackageExtractProgress(format!(
                "{} ({}/{})",
                name,
                i + 1,
                total
            )));
        }
        let target = strip_dir(&name, filter).unwrap_or(name);
        if Path::new(&target)
            .components()
            .any(|c| c == Component::ParentDir)
        {
            return Err(Error::PathTraversalDetected(format!("zip: {}", target)));
        }
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
    emitter: &Option<Sender<Event>>,
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
        let entries = archive.entries()?;
        for entry in entries {
            let mut entry = entry?;
            let path = entry.path()?.to_string_lossy().to_string();
            if let Some(f) = filter {
                if !f.iter().any(|d| path.starts_with(d)) {
                    continue;
                }
            }
            if let Some(tx) = emitter {
                let _ = tx.send(Event::PackageExtractProgress(path.clone()));
            }
            let target = strip_dir(&path, filter).unwrap_or(path);
            if Path::new(&target)
                .components()
                .any(|c| c == Component::ParentDir)
            {
                return Err(Error::PathTraversalDetected(format!("tar: {}", target)));
            }
            let target_path = dest.join(&target);
            if entry.header().entry_type().is_dir() {
                crate::internal::fs::ensure_dir(&target_path)?;
                continue;
            }
            if let Some(parent) = target_path.parent() {
                crate::internal::fs::ensure_dir(parent)?;
            }
            let mut data = Vec::new();
            entry.read_to_end(&mut data)?;
            std::fs::write(&target_path, &data)?;
        }
    } else {
        let mut archive = TarArchive::new(reader);
        archive.unpack(dest)?;
    }
    Ok(())
}

// ─── RAR extraction via unrar crate ─────────────────────────────────

fn extract_rar(
    src: &Path,
    dest: &Path,
    filter: Option<&[&str]>,
    emitter: &Option<Sender<Event>>,
) -> Fallible<()> {
    use unrar::Archive;

    let mut archive = Archive::new(src)
        .open_for_processing()
        .map_err(|e| Error::ExtractionFailed(format!("cannot open rar: {}", e)))?;

    loop {
        let entry = match archive.read_header() {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(e) => return Err(Error::ExtractionFailed(format!("rar read header: {}", e))),
        };

        let name = entry.entry().filename.to_string_lossy().to_string();
        if let Some(f) = filter {
            if !f.iter().any(|d| name.starts_with(d)) {
                archive = entry
                    .skip()
                    .map_err(|e| Error::ExtractionFailed(format!("rar skip: {}", e)))?;
                continue;
            }
        }
        if let Some(tx) = emitter {
            let _ = tx.send(Event::PackageExtractProgress(name.clone()));
        }

        let target = strip_dir(&name, filter).unwrap_or(name);
        if Path::new(&target)
            .components()
            .any(|c| c == Component::ParentDir)
        {
            return Err(Error::PathTraversalDetected(format!("rar: {}", target)));
        }
        let target_path = dest.join(&target);

        if entry.entry().is_directory() {
            crate::internal::fs::ensure_dir(&target_path)?;
            archive = entry
                .skip()
                .map_err(|e| Error::ExtractionFailed(format!("rar skip dir: {}", e)))?;
        } else {
            if let Some(parent) = target_path.parent() {
                crate::internal::fs::ensure_dir(parent)?;
            }
            let (data, rest) = entry
                .read()
                .map_err(|e| Error::ExtractionFailed(format!("rar read: {}", e)))?;
            std::fs::write(&target_path, &data)?;
            archive = rest;
        }
    }
    Ok(())
}

// ─── Fallback: call external 7z.exe for ISO ─────────────────────────

fn extract_with_7z_exe(src: &Path, dest: &Path, emitter: &Option<Sender<Event>>) -> Fallible<()> {
    // Look for 7z.exe in PATH or in Scoop's shims directory
    let path_env = std::env::var("SCOOP").unwrap_or_default();
    let shims_7z = std::path::Path::new(&path_env).join("shims").join("7z.exe");
    let exe = if shims_7z.exists() {
        shims_7z
    } else {
        std::path::PathBuf::from("7z.exe")
    };

    let mut child = Command::new(&exe)
        .arg("x")
        .arg(src)
        .arg(format!("-o{}", dest.display()))
        .arg("-y")
        .arg("-bso0")
        .arg("-bsp1")
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            Error::ExtractionFailed(format!(
                "failed to launch 7z.exe (install '7zip' via hok first): {}",
                e
            ))
        })?;

    // Parse progress from stderr (format: "\r69% 6143")
    if let Some(stderr) = child.stderr.take() {
        use std::io::{BufRead, BufReader};
        let reader = BufReader::new(stderr);
        for text in reader.lines().map_while(Result::ok) {
            if let Some(pct) = text
                .trim()
                .split('%')
                .next()
                .and_then(|s| s.parse::<u8>().ok())
            {
                if let Some(tx) = emitter {
                    let _ = tx.send(Event::PackageExtractProgress(format!("7z {}%", pct)));
                }
            }
        }
    }

    let status = child
        .wait()
        .map_err(|e| Error::ExtractionFailed(format!("failed to wait for 7z.exe: {}", e)))?;

    if !status.success() {
        return Err(Error::ExtractionFailed(format!(
            "7z.exe exited with code {:?}",
            status.code()
        )));
    }
    Ok(())
}

// ─── Inno Setup extraction via innospect ─────────────────────────────

fn extract_innosetup(
    src: &Path,
    dest: &Path,
    filter: Option<&[&str]>,
    emitter: &Option<Sender<Event>>,
) -> Fallible<()> {
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
        if let Some(tx) = emitter {
            let _ = tx.send(Event::PackageExtractProgress(name.to_string()));
        }
        let target = strip_dir(name, filter).unwrap_or_else(|| name.to_string());
        if Path::new(&target)
            .components()
            .any(|c| c == Component::ParentDir)
        {
            return Err(Error::PathTraversalDetected(format!(
                "innosetup: {}",
                target
            )));
        }
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

// ─── NSIS extraction via the nsis crate ──────────────────────────────

fn extract_nsis(
    src: &Path,
    dest: &Path,
    filter: Option<&[&str]>,
    emitter: &Option<Sender<Event>>,
) -> Fallible<()> {
    let data = std::fs::read(src)
        .map_err(|e| Error::ExtractionFailed(format!("cannot read {}: {}", src.display(), e)))?;

    let installer = nsis::NsisInstaller::from_bytes(&data)
        .map_err(|e| Error::ExtractionFailed(format!("nsis parse error: {}", e)))?;

    // 7-Zip derives each file's destination from the *instruction stream*:
    // EW_CREATEDIR (update_instdir) / EW_ASSIGNVAR($OUTDIR) track the current
    // install directory, and every EW_EXTRACTFILE is written under it with
    // the `$INSTDIR` prefix stripped. `files()` only exposes the bare
    // EXTRACTFILE target name, so the directory is tracked separately:
    // pass 1 records the directory in effect for each EXTRACTFILE (stream
    // order), pass 2 pairs it with each file from `files()` (same order).
    let dirs = nsis_extract_dirs(&installer)?;
    let mut dir_idx = 0usize;

    for result in installer.files() {
        let file =
            result.map_err(|e| Error::ExtractionFailed(format!("nsis file error: {}", e)))?;
        let name = file
            .name()
            .map_err(|e| Error::ExtractionFailed(format!("nsis name error: {}", e)))?
            .to_string();
        if name.is_empty() {
            continue;
        }
        let dir = dirs.get(dir_idx).map(String::as_str).unwrap_or("");
        dir_idx += 1;

        let rel = nsis_dest_path(dir, &name);
        if rel.is_empty() {
            continue;
        }
        if let Some(f) = filter {
            if !f.iter().any(|d| rel.starts_with(d)) {
                continue;
            }
        }
        if let Some(tx) = emitter {
            let _ = tx.send(Event::PackageExtractProgress(rel.clone()));
        }
        // NSIS paths use backslashes and keep their `$VAR` directories
        // literally (`$PLUGINSDIR\app-64.7z`) — pre_install scripts refer to
        // them by that exact name, matching what 7z.exe produces.
        let rel_slashes = rel.replace('\\', "/");
        let target = strip_dir(&rel_slashes, filter).unwrap_or(rel_slashes);
        if Path::new(&target)
            .components()
            .any(|c| c == Component::ParentDir)
        {
            return Err(Error::PathTraversalDetected(format!("nsis: {}", target)));
        }
        let target_path = dest.join(&target);
        if let Some(parent) = target_path.parent() {
            crate::internal::fs::ensure_dir(parent)?;
        }
        let content = file
            .decompress()
            .map_err(|e| Error::ExtractionFailed(format!("nsis decompress '{}': {}", name, e)))?;
        std::fs::write(&target_path, &content).map_err(|e| {
            Error::ExtractionFailed(format!("cannot write {}: {}", target_path.display(), e))
        })?;
    }

    // pass 3 — uninstaller stub: 7-Zip also emits `Uninstall.exe` from the
    // EW_WRITEUNINSTALLER payload (the crate skips the icon/patch prefix and
    // re-attaches the installer's own PE stub to the decompressed overlay).
    for result in installer.uninstallers() {
        let uninstaller = result
            .map_err(|e| Error::ExtractionFailed(format!("nsis uninstaller error: {}", e)))?;
        let rel = uninstaller
            .path()
            .map_err(|e| Error::ExtractionFailed(format!("nsis uninstaller name error: {}", e)))
            .map(|n| nsis_dest_path("", &n.to_string()))?;
        if rel.is_empty() {
            continue;
        }
        if let Some(f) = filter {
            if !f.iter().any(|d| rel.starts_with(d)) {
                continue;
            }
        }
        if let Some(tx) = emitter {
            let _ = tx.send(Event::PackageExtractProgress(rel.clone()));
        }
        let rel_slashes = rel.replace('\\', "/");
        let target = strip_dir(&rel_slashes, filter).unwrap_or(rel_slashes);
        if Path::new(&target)
            .components()
            .any(|c| c == Component::ParentDir)
        {
            return Err(Error::PathTraversalDetected(format!("nsis: {}", target)));
        }
        let target_path = dest.join(&target);
        if let Some(parent) = target_path.parent() {
            crate::internal::fs::ensure_dir(parent)?;
        }
        // A broken uninstaller overlay must not fail the whole extraction
        // (the payload files above are already written).
        let Ok(content) = uninstaller.decompress() else {
            continue;
        };
        std::fs::write(&target_path, &content).map_err(|e| {
            Error::ExtractionFailed(format!("cannot write {}: {}", target_path.display(), e))
        })?;
    }
    Ok(())
}

/// Track the current install directory through the NSIS instruction stream,
/// recording the directory in effect at each `EW_EXTRACTFILE` (stream order).
///
/// Mirrors 7-Zip's NsisIn.cpp: `EW_CREATEDIR` with `update_instdir` set and
/// `EW_ASSIGNVAR` targeting `$OUTDIR` (variable index 22) both update the
/// current directory; subsequent files are extracted under it. A file name
/// that already contains a separator (`$PLUGINSDIR\app-64.7z`) is used as-is
/// and does not need a directory from the stream.
fn nsis_extract_dirs(installer: &nsis::NsisInstaller<'_>) -> Fallible<Vec<String>> {
    const OUTDIR_VAR: i32 = 22; // $OUTDIR
    let mut dirs: Vec<String> = Vec::new();
    let mut cur_dir: Option<String> = None;
    for result in installer.entries() {
        let entry =
            result.map_err(|e| Error::ExtractionFailed(format!("nsis entry error: {}", e)))?;
        match installer.normalize_opcode(entry.which()) {
            nsis::opcode::EW_CREATEDIR => {
                let path = installer
                    .read_string(entry.offset(0))
                    .map_err(|e| Error::ExtractionFailed(format!("nsis createdir error: {}", e)))?
                    .to_string();
                if entry.offset(1) != 0 {
                    cur_dir = Some(path);
                }
            }
            nsis::opcode::EW_ASSIGNVAR => {
                if entry.offset(0) == OUTDIR_VAR {
                    let path = installer
                        .read_string(entry.offset(1))
                        .map_err(|e| {
                            Error::ExtractionFailed(format!("nsis assignvar error: {}", e))
                        })?
                        .to_string();
                    cur_dir = Some(path);
                }
            }
            nsis::opcode::EW_EXTRACTFILE => {
                dirs.push(cur_dir.clone().unwrap_or_default());
            }
            _ => {}
        }
    }
    Ok(dirs)
}

/// 7-Zip-style destination for an NSIS `EW_EXTRACTFILE`: join the tracked
/// current directory with the bare target name and strip the `$INSTDIR`
/// install-root prefix (the extraction root). A name that already contains a
/// directory separator is used verbatim.
fn nsis_dest_path(dir: &str, name: &str) -> String {
    let joined = if name.contains('\\') || name.contains('/') {
        name.to_string()
    } else if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}\\{name}")
    };
    joined
        .strip_prefix("$INSTDIR")
        .map(|rest| rest.trim_start_matches('\\').trim_start_matches('/'))
        .unwrap_or(&joined)
        .to_string()
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

    /// Extract an electron-builder NSIS installer (the real-world
    /// $PLUGINSDIR\app-64.7z payload) and verify the payload file is
    /// written with its literal directory. Ignored by default — run
    /// manually on a machine that has the sample (a renamed NSIS installer
    /// at `D:\App\Scoop\cache\Obsidian#1.13.6#3cb95fb.7z`). Verified
    /// 2026-08-12: the extracted app-64.7z (114 MiB) matched 7z.exe output
    /// byte-for-byte.
    #[test]
    #[ignore = "requires local NSIS installer sample"]
    fn extract_nsis_electron_builder_corpus() {
        let path = r"D:\App\Scoop\cache\Obsidian#1.13.6#3cb95fb.7z";
        let dest = crate::test_utils::tmpdir("nsis_obsidian");
        extract_nsis(std::path::Path::new(path), &dest, None, &None).unwrap();
        // pre_install scripts reference the payload by its literal $PLUGINSDIR
        // path (Expand-7zipArchive $dir\$PLUGINSDIR\app-64.7z).
        let app7z = dest.join("$PLUGINSDIR").join("app-64.7z");
        assert!(app7z.exists(), "missing $PLUGINSDIR\\app-64.7z");
        assert!(std::fs::metadata(&app7z)
            .map(|m| m.len() > 0)
            .unwrap_or(false));
    }

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
    fn test_is_installer_pe() {
        // NSIS (electron-builder etc.) — marker in the stub
        let nsis = b"MZ\x90\x00\x03\x00\x00\x00\x04\x00\x00\x00\xff\xff\x00\x00\
Nullsoft Install System v3.08\x00\x00\x00\x00uninstall.exe";
        assert!(is_installer_pe(nsis));

        // NSIS with a stripped/custom stub that only carries the overlay
        // FirstHeader magic (`NullsoftInst`) — e.g. AltSnap 1.67.
        let first_header_only = b"MZ\x90\x00\x03\x00\x00\x00\x04\x00\x00\x00\xff\xff\x00\x00\
\xef\xbe\xad\xdeNullsoftInst\x00\x00\x00\x00";
        assert!(is_installer_pe(first_header_only));

        // Inno Setup — marker in the stub
        let inno = b"MZ\x90\x00\x03\x00\x00\x00\x04\x00\x00\x00\xff\xff\x00\x00\
Inno Setup Setup Data (5.8.3)";
        assert!(is_installer_pe(inno));

        // Plain PE (e.g. a real 7z SFX stub) — no installer marker
        let sfx = b"MZ\x90\x00\x03\x00\x00\x00\x04\x00\x00\x00\xff\xff\x00\x00\
\x37\x7a\xbc\xaf\x27\x1c\x00\x04";
        assert!(!is_installer_pe(sfx));

        // Non-PE data
        assert!(!is_installer_pe(b"7z\xbc\xaf\x27\x1c random data"));

        // Marker beyond the 1 MiB scan limit is ignored (payload area)
        let mut deep = Vec::from(b"MZ\x90\x00\x03".as_slice());
        deep.resize(INSTALLER_SIGNATURE_SCAN_LIMIT + 64, 0);
        deep[INSTALLER_SIGNATURE_SCAN_LIMIT + 10..][..NSIS_MARKER.len()]
            .copy_from_slice(NSIS_MARKER);
        assert!(!is_installer_pe(&deep));

        // ...and the FirstHeader magic is likewise bounded by the scan limit
        let mut deep2 = Vec::from(b"MZ\x90\x00\x03".as_slice());
        deep2.resize(INSTALLER_SIGNATURE_SCAN_LIMIT + 64, 0);
        deep2[INSTALLER_SIGNATURE_SCAN_LIMIT + 10..][..NSIS_FIRST_HEADER_MAGIC.len()]
            .copy_from_slice(NSIS_FIRST_HEADER_MAGIC);
        assert!(!is_installer_pe(&deep2));
    }

    #[test]
    fn test_pe_embedded_sfx_searchable() {
        // NSIS/Inno installers never take the embedded-7z search path
        let nsis = b"MZ\x90\x00\x03Nullsoft Install System v3.08";
        assert!(!pe_embedded_sfx_searchable(nsis));
        // FirstHeader-magic-only NSIS (AltSnap-style) is excluded too
        let fh = b"MZ\x90\x00\x03\xef\xbe\xad\xdeNullsoftInst";
        assert!(!pe_embedded_sfx_searchable(fh));
        let inno = b"MZ\x90\x00\x03Inno Setup Setup Data (5.8.3)";
        assert!(!pe_embedded_sfx_searchable(inno));

        // A plain PE without installer markers may be a 7z SFX
        let sfx = b"MZ\x90\x00\x03\x00\x00\x00\x00\x00";
        assert!(pe_embedded_sfx_searchable(sfx));

        // Non-PE never searches
        assert!(!pe_embedded_sfx_searchable(b"not a PE"));
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

    #[test]
    fn test_nsis_dest_path() {
        // Bare name under the install root ($INSTDIR is the extraction root).
        assert_eq!(nsis_dest_path("$INSTDIR", "payload.txt"), "payload.txt");
        // Bare name under a tracked subdirectory — the traditional-NSIS case
        // that 7-Zip preserves (e.g. AltSnap's `Lang\de_DE.ini`).
        assert_eq!(
            nsis_dest_path("$INSTDIR\\docs", "payload.txt"),
            "docs\\payload.txt"
        );
        // No directory tracked yet (no leading SetOutPath): bare name.
        assert_eq!(nsis_dest_path("", "config.ini"), "config.ini");
        // Tracked directory without an $INSTDIR prefix (relative SetOutPath).
        assert_eq!(nsis_dest_path("Lang", "de_DE.ini"), "Lang\\de_DE.ini");
        // electron-builder: the name already carries its literal $VAR
        // directory, kept verbatim (pre_install scripts reference it as-is).
        assert_eq!(
            nsis_dest_path("$INSTDIR", "$PLUGINSDIR\\app-64.7z"),
            "$PLUGINSDIR\\app-64.7z"
        );
        // Name with a plain directory is used verbatim too.
        assert_eq!(nsis_dest_path("$INSTDIR\\a", "sub\\x.bin"), "sub\\x.bin");
    }

    /// True when `dir` contains at least one file in a nested subdirectory.
    fn has_nested_file(dir: &std::path::Path) -> bool {
        fn walk(dir: &std::path::Path, nested: bool) -> bool {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return false;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() && walk(&p, true) {
                    return true;
                }
                if nested && p.is_file() {
                    return true;
                }
            }
            false
        }
        walk(dir, false)
    }

    /// Extract a traditional NSIS installer (subdirectories via EW_CREATEDIR
    /// tracking) and verify the directory structure survives — the 7-Zip
    /// parity fix. Point `NSIS_SAMPLE` at such an installer (e.g. AltSnap's
    /// setup exe) to run manually.
    #[test]
    #[ignore = "requires a traditional NSIS installer sample (NSIS_SAMPLE env)"]
    fn extract_nsis_traditional_dir_corpus() {
        let Ok(path) = std::env::var("NSIS_SAMPLE") else {
            return;
        };
        let dest = crate::test_utils::tmpdir("nsis_traditional");
        extract_nsis(std::path::Path::new(&path), &dest, None, &None).unwrap();
        assert!(
            has_nested_file(&dest),
            "expected a nested directory in extraction of {path}"
        );
    }

    // ─── Integration tests ────────────────────────────────────────

    use crate::test_utils::tmpdir;

    fn create_test_zip(path: &std::path::Path) {
        use zip::write::FileOptions;
        use zip::CompressionMethod;
        use zip::ZipWriter;

        let file = std::fs::File::create(path).unwrap();
        let mut zip = ZipWriter::new(file);
        let opts: FileOptions<'_, ()> =
            FileOptions::default().compression_method(CompressionMethod::Stored);

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

        extract(&archive_path, &dest, None, None, false, &None).unwrap();

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
        extract(&archive_path, &dest, Some(&filter), None, false, &None).unwrap();

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
        extract(&archive_path, &dest, None, Some(&subdir), false, &None).unwrap();

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

        extract(&archive_path, &dest, None, None, false, &None).unwrap();

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

        let result = extract(&archive_path, &dest, None, None, false, &None);
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

        let result = extract(&archive_path, &dest, None, None, false, &None);
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

        let result = extract(&archive_path, &dest, None, None, true, &None);
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
