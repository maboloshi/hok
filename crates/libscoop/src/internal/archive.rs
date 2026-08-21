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

use std::io::{Read, Seek};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

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
        "7z" => extract_pseudo_7z(cache_path, &effective_dest, extract_dir, emitter),
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

/// Installer signature detected in the file head.
///
/// A single-pass classification: all markers live in the first 1 MiB of a PE
/// stub, so one bounded scan answers "is this an NSIS / Inno Setup installer,
/// and which one?" — used by both the SFX probe (which must *exclude*
/// installers) and the dispatch in [`extract_pseudo_7z`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstallerKind {
    /// Not a recognised NSIS / Inno Setup installer.
    None,
    Nsis,
    Inno,
}

/// Classify the installer format from the file head.
///
/// `NSIS_MARKER` / `NSIS_FIRST_HEADER_MAGIC` and `INNO_MARKER` are matched
/// over the first `INSTALLER_SIGNATURE_SCAN_LIMIT` bytes in one pass (the
/// Windows substring search is cheap; avoiding three separate 1 MiB scans
/// matters less than clarity, but one classifier keeps the three consumers
/// consistent by construction).
fn classify_installer(data: &[u8]) -> InstallerKind {
    let head = &data[..data.len().min(INSTALLER_SIGNATURE_SCAN_LIMIT)];
    if head.windows(NSIS_MARKER.len()).any(|w| w == NSIS_MARKER)
        || head
            .windows(NSIS_FIRST_HEADER_MAGIC.len())
            .any(|w| w == NSIS_FIRST_HEADER_MAGIC)
    {
        InstallerKind::Nsis
    } else if head.windows(INNO_MARKER.len()).any(|w| w == INNO_MARKER) {
        InstallerKind::Inno
    } else {
        InstallerKind::None
    }
}

/// Own the filter slice so it can be shared across worker threads.
fn owned_filter(filter: Option<&[&str]>) -> Option<Vec<String>> {
    filter.map(|f| f.iter().map(|s| s.to_string()).collect())
}

/// Extract a `.7z`-named cache file whose real content is unknown up front —
/// hence "pseudo-7z". Dispatches by sniffing the actual bytes: plain 7z,
/// 7z SFX / zip SFX (PE stub prefix), NSIS / Inno Setup installers, or a
/// fallback to external 7z.exe.
fn extract_pseudo_7z(
    src: &Path,
    dest: &Path,
    filter: Option<&[&str]>,
    emitter: &Option<Sender<Event>>,
) -> Fallible<()> {
    // Read the whole file into memory once: block-parallel decoding shares
    // this buffer across worker threads. A plain 7z archive starts at
    // offset 0; the git package is a 7z SFX whose archive follows the PE
    // stub. Shared via Arc so the zip-SFX branch can hand it to workers
    // without moving it (a failed zip parse still falls through to the
    // 7z.exe fallback below).
    let file_data =
        Arc::new(std::fs::read(src).map_err(|e| {
            Error::ExtractionFailed(format!("cannot read {}: {}", src.display(), e))
        })?);

    const MAGIC_7Z: &[u8] = &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];

    // Plain 7z archive.
    if file_data.starts_with(MAGIC_7Z) {
        return extract_7z_entries_mt(
            file_data,
            0,
            dest.to_path_buf(),
            owned_filter(filter),
            emitter.clone(),
        );
    }

    // Classify the file once (single 1 MiB head scan) and dispatch:
    // - None: any non-installer pseudo-7z may carry a 7z payload behind a PE
    //   stub (7z SFX) or a ZIP payload (behind a PE stub, e.g. Doubao, or as
    //   a plain zip misnamed .7z — the EOCD locator works for both, a
    //   prefix-less zip yields start 0). NSIS/Inno installers never take
    //   this path: they legitimately embed standard 7z payloads
    //   (`$PLUGINSDIR\app-64.7z` in electron-builder, `{tmp}` blobs in
    //   Inno) and must keep their installer structure — 7-Zip preserves it.
    // - Nsis/Inno: extract the installer structure with the pure-Rust nsis /
    //   innospect crates. The manifest's `innosetup: true` flag is the
    //   primary Inno route (exe files); this signature branch is the
    //   fallback for a `.7z`-named file whose content is actually an
    //   installer.
    // On any failure fall through to 7z.exe — never a hard error.
    match classify_installer(&file_data) {
        InstallerKind::None => {
            // PE-prefixed 7z SFX: search for embedded 7z data. The magic
            // search is bounded to PE files (a 7z SFX stub is always MZ);
            // the zip probe below has no such constraint.
            if file_data.starts_with(b"MZ") {
                if let Some(pos) = file_data
                    .windows(MAGIC_7Z.len())
                    .position(|w| w == MAGIC_7Z)
                {
                    // Validate the candidate before committing to the Rust
                    // path, so a false magic match falls through to 7z.exe.
                    let mut probe = std::io::Cursor::new(&file_data[pos..]);
                    if sevenz_rust2::Archive::read(&mut probe, &sevenz_rust2::Password::empty())
                        .is_ok()
                    {
                        return extract_7z_entries_mt(
                            file_data,
                            pos,
                            dest.to_path_buf(),
                            owned_filter(filter),
                            emitter.clone(),
                        );
                    }
                }
            }
            // ZIP SFX / plain-zip-misnamed-.7z: locate the EOCD, slice from
            // the zip start, extract with the zip crate.
            if let Some(zip_start) = find_embedded_zip_start(&file_data) {
                if extract_zip_embedded(file_data, zip_start, dest, filter, emitter).is_ok() {
                    return Ok(());
                }
            }
        }
        InstallerKind::Nsis if extract_nsis(src, dest, filter, emitter).is_ok() => {
            return Ok(());
        }
        InstallerKind::Inno if extract_innosetup(src, dest, filter, emitter).is_ok() => {
            return Ok(());
        }
        _ => {}
    }

    // Fall back to external 7z.exe which handles 7z SFX, Inno, NSIS, etc.
    extract_with_7z_exe(src, dest, emitter)
}

/// Extract a 7z archive from `data[start..]`, decoding solid blocks in
/// parallel (one worker per block, up to the CPU count).
///
/// Single-pass decoding per block via `BlockDecoder::for_each_entries` (the
/// naive per-file `read_file` re-decodes all data before the target file on
/// every call — catastrophic for solid archives like git: 56 MB, 6 solid
/// LZMA blocks, ~9.5k files). Decoding dominates (~42 s of ~61 s
/// single-threaded); block parallelism brings total extraction to ~30 s,
/// close to 7z.exe's ~29 s.
fn extract_7z_entries_mt(
    data: Arc<Vec<u8>>,
    start: usize,
    dest: PathBuf,
    filter: Option<Vec<String>>,
    emitter: Option<Sender<Event>>,
) -> Fallible<()> {
    use sevenz_rust2::{Archive, BlockDecoder, Password};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    let mut cursor = std::io::Cursor::new(&data[start..]);
    let archive = Archive::read(&mut cursor, &Password::empty())
        .map_err(|e| Error::ExtractionFailed(format!("7z parse error: {}", e)))?;
    let archive = Arc::new(archive);
    let password = Arc::new(Password::empty());
    let filter = Arc::new(filter);

    let total = archive
        .files
        .iter()
        .filter(|e: &&sevenz_rust2::ArchiveEntry| {
            !e.is_directory()
                && match filter.as_ref().as_ref() {
                    Some(f) => f.iter().any(|d| e.name().starts_with(d)),
                    None => true,
                }
        })
        .count();

    let block_count = archive.blocks.len();
    let workers = block_count
        .min(
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
        )
        .max(1);
    let next_block = Arc::new(AtomicUsize::new(0));
    let file_idx = Arc::new(AtomicUsize::new(0));
    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let handles: Vec<_> = (0..workers)
        .map(|_| {
            let data = data.clone();
            let archive = archive.clone();
            let password = password.clone();
            let filter = filter.clone();
            let next_block = next_block.clone();
            let file_idx = file_idx.clone();
            let errors = errors.clone();
            let emitter = emitter.clone();
            let dest = dest.clone();
            std::thread::spawn(move || {
                // Borrowed filter view for strip_dir; the String storage
                // lives in the Arc shared by this worker, so the borrows are
                // valid for the whole loop.
                let filter_ref: Option<Vec<&str>> = filter
                    .as_ref()
                    .as_ref()
                    .map(|f| f.iter().map(|s| s.as_str()).collect());
                loop {
                    let block_index = next_block.fetch_add(1, Ordering::Relaxed);
                    if block_index >= archive.blocks.len() {
                        break;
                    }
                    let mut source = std::io::Cursor::new(&data[start..]);
                    let decoder =
                        BlockDecoder::new(1, block_index, &archive, &password, &mut source);
                    let result = decoder.for_each_entries(&mut |entry, reader| {
                        if entry.is_directory() {
                            return Ok(true);
                        }
                        let name = entry.name();
                        if let Some(f) = filter.as_deref() {
                            if !f.iter().any(|d| name.starts_with(d)) {
                                return Ok(true);
                            }
                        }
                        let n = file_idx.fetch_add(1, Ordering::Relaxed);
                        emit_extract_progress(&emitter, n, Some(total));
                        let target = strip_dir(name, filter_ref.as_deref())
                            .unwrap_or_else(|| name.to_string());
                        if Path::new(&target)
                            .components()
                            .any(|c| c == Component::ParentDir)
                        {
                            return Err(sevenz_rust2::Error::Other(
                                format!("path traversal: {}", target).into(),
                            ));
                        }
                        let target_path = dest.join(&target);
                        if let Some(parent) = target_path.parent() {
                            crate::internal::fs::ensure_dir(parent)
                                .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
                        }
                        let mut buf = Vec::new();
                        reader
                            .read_to_end(&mut buf)
                            .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
                        std::fs::write(&target_path, &buf)
                            .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
                        Ok(true)
                    });
                    if let Err(e) = result {
                        errors.lock().unwrap().push(e.to_string());
                        break;
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle
            .join()
            .map_err(|_| Error::ExtractionFailed("7z extract worker panicked".into()))?;
    }
    if let Some(e) = errors.lock().unwrap().first() {
        return Err(Error::ExtractionFailed(format!("7z extract error: {}", e)));
    }
    Ok(())
}

// ─── Zip extraction via zip crate ────────────────────────────────────

/// Combination of `Read + Seek` usable as a single trait object bound
/// (`dyn Read + Seek` is illegal — only one non-auto trait may be the
/// object's principal trait). Blanket-implemented for every reader.
trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

/// Where a zip archive's bytes come from for parallel extraction. Each worker
/// builds its own `ZipArchive` from this source (the zip crate's `ZipArchive`
/// is not `Sync`), so a file source is reopened per worker and a memory
/// source is a shared slice each worker wraps in its own `Cursor`.
enum ZipSource {
    /// A plain `.zip` file on disk.
    File(PathBuf),
    /// A zip payload embedded at `start` inside `data` (PE/SFX stub prefix).
    Memory { data: Arc<Vec<u8>>, start: usize },
}

impl ZipSource {
    fn open_archive(&self) -> Fallible<zip::ZipArchive<Box<dyn ReadSeek + '_>>> {
        use std::io::Cursor;
        match self {
            ZipSource::File(path) => {
                let file = std::fs::File::open(path)?;
                let reader: Box<dyn ReadSeek> = Box::new(file);
                let archive = zip::ZipArchive::new(reader).map_err(|e| {
                    Error::ExtractionFailed(format!("zip error for {}: {}", path.display(), e))
                })?;
                Ok(archive)
            }
            ZipSource::Memory { data, start } => {
                let payload = data.get(*start..).ok_or_else(|| {
                    Error::ExtractionFailed(format!("embedded zip start {} out of bounds", start))
                })?;
                let reader: Box<dyn ReadSeek> = Box::new(Cursor::new(payload));
                let archive = zip::ZipArchive::new(reader).map_err(|e| {
                    Error::ExtractionFailed(format!("embedded zip parse error: {}", e))
                })?;
                Ok(archive)
            }
        }
    }

    /// Number of entries in the archive (probed once before spawning workers).
    fn len(&self) -> Fallible<usize> {
        Ok(self.open_archive()?.len())
    }

    /// Entry indices ordered by compressed size, largest first. Workers claim
    /// indices in this order so the few huge entries (e.g. Doubao.dll at 304 MB
    /// compressed in a 614 MB SFX) are started first — otherwise a worker can
    /// get stuck on one big file while the others idle on small ones.
    fn entry_order(&self) -> Fallible<Vec<usize>> {
        let mut archive = self.open_archive()?;
        let mut order: Vec<(usize, u64)> = Vec::with_capacity(archive.len());
        for i in 0..archive.len() {
            let size = archive
                .by_index(i)
                .map(|e| e.compressed_size())
                .unwrap_or(0);
            order.push((i, size));
        }
        order.sort_by_key(|b| std::cmp::Reverse(b.1));
        Ok(order.into_iter().map(|(i, _)| i).collect())
    }
}

fn extract_zip(
    src: &Path,
    dest: &Path,
    filter: Option<&[&str]>,
    emitter: &Option<Sender<Event>>,
) -> Fallible<()> {
    let source = Arc::new(ZipSource::File(src.to_path_buf()));
    let total = source.len()?;
    extract_zip_mt(
        source,
        total,
        dest.to_path_buf(),
        owned_filter(filter),
        emitter.clone(),
    )
}

/// Parallel zip extraction core: each worker owns a `ZipArchive` instance
/// (built from `source` — `ZipArchive` is not `Sync`, so it cannot be
/// shared), and entries are claimed dynamically via an atomic counter. ZIP
/// entries carry their own local headers, so unlike solid 7z there is no
/// decode dependency between them — pure per-entry parallelism.
///
/// Progress/filter/path-traversal semantics match the single-threaded
/// version; failures are collected and reported after all workers join.
fn extract_zip_mt(
    source: Arc<ZipSource>,
    total: usize,
    dest: PathBuf,
    filter: Option<Vec<String>>,
    emitter: Option<Sender<Event>>,
) -> Fallible<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    // Largest-first claim order: huge entries start on the first workers
    // instead of leaving them idle while small files finish.
    let order = Arc::new(source.entry_order()?);
    let workers = total
        .min(
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
        )
        .max(1);
    let next = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicUsize::new(0));
    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let filter = Arc::new(filter);

    let handles: Vec<_> = (0..workers)
        .map(|_| {
            let source = source.clone();
            let order = order.clone();
            let next = next.clone();
            let done = done.clone();
            let errors = errors.clone();
            let filter = filter.clone();
            let emitter = emitter.clone();
            let dest = dest.clone();
            std::thread::spawn(move || {
                let filter_ref: Option<Vec<&str>> = filter
                    .as_ref()
                    .as_ref()
                    .map(|f| f.iter().map(|s| s.as_str()).collect());
                let mut archive = match source.open_archive() {
                    Ok(a) => a,
                    Err(e) => {
                        errors.lock().unwrap().push(e.to_string());
                        return;
                    }
                };
                // Cache directories created by this worker so repeat
                // `create_dir_all` calls for the same parent are skipped.
                let mut made_dirs: std::collections::HashSet<PathBuf> =
                    std::collections::HashSet::new();
                loop {
                    let slot = next.fetch_add(1, Ordering::Relaxed);
                    if slot >= order.len() {
                        break;
                    }
                    let i = order[slot];
                    let mut entry = match archive.by_index(i) {
                        Ok(e) => e,
                        Err(e) => {
                            errors.lock().unwrap().push(format!("zip entry {i}: {e}"));
                            break;
                        }
                    };
                    // Windows-created zips (e.g. croc's release) sometimes
                    // use backslash separators; normalize to `/` so directory
                    // detection (`ends_with('/')`), strip_dir and path-join all
                    // see consistent separators. Writing an entry that still
                    // ends in `\` would fail on Windows (path treated as a
                    // directory).
                    let name = entry.name().replace('\\', "/");
                    if name.ends_with('/') {
                        continue;
                    }
                    if let Some(f) = filter_ref.as_deref() {
                        if !f.iter().any(|d| name.starts_with(d)) {
                            continue;
                        }
                    }
                    let n = done.fetch_add(1, Ordering::Relaxed);
                    emit_extract_progress(&emitter, n, Some(total));
                    let target = strip_dir(&name, filter_ref.as_deref()).unwrap_or(name);
                    if Path::new(&target)
                        .components()
                        .any(|c| c == Component::ParentDir)
                    {
                        errors
                            .lock()
                            .unwrap()
                            .push(format!("path traversal: {}", target));
                        break;
                    }
                    let target_path = dest.join(&target);
                    if let Some(parent) = target_path.parent() {
                        if made_dirs.insert(parent.to_path_buf()) {
                            if let Err(e) = crate::internal::fs::ensure_dir(parent) {
                                errors.lock().unwrap().push(e.to_string());
                                break;
                            }
                        }
                    }
                    let mut data = Vec::new();
                    if let Err(e) = entry.read_to_end(&mut data) {
                        errors
                            .lock()
                            .unwrap()
                            .push(format!("zip read {target}: {e}"));
                        break;
                    }
                    if let Err(e) = std::fs::write(&target_path, &data) {
                        errors.lock().unwrap().push(format!(
                            "cannot write {}: {}",
                            target_path.display(),
                            e
                        ));
                        break;
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle
            .join()
            .map_err(|_| Error::ExtractionFailed("zip extract worker panicked".into()))?;
    }

    let errs = errors.lock().unwrap();
    if !errs.is_empty() {
        return Err(Error::ExtractionFailed(format!(
            "zip extraction failed: {}",
            errs.join("; ")
        )));
    }
    Ok(())
}

/// Locate a ZIP archive embedded inside a PE/SFX file (e.g. an installer
/// stub that prepends executable code to a zip payload) by walking back from
/// the end-of-central-directory record.
///
/// Returns the byte offset of the zip payload (where the central directory's
/// relative offsets start) so callers can slice `data[start..]` and feed it to
/// [`zip::ZipArchive`] directly. `None` when no EOCD is found or the computed
/// start is inconsistent (out of bounds / not preceding the EOCD).
///
/// The EOCD's `cd_offset` is relative to the *zip payload start*, not the PE
/// file start, so the slice must begin there or every central-directory /
/// local-header offset would be wrong.
///
/// Layout: `[payload][central directory (cd_size)][EOCD record (22+comment)]`.
/// The EOCD record (comment included) sits entirely *after* the central
/// directory, so the CD starts at `eocd_pos - cd_size` — the comment length
/// does not shift it (unlike a naive `eocd - comment - cd_size`). This
/// mirrors rc-zip's `global_offset` derivation.
fn find_embedded_zip_start(data: &[u8]) -> Option<usize> {
    const EOCD_SIG: &[u8] = b"PK\x05\x06";
    const EOCD_LEN: usize = 22;
    const MAX_COMMENT: usize = 0xFFFF;

    // EOCD sits within the last 64 KiB + comment of the file.
    let tail_start = data.len().saturating_sub(MAX_COMMENT + EOCD_LEN);
    let tail = data.get(tail_start..)?;
    let eocd_rel = tail
        .windows(EOCD_LEN)
        .rposition(|w| w.starts_with(EOCD_SIG))?;
    let eocd = tail_start + eocd_rel;

    let u16_at = |o: usize| -> Option<u16> {
        data.get(eocd + o..eocd + o + 2)?
            .try_into()
            .ok()
            .map(u16::from_le_bytes)
    };
    let u32_at = |o: usize| -> Option<u32> {
        data.get(eocd + o..eocd + o + 4)?
            .try_into()
            .ok()
            .map(u32::from_le_bytes)
    };

    let records = u16_at(10)?;
    let cd_size = u32_at(12)?;
    let cd_offset = u32_at(16)?;

    // ZIP64: sentinel values in the EOCD mean the real 64-bit numbers live in
    // the zip64 EOCD record (PK\x06\x06), found via the locator (PK\x06\x07)
    // which sits directly before the EOCD record.
    let (cd_size, cd_offset, cd_end) =
        if records == 0xFFFF || cd_size == u32::MAX || cd_offset == u32::MAX {
            const LOCATOR_SIG: &[u8] = b"PK\x06\x07";
            const LOCATOR_LEN: usize = 20;
            const Z64_SIG: &[u8] = b"PK\x06\x06";
            let locator = data.get(eocd.checked_sub(LOCATOR_LEN)?..eocd)?;
            if !locator.starts_with(LOCATOR_SIG) {
                return None;
            }
            let z64_offset = u64::from_le_bytes(locator.get(8..16)?.try_into().ok()?) as usize;
            let z64 = data.get(z64_offset..)?;
            if !z64.starts_with(Z64_SIG) {
                return None;
            }
            // zip64 EOCD record: ... records(8) @24, cd_size(8) @40, cd_offset(8) @48.
            // The central directory ends where the zip64 EOCD record begins.
            let cd_size = u64::from_le_bytes(z64.get(40..48)?.try_into().ok()?) as usize;
            let cd_offset = u64::from_le_bytes(z64.get(48..56)?.try_into().ok()?) as usize;
            (cd_size, cd_offset, z64_offset)
        } else {
            (cd_size as usize, cd_offset as usize, eocd)
        };

    // The central directory ends where the EOCD (or zip64 EOCD) record
    // begins; comment bytes are inside that record, not between CD and EOCD.
    let cd_start = cd_end.checked_sub(cd_size)?;
    if cd_start >= cd_end {
        return None;
    }
    let zip_start = cd_start.checked_sub(cd_offset)?;
    Some(zip_start)
}

/// Extract a ZIP archive embedded at `start` inside `data` (PE/SFX stub
/// prefix), with progress + filter semantics matching [`extract_zip`].
/// Parallel: each worker slices its own `Cursor` over the shared payload.
fn extract_zip_embedded(
    data: Arc<Vec<u8>>,
    start: usize,
    dest: &Path,
    filter: Option<&[&str]>,
    emitter: &Option<Sender<Event>>,
) -> Fallible<()> {
    let source = Arc::new(ZipSource::Memory { data, start });
    let total = source.len()?;
    extract_zip_mt(
        source,
        total,
        dest.to_path_buf(),
        owned_filter(filter),
        emitter.clone(),
    )
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
        // lzma-rust2 (the same optimized decoder sevenz-rust2 uses) is ~1.5x
        // faster than lzma-rs 0.3 and streams instead of buffering the whole
        // decompressed archive in memory.
        Some(Compression::Xz) => Box::new(lzma_rust2::XzReader::new(file, false)),
        Some(Compression::Zstd) => Box::new(zstd::Decoder::new(file)?),
        None => Box::new(file),
    };

    if filter.is_some() {
        let mut archive = TarArchive::new(reader);
        let entries = archive.entries()?;
        for (i, entry) in entries.enumerate() {
            let mut entry = entry?;
            let path = entry.path()?.to_string_lossy().to_string();
            if let Some(f) = filter {
                if !f.iter().any(|d| path.starts_with(d)) {
                    continue;
                }
            }
            emit_extract_progress(emitter, i, None);
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
    let mut file_idx = 0usize;

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
        emit_extract_progress(emitter, file_idx, None);
        file_idx += 1;

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

    // Parse progress from stderr. 7-Zip's `-bsp1` writes carriage-return
    // separated progress (`\r69% 6143`) WITHOUT newlines, so
    // `BufRead::lines()` would buffer the entire stream until the process
    // exits and never emit progress. Read up to each `\r` instead.
    if let Some(stderr) = child.stderr.take() {
        use std::io::{BufRead, BufReader};
        let mut reader = BufReader::new(stderr);
        let mut line = Vec::new();
        loop {
            line.clear();
            let n = reader
                .read_until(b'\r', &mut line)
                .map_err(|e| Error::ExtractionFailed(format!("read 7z progress: {}", e)))?;
            if n == 0 {
                break;
            }
            let text = String::from_utf8_lossy(&line);
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

    for (i, result) in installer.extract_files().enumerate() {
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
        emit_extract_progress(emitter, i, None);
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

    // Parse with a generous decompression budget. The crate's default
    // (64 MiB) bounds solid streams via DecodeLimit::Truncate, so large solid
    // installers would fail with a confusing bounds error and fall back to
    // 7z.exe. 512 MiB covers real-world installers (Obsidian's
    // electron-builder payload alone is ~114 MiB decompressed). Upstream 0.4.0
    // reports over-budget files with Error::OutputTooLarge instead of silent
    // truncation.
    let installer = nsis::NsisInstaller::builder(&data)
        .max_decompressed_size(512 * 1024 * 1024)
        .parse()
        .map_err(|e| Error::ExtractionFailed(format!("nsis parse error: {}", e)))?;

    // 7-Zip derives each file's destination from the *instruction stream*:
    // `SetOutPath` instructions track the current install directory and every
    // EW_EXTRACTFILE is written under it. Upstream `files()` already walks the
    // stream (EW_CREATEDIR / $OUTDIR assignment) and `dest_path()` yields the
    // installer's own destination path; `to_install_path()` renders it in
    // 7-Zip's form — `$INSTDIR\` prefix stripped, `$VAR` directories kept
    // literally (`$PLUGINSDIR\app-64.7z`), backslash separators.
    for result in installer.files() {
        let file =
            result.map_err(|e| Error::ExtractionFailed(format!("nsis file error: {}", e)))?;
        let rel = file
            .dest_path()
            .map_err(|e| Error::ExtractionFailed(format!("nsis dest_path error: {}", e)))?
            .to_install_path();
        if rel.is_empty() {
            continue;
        }
        if let Some(f) = filter {
            if !f.iter().any(|d| rel.starts_with(d)) {
                continue;
            }
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
            .map_err(|e| Error::ExtractionFailed(format!("nsis decompress '{}': {}", rel, e)))?;
        std::fs::write(&target_path, &content).map_err(|e| {
            Error::ExtractionFailed(format!("cannot write {}: {}", target_path.display(), e))
        })?;
    }

    // Uninstaller stub: 7-Zip also emits `Uninstall.exe` from the
    // EW_WRITEUNINSTALLER payload (the crate skips the icon/patch prefix and
    // re-attaches the installer's own PE stub to the decompressed overlay).
    for (i, result) in installer.uninstallers().enumerate() {
        let uninstaller = result
            .map_err(|e| Error::ExtractionFailed(format!("nsis uninstaller error: {}", e)))?;
        let rel = uninstaller
            .path()
            .map_err(|e| Error::ExtractionFailed(format!("nsis uninstaller name error: {}", e)))?
            .to_install_path();
        if rel.is_empty() {
            continue;
        }
        if let Some(f) = filter {
            if !f.iter().any(|d| rel.starts_with(d)) {
                continue;
            }
        }
        emit_extract_progress(emitter, i, None);
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

// ─── Helpers ─────────────────────────────────────────────────────────

/// Emit an extraction progress event: a bare `n/total` counter (no filename —
/// users don't care which file is being written), throttled to every
/// `EXTRACT_PROGRESS_THROTTLE` entries plus the final one, so archives with
/// thousands of files (e.g. git's mingw64 tzdata) don't flood the event bus
/// and terminal with per-file redraws. Formats without a known total emit
/// nothing (Start/Done feedback is enough).
fn emit_extract_progress(emitter: &Option<Sender<Event>>, i: usize, total: Option<usize>) {
    const THROTTLE: usize = 50;
    let Some(total) = total else {
        return;
    };
    if !i.is_multiple_of(THROTTLE) && i + 1 != total {
        return;
    }
    if let Some(tx) = emitter {
        let _ = tx.send(Event::PackageExtractProgress(format!(
            "{}/{}",
            i + 1,
            total
        )));
    }
}

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
    fn test_classify_installer() {
        use InstallerKind::*;

        // NSIS (electron-builder etc.) — marker in the stub
        let nsis = b"MZ\x90\x00\x03Nullsoft Install System v3.08";
        assert_eq!(classify_installer(nsis), Nsis);

        // FirstHeader-magic-only NSIS (AltSnap-style)
        let fh = b"MZ\x90\x00\x03\xef\xbe\xad\xdeNullsoftInst";
        assert_eq!(classify_installer(fh), Nsis);

        // Inno Setup — marker in the stub
        let inno = b"MZ\x90\x00\x03Inno Setup Setup Data (5.8.3)";
        assert_eq!(classify_installer(inno), Inno);

        // Plain PE without installer markers
        let sfx = b"MZ\x90\x00\x03\x00\x00\x00\x00\x00";
        assert_eq!(classify_installer(sfx), None);

        // Non-PE data
        assert_eq!(classify_installer(b"not a PE"), None);
        assert_eq!(classify_installer(b"7z\xbc\xaf\x27\x1c random data"), None);

        // Marker beyond the 1 MiB scan limit is ignored (payload area)
        let mut deep = Vec::from(b"MZ\x90\x00\x03".as_slice());
        deep.resize(INSTALLER_SIGNATURE_SCAN_LIMIT + 64, 0);
        deep[INSTALLER_SIGNATURE_SCAN_LIMIT + 10..][..NSIS_MARKER.len()]
            .copy_from_slice(NSIS_MARKER);
        assert_eq!(classify_installer(&deep), None);

        // ...and the FirstHeader magic is likewise bounded by the scan limit
        let mut deep2 = Vec::from(b"MZ\x90\x00\x03".as_slice());
        deep2.resize(INSTALLER_SIGNATURE_SCAN_LIMIT + 64, 0);
        deep2[INSTALLER_SIGNATURE_SCAN_LIMIT + 10..][..NSIS_FIRST_HEADER_MAGIC.len()]
            .copy_from_slice(NSIS_FIRST_HEADER_MAGIC);
        assert_eq!(classify_installer(&deep2), None);
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
    fn test_nsis_install_path_render() {
        // Upstream 0.4.0 owns the 7-Zip-style destination semantics
        // (`ExtractedFile::dest_path()` + `NsisString::to_install_path()`),
        // replacing our vendored `nsis_dest_path`. This pins the rendering
        // contract we depend on: a leading `$INSTDIR\` is stripped (the
        // install root IS the extraction root) and `$VAR` directories are
        // kept literally (`$PLUGINSDIR\app-64.7z`, which pre_install scripts
        // reference as-is).
        use nsis::strings::{NsisString, StringSegment};
        let lit = |s: &str| NsisString {
            segments: vec![StringSegment::Literal(s.into())],
        };
        assert_eq!(
            lit("$INSTDIR\\docs\\payload.txt").to_install_path(),
            "docs\\payload.txt"
        );
        assert_eq!(
            lit("$INSTDIR\\a\\sub\\x.bin").to_install_path(),
            "a\\sub\\x.bin"
        );
        // No $INSTDIR prefix → rendered verbatim (relative SetOutPath).
        assert_eq!(lit("Lang\\de_DE.ini").to_install_path(), "Lang\\de_DE.ini");
        // electron-builder: name carries its own literal $VAR directory.
        assert_eq!(
            lit("$PLUGINSDIR\\app-64.7z").to_install_path(),
            "$PLUGINSDIR\\app-64.7z"
        );
        assert_eq!(lit("config.ini").to_install_path(), "config.ini");
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

    /// True when `dir` contains at least one file with a non-zero size.
    fn has_nonempty_file(dir: &std::path::Path) -> bool {
        fn walk(dir: &std::path::Path) -> bool {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return false;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() && walk(&p) {
                    return true;
                }
                if p.is_file() && std::fs::metadata(&p).map(|m| m.len() > 0).unwrap_or(false) {
                    return true;
                }
            }
            false
        }
        walk(dir)
    }

    /// Extract a large solid NSIS installer with the raised decompression
    /// budget and verify the payload survives (no silent 64 MiB truncation).
    /// Point `NSIS_SAMPLE` at a solid installer whose decompressed size
    /// exceeds the crate default budget (e.g. makensis `SetCompressor /SOLID`
    /// with a >64 MiB payload) to run manually.
    #[test]
    #[ignore = "requires a large solid NSIS sample (NSIS_SAMPLE env)"]
    fn extract_nsis_large_solid_corpus() {
        let Ok(path) = std::env::var("NSIS_SAMPLE") else {
            return;
        };
        let dest = crate::test_utils::tmpdir("nsis_large_solid");
        extract_nsis(std::path::Path::new(&path), &dest, None, &None).unwrap();
        assert!(
            has_nonempty_file(&dest),
            "expected a non-empty payload in extraction of {path}"
        );
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

    /// Windows-created zips sometimes use backslash separators in entry
    /// names (e.g. croc's release archive: `src\\codephrase\\`). The
    /// extractor must normalize `\\` to `/` so directory entries are
    /// recognised and files land under the same joined paths — without the
    /// fix a directory entry ending in `\\` is treated as a file and written
    /// to an illegal trailing-backslash path (os error 3 on Windows).
    #[test]
    fn test_extract_zip_backslash_separators() {
        use zip::write::FileOptions;
        use zip::CompressionMethod;
        use zip::ZipWriter;

        let dir = tmpdir("extract_zip_backslash");
        let archive_path = dir.join("backslash.zip");
        let file = std::fs::File::create(&archive_path).unwrap();
        let mut zip = ZipWriter::new(file);
        let opts: FileOptions<'_, ()> =
            FileOptions::default().compression_method(CompressionMethod::Stored);
        // Entry names use backslashes exactly as croc's release zip does.
        zip.add_directory("src\\codephrase\\", opts).unwrap();
        zip.start_file("src\\codephrase\\words.txt", opts).unwrap();
        zip.write_all(b"word list").unwrap();
        zip.start_file("croc.exe", opts).unwrap();
        zip.write_all(b"PE bytes").unwrap();
        zip.finish().unwrap();

        let dest = dir.join("out");
        std::fs::create_dir_all(&dest).unwrap();

        extract(&archive_path, &dest, None, None, false, &None).unwrap();

        // Files must be extracted (backslashes treated as separators).
        assert!(dest.join("src/codephrase/words.txt").exists());
        assert_eq!(
            std::fs::read_to_string(dest.join("src/codephrase/words.txt")).unwrap(),
            "word list"
        );
        assert!(dest.join("croc.exe").exists());
    }

    /// `find_embedded_zip_start` locates a ZIP payload behind a PE/SFX stub:
    /// the EOCD's `cd_offset` is relative to the *zip payload start*, so the
    /// computed offset must point at the zip (not the stub), and the
    /// extracted slice must parse via `ZipArchive`.
    #[test]
    fn find_embedded_zip_start_locates_payload_behind_stub() {
        use std::io::{Cursor, Read};

        // Build a real zip, then prepend a fake PE stub (MZ + junk + NSIS
        // marker to force the installer path away from plain zip dispatch).
        let mut zip_bytes = Vec::new();
        {
            let cursor = Cursor::new(&mut zip_bytes);
            let mut zip = zip::ZipWriter::new(cursor);
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("payload.txt", opts).unwrap();
            zip.write_all(b"stub-hidden payload").unwrap();
            zip.finish().unwrap();
        }

        // Fake PE stub: "MZ" header + junk bytes padded to a fixed stub size.
        let mut stub = vec![0u8; 0x2000];
        stub[..4].copy_from_slice(b"MZ\x90\x00");
        stub[4..28].copy_from_slice(b"embedded installer stub\x00");
        let mut pe = stub;
        pe.extend_from_slice(&zip_bytes);

        let start = find_embedded_zip_start(&pe).expect("EOCD found behind stub");
        assert!(
            start >= 0x2000,
            "payload starts after the stub, got {start}"
        );
        // zip_start is where the EOCD's cd_offset is relative to; the first
        // local header may sit a few bytes later (e.g. 22-byte data-descriptor
        // / extra prefix), so don't assert PK at `start` itself.

        // The slice at `start` must parse as a zip.
        let mut archive = zip::ZipArchive::new(Cursor::new(&pe[start..])).unwrap();
        assert_eq!(archive.len(), 1);
        let mut entry = archive.by_index(0).unwrap();
        let mut out = String::new();
        entry.read_to_string(&mut out).unwrap();
        assert_eq!(out, "stub-hidden payload");
    }

    /// No EOCD → no embedded zip (a plain PE/EXE returns None, so the caller
    /// falls through to 7z.exe instead of mis-parsing).
    #[test]
    fn find_embedded_zip_start_none_for_plain_pe() {
        let mut data = vec![0u8; 0x1000];
        data[..4].copy_from_slice(b"MZ\x90\x00");
        assert_eq!(find_embedded_zip_start(&data), None);
    }

    /// End-to-end on a real PE-prefixed ZIP installer (Doubao's download:
    /// `Doubao_installer_*.exe`, cached as `doubao#<ver>#<hash>.7z`). Verifies
    /// the EOCD-derived start parses via the zip crate and extraction writes
    /// files. Point `DOUBAO_SAMPLE` at the cache file to run manually.
    #[test]
    #[ignore = "requires a local Doubao SFX sample (DOUBAO_SAMPLE env)"]
    fn extract_embedded_zip_doubao_corpus() {
        let Ok(path) = std::env::var("DOUBAO_SAMPLE") else {
            return;
        };
        let data = Arc::new(std::fs::read(&path).unwrap());
        assert!(data.starts_with(b"MZ"), "sample should be a PE file");

        let start =
            find_embedded_zip_start(&data).unwrap_or_else(|| panic!("no EOCD found in {path}"));
        let dest = tmpdir("doubao_sfx");
        extract_zip_embedded(data, start, &dest, None, &None).unwrap();
        assert!(
            has_nonempty_file(&dest),
            "expected a non-empty payload in extraction of {path}"
        );
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

    /// TEMP benchmark: 10000-file zip extraction with no progress events vs
    /// throttled progress events (sink does a real `\r` print + flush like
    /// scoop_handler). Measures whether the progress bar slows extraction.
    #[test]
    #[ignore = "temporary progress-overhead benchmark"]
    fn bench_progress_overhead() {
        use std::io::Write;
        use std::time::Instant;

        let dir = tmpdir("bench_progress");
        let zip_path = dir.join("many.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zw = zip::ZipWriter::new(file);
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for i in 0..10_000 {
                zw.start_file(format!("f{i:05}.txt"), opts).unwrap();
                zw.write_all(b"x").unwrap();
            }
            zw.finish().unwrap();
        }

        let run = |with_progress: bool| {
            let dest = dir.join(if with_progress { "out_p" } else { "out_n" });
            std::fs::create_dir_all(&dest).unwrap();
            let tx = if with_progress {
                let (tx, rx) = flume::unbounded();
                std::thread::spawn(move || {
                    while let Ok(ev) = rx.recv() {
                        if let Event::PackageExtractProgress(ctx) = ev {
                            print!("\r\x1b[2K  {ctx}");
                            let _ = std::io::stdout().flush();
                        }
                    }
                });
                Some(tx)
            } else {
                None
            };
            let t = Instant::now();
            extract_zip(&zip_path, &dest, None, &tx).unwrap();
            let elapsed = t.elapsed();
            drop(tx);
            elapsed
        };

        let t_none = run(false);
        let t_prog = run(true);
        println!("no-progress: {t_none:?}   throttled-progress: {t_prog:?}");
        assert!(
            t_prog.as_secs_f64() < t_none.as_secs_f64() * 5.0 + 0.5,
            "progress overhead too large: {t_none:?} vs {t_prog:?}"
        );
    }

    #[test]
    #[ignore = "temporary git 7z benchmark"]
    fn bench_git_7z_extract() {
        use std::time::Instant;
        let Ok(path) = std::env::var("GIT7Z_BENCH") else {
            return;
        };

        // decode-only: read all data, discard (no disk writes). The git
        // package is a 7z SFX (MZ header), so locate the embedded 7z like
        // extract_pseudo_7z does.
        let t = Instant::now();
        {
            let file_data = std::fs::read(&path).unwrap();
            const MAGIC: &[u8] = &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];
            let pos = file_data
                .windows(MAGIC.len())
                .position(|w| w == MAGIC)
                .expect("embedded 7z magic");
            let mut reader = sevenz_rust2::ArchiveReader::new(
                std::io::Cursor::new(&file_data[pos..]),
                sevenz_rust2::Password::empty(),
            )
            .expect("ArchiveReader::new failed");
            reader
                .for_each_entries(
                    &mut |_e: &sevenz_rust2::ArchiveEntry, data: &mut dyn std::io::Read| {
                        let mut b = Vec::new();
                        data.read_to_end(&mut b)
                            .map_err(|e| sevenz_rust2::Error::Other(e.to_string().into()))?;
                        Ok(true)
                    },
                )
                .unwrap();
        }
        println!("decode-only: {:?}", t.elapsed());

        // Block layout + parallel decode ceiling (max block time).
        let file_data = std::fs::read(&path).unwrap();
        const MAGIC: &[u8] = &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];
        let pos = file_data
            .windows(MAGIC.len())
            .position(|w| w == MAGIC)
            .expect("embedded 7z magic");
        let archive = sevenz_rust2::Archive::read(
            &mut std::io::Cursor::new(&file_data[pos..]),
            &sevenz_rust2::Password::empty(),
        )
        .expect("archive read");
        let mut total_unpack = 0u64;
        for (i, b) in archive.blocks.iter().enumerate() {
            let sz = b.get_unpack_size();
            total_unpack += sz;
            println!("block {i}: unpack={} MiB", sz / (1024 * 1024));
        }
        println!("total unpack: {} MiB", total_unpack / (1024 * 1024));

        let t = Instant::now();
        {
            use std::sync::atomic::{AtomicUsize, Ordering};
            use std::sync::Arc;
            let archive = Arc::new(archive);
            let password = Arc::new(sevenz_rust2::Password::empty());
            let next = Arc::new(AtomicUsize::new(0));
            let handles: Vec<_> = (0..archive.blocks.len().min(16))
                .map(|_| {
                    let archive = archive.clone();
                    let password = password.clone();
                    let next = next.clone();
                    let file_data = file_data.clone();
                    std::thread::spawn(move || loop {
                        let bi = next.fetch_add(1, Ordering::Relaxed);
                        if bi >= archive.blocks.len() {
                            break;
                        }
                        let mut src = std::io::Cursor::new(&file_data[pos..]);
                        let decoder =
                            sevenz_rust2::BlockDecoder::new(1, bi, &archive, &password, &mut src);
                        decoder
                            .for_each_entries(&mut |_e, r| {
                                let mut b = Vec::new();
                                r.read_to_end(&mut b).map_err(|e| {
                                    sevenz_rust2::Error::Other(e.to_string().into())
                                })?;
                                Ok(true)
                            })
                            .expect("block decode");
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }
            println!(
                "parallel decode-only ({} blocks): {:?}",
                archive.blocks.len(),
                t.elapsed()
            );
        }

        let dest = tmpdir("git7z_hok");
        let t = Instant::now();
        extract_pseudo_7z(std::path::Path::new(&path), &dest, None, &None).unwrap();
        println!("hok builtin extract git pkg: {:?}", t.elapsed());
    }
}
