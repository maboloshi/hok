//! Scoop download cache file representation.
//!
//! Cache files follow the naming convention `app#version#filenamified_url`.
//! The [`CacheFile`] struct parses and validates this pattern, providing
//! typed accessors for `package_name()`, `version()`, and `path()`.

use std::path::{Path, PathBuf};

use crate::constant::REGEX_CACHE_FILE;
use crate::error::{Error, Fallible};

/// Scoop cache file representation
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheFile {
    path: PathBuf,
    /// Length in bytes at enumeration time. Cached because the file is often
    /// removed straight after being listed (`remove`), and upstream likewise
    /// measures `$file.Length` while the file still exists
    /// (`scoop-cache.ps1:19,47`). `0` when the metadata could not be read.
    size: u64,
}

impl CacheFile {
    pub fn from(path: PathBuf) -> Fallible<CacheFile> {
        // Non-UTF-8 names cannot match the `app#version#url` cache naming
        // convention — reject them as invalid cache files.
        let Some(text) = path.file_name().and_then(|n| n.to_str()) else {
            return Err(Error::InvalidCacheFile { path });
        };
        match REGEX_CACHE_FILE.is_match(text) {
            false => Err(Error::InvalidCacheFile { path }),
            true => {
                let size = path.metadata().map(|m| m.len()).unwrap_or(0);
                Ok(CacheFile { path, size })
            }
        }
    }

    /// Get path of this cache file
    #[inline]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get file name of this cache file
    #[inline]
    pub fn file_name(&self) -> &str {
        self.path.file_name().unwrap().to_str().unwrap()
    }

    /// Get package name of this cache file
    #[inline]
    pub fn package_name(&self) -> &str {
        self.file_name().split_once('#').map(|s| s.0).unwrap()
    }

    /// Get version of this cache file
    #[inline]
    pub fn version(&self) -> &str {
        self.file_name().splitn(3, '#').collect::<Vec<_>>()[1]
    }

    /// Get the length of this cache file in bytes, as measured when it was
    /// enumerated.
    #[inline]
    pub fn size(&self) -> u64 {
        self.size
    }
}

// ─── Session operations ────────────────────────────────────────────────────

/// Whether `apps` selects every cache entry.
///
/// An empty selection (the bare `hok cache` default) or an explicit `*` means
/// "every file in the cache directory", upstream's `Get-ChildItem $cachedir`
/// branch (`scoop-cache.ps1:42`).
fn is_wildcard(apps: &[String]) -> bool {
    apps.is_empty() || apps.iter().any(|a| a == "*")
}

/// Whether `file` belongs to any app in `apps`.
///
/// Upstream anchors the query at the start of the cache file name
/// (`Where-Object Name -Value "^$app#" -Match`, `scoop-cache.ps1:28,45`).
/// Comparing the parsed package name for case-insensitive equality reproduces
/// that anchor without inheriting upstream's regex metacharacters, so
/// `cache rm zip` no longer also matches `7zip`.
fn matches_apps(file: &CacheFile, apps: &[String]) -> bool {
    apps.iter()
        .any(|a| file.package_name().eq_ignore_ascii_case(a))
}

/// Remove one cache entry (file or directory), tolerating an entry that is
/// already gone and reporting any other failure through the session output
/// channel so a single locked file never aborts the batch.
///
/// Returns whether this call removed it: `false` when it had already vanished
/// (a concurrent download finishing, say) or could not be removed, so a caller
/// that reports a summary ([`remove_incomplete`]) counts only the bytes it
/// actually freed. Every other caller ignores the result.
fn remove_entry(session: &crate::Session, path: &Path) -> bool {
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match result {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => {
            session.output().error(hok_i18n::t!(
                "cmd.failed_remove_cache",
                path = path.display(),
                error = e
            ));
            false
        }
    }
}

/// Get a list of downloaded cache files.
///
/// `apps` filters the listing to those package names; an empty slice (or one
/// containing `*`) lists every cache file.
///
/// # Returns
///
/// A list of downloaded cache files.
///
/// # Errors
///
/// I/O errors will be returned if the cache directory is not readable.
pub fn list(session: &crate::Session, apps: &[String]) -> Fallible<Vec<CacheFile>> {
    let list_all = is_wildcard(apps);
    let config = session.config();
    let cache_dir = config.cache_path();
    let mut files = vec![];

    match cache_dir.read_dir() {
        Err(err) => {
            tracing::debug!("failed to read cache dir (err: {})", err);
        }
        Ok(entires) => {
            files = entires
                .filter_map(|de| {
                    if let Ok(entry) = de {
                        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
                        if is_file
                            && let Ok(item) = CacheFile::from(entry.path())
                            && (list_all || matches_apps(&item, apps))
                        {
                            return Some(item);
                        }
                    }
                    None
                })
                .collect::<Vec<_>>();
        }
    }

    sort_listing(&mut files);

    Ok(files)
}

/// Order a listing by package name (case-insensitively) then version — the
/// order upstream's `Get-ChildItem` presents, and the one that makes a listing
/// reproducible: `read_dir` returns an arbitrary order, so without this the
/// rows (and therefore a rendered table's column widths) would differ between
/// filesystems and runs.
fn sort_listing(files: &mut [CacheFile]) {
    files.sort_by(|a, b| {
        a.package_name()
            .to_lowercase()
            .cmp(&b.package_name().to_lowercase())
            .then_with(|| a.version().cmp(b.version()))
            .then_with(|| a.file_name().cmp(b.file_name()))
    });
}

/// Remove cache files for `apps`.
///
/// An empty selection (or one containing `*`) clears the whole cache
/// directory, exactly as upstream's `Get-ChildItem $cachedir` branch does;
/// otherwise only the selected apps' files are removed, together with each
/// app's aria2 sidecar `<cachedir>/<app>.txt` (`scoop-cache.ps1:53-55`).
/// Selection is anchored to the app name, so `zip` never matches `7zip`.
///
/// Returns the removed cache entries, in the same order [`list`] presents
/// them, so the caller can report each removal (`Removing <name>...`) and the
/// `Deleted: N files, <size>` summary the way upstream does
/// (`scoop-cache.ps1:49-58`). The wildcard branch also deletes entries that do
/// not follow the `app#version#url` naming (aria2 sidecars, stray files,
/// directories) but only the recognizable cache files are returned — those are
/// the ones the summary accounts for.
///
/// Files that are already gone are silently skipped. Files that fail to
/// remove (e.g. locked by a running process on Windows) are reported through
/// the session output channel and skipped, so a single locked file never
/// aborts the batch.
///
/// # Errors
///
/// I/O errors will be returned if the cache directory is not readable.
pub fn remove(session: &crate::Session, apps: &[String]) -> Fallible<Vec<CacheFile>> {
    let cache_dir = session.config().cache_path().to_path_buf();

    if is_wildcard(apps) {
        if !cache_dir.exists() {
            return Ok(Vec::new());
        }
        let mut removed = Vec::new();
        for entry in cache_dir.read_dir()? {
            let path = entry?.path();
            // Enumerate through the same constructor `list` uses, so the
            // caller's per-file output matches what `cache` (show) would list.
            if let Ok(file) = CacheFile::from(path.clone()) {
                removed.push(file);
            }
            remove_entry(session, &path);
        }
        sort_listing(&mut removed);
        return Ok(removed);
    }

    let mut sidecars: Vec<String> = vec![];
    let mut removed = Vec::new();
    for f in list(session, apps)? {
        let app = f.package_name().to_owned();
        if !sidecars.iter().any(|s| s.eq_ignore_ascii_case(&app)) {
            sidecars.push(app);
        }
        remove_entry(session, f.path());
        removed.push(f);
    }

    // aria2 drops a `<app>.txt` control file beside the downloads; upstream
    // removes it together with the matching cache files.
    for app in sidecars {
        remove_entry(session, &cache_dir.join(format!("{app}.txt")));
    }

    Ok(removed)
}

/// Remove one app's outdated cache files, keeping `keep_version`.
///
/// Upstream `cleanup -k` (`Remove-Item "$cachedir\$app#*" -Exclude
/// "$app#$current_version#*"`, `scoop-cleanup.ps1:32-33`): the currently
/// installed version stays cached so a reinstall does not re-download it. The
/// aria2 sidecar is deliberately left alone — upstream's `cleanup` removes it
/// only through `cache rm`.
///
/// Returns the removed entries, in [`list`] order, so the caller can report
/// what `-k` freed: upstream prints nothing here, which made a run that only
/// dropped cache look like a no-op.
///
/// # Errors
///
/// I/O errors will be returned if the cache directory is not readable.
pub fn remove_outdated(
    session: &crate::Session,
    app: &str,
    keep_version: &str,
) -> Fallible<Vec<CacheFile>> {
    remove_app_cache(session, app, Some(keep_version))
}

/// Remove *every* cached download of one app, whatever version it belongs to.
///
/// This is upstream's `Remove-Item "$cachedir\$app#*" -Exclude
/// "$app#$current_version#*"` reached with no current version to exclude: the
/// exclude pattern degenerates to `"$app##*"`, matches nothing, and the app's
/// whole cache goes (`scoop-cleanup.ps1:32-33`). `cleanup -k` uses it for an
/// app whose current version cannot be resolved, so a broken install does not
/// pin its downloads forever.
///
/// Like [`remove_outdated`], the aria2 sidecar is left alone.
///
/// # Errors
///
/// I/O errors will be returned if the cache directory is not readable.
pub fn remove_all_for_app(session: &crate::Session, app: &str) -> Fallible<Vec<CacheFile>> {
    remove_app_cache(session, app, None)
}

/// Shared body of [`remove_outdated`] / [`remove_all_for_app`]: remove `app`'s
/// cache files, skipping the versions in `keep` (`None` keeps nothing).
fn remove_app_cache(
    session: &crate::Session,
    app: &str,
    keep: Option<&str>,
) -> Fallible<Vec<CacheFile>> {
    let mut removed = Vec::new();
    for f in list(session, &[app.to_owned()])? {
        if keep == Some(f.version()) {
            continue;
        }
        remove_entry(session, f.path());
        removed.push(f);
    }
    Ok(removed)
}

/// What [`remove_incomplete`] swept away: partial downloads that no finished
/// cache entry owns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IncompleteSweep {
    /// `*.download` files and `<cache name>.parts` directories removed.
    pub entries: usize,
    /// Bytes they held, measured before the removal.
    pub bytes: u64,
}

/// Remove leftover partial downloads: `*.download` files and the `.parts`
/// directories a fragmented download leaves behind.
///
/// Upstream runs the `*.download` half once after `cleanup -k` has swept every
/// app (`scoop-cleanup.ps1:79-81`). The `.parts` half is hok's, and it is the
/// half that accumulates: an interrupted fragmented download leaves
/// `<cache name>.parts` holding chunks and nothing else — no cache entry exists
/// yet — so neither [`remove_outdated`] (files only, installed apps only) nor
/// upstream's `Remove-Item` (non-recursive) can reach it.
///
/// A `.parts` directory belongs to a *running* download only until its writer
/// releases it: the recursive removal then fails against that writer's file
/// lock and is reported through the session output channel, exactly like any
/// other locked entry, so a concurrent download is not silently gutted.
///
/// Returns what it swept, so the frontend can report it. Byte sizes are summed
/// recursively — a directory's own `metadata().len()` is 0.
///
/// # Errors
///
/// I/O errors will be returned if the cache directory is not readable.
pub fn remove_incomplete(session: &crate::Session) -> Fallible<IncompleteSweep> {
    let cache_dir = session.config().cache_path().to_path_buf();
    let Ok(entries) = cache_dir.read_dir() else {
        return Ok(IncompleteSweep::default());
    };
    let mut sweep = IncompleteSweep::default();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_download = path.extension().is_some_and(|ext| ext == "download");
        let is_parts = entry.file_type().is_ok_and(|t| t.is_dir()) && is_parts_dir(&path);
        if !(is_download || is_parts) {
            continue;
        }
        let bytes = path_len(&path);
        if remove_entry(session, &path) {
            sweep.entries += 1;
            sweep.bytes += bytes;
        }
    }
    Ok(sweep)
}

/// Whether `path` is a fragmented download's parts directory: a directory whose
/// name is a cache name plus the `.parts` suffix `download.rs` builds it with.
///
/// The cache-name test is the same one every other cache path uses, so an
/// unrelated directory that merely ends in `.parts` is left alone.
fn is_parts_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".parts") && REGEX_CACHE_FILE.is_match(name))
}

/// Total bytes under `path` — the file itself, or everything below a directory —
/// and `0` when it cannot be measured.
fn path_len(path: &Path) -> u64 {
    let Ok(meta) = path.metadata() else {
        return 0;
    };
    if meta.is_file() {
        return meta.len();
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries.flatten().map(|entry| path_len(&entry.path())).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file(name: &str) -> CacheFile {
        CacheFile::from(PathBuf::from(name)).expect("valid cache file name")
    }

    fn identity(f: &CacheFile) -> String {
        format!("{}#{}", f.package_name(), f.version())
    }

    /// A listing is ordered by package name (case-insensitively) then version,
    /// regardless of the order the directory returned.
    #[test]
    fn listing_is_ordered_by_name_then_version() {
        let mut files = vec![
            file("zstd#1.5.7#a.zip"),
            file("Brave#1.94.121#b.zip"),
            file("7zip#26.03#c.msi"),
            file("croc#11.5.0#d.zip"),
            file("croc#11.4.0#e.zip"),
        ];

        sort_listing(&mut files);

        let order = files.iter().map(identity).collect::<Vec<_>>();
        assert_eq!(
            order,
            [
                "7zip#26.03",
                "Brave#1.94.121",
                "croc#11.4.0",
                "croc#11.5.0",
                "zstd#1.5.7"
            ]
        );
    }

    /// Two files of the same package and version (different URLs) still get a
    /// deterministic order.
    #[test]
    fn listing_is_stable_for_equal_name_and_version() {
        let mut files = vec![file("croc#1.0#z.zip"), file("croc#1.0#a.zip")];
        sort_listing(&mut files);

        let order = files
            .iter()
            .map(|f| f.file_name().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(order, ["croc#1.0#a.zip", "croc#1.0#z.zip"]);
    }

    /// App selection is anchored to the whole package name, so `zip` must not
    /// drag in `7zip`/`unzip` (upstream's `^$app#` anchor), and it is
    /// case-insensitive like PowerShell's `-Match`.
    #[test]
    fn app_selection_is_anchored_and_case_insensitive() {
        let zip = file("zip#1.0#a.zip");
        let seven = file("7zip#26.03#b.7z");
        let unzip = file("unzip#6.0#c.zip");

        assert!(matches_apps(&zip, &["zip".to_owned()]));
        assert!(matches_apps(&zip, &["ZIP".to_owned()]));
        assert!(!matches_apps(&seven, &["zip".to_owned()]));
        assert!(!matches_apps(&unzip, &["zip".to_owned()]));
        // Any-of semantics matches upstream's `(a|b)` alternation.
        assert!(matches_apps(&seven, &["zip".to_owned(), "7zip".to_owned()]));
    }

    /// An empty selection or a literal `*` means "the whole cache"; anything
    /// else is a per-app selection.
    #[test]
    fn empty_and_star_selections_are_wildcards() {
        assert!(is_wildcard(&[]));
        assert!(is_wildcard(&["*".to_owned()]));
        assert!(!is_wildcard(&["zip".to_owned()]));
    }

    /// A cache name carries whatever the bucket's `version` says — `_`, `+` and
    /// `@` included (`1.9.11_2007`, `1.4.3-canary.1+29d9638da`,
    /// `dingtalk@7.0.10.1026`) — while a name that is not a cache entry stays
    /// rejected.
    #[test]
    fn cache_names_carry_any_version_spelling() {
        for name in [
            "authpass#1.9.11_2007#a.exe",
            "7zip#21.03+zs#b.7z",
            "dingtalk@7.0.10.1026#7.0.10.1026#c.exe",
            "rstudio#2026.10.0+155-daily#d.zip",
            // The URL segment is free-form: upstream sanitizes it into `#`s.
            "zip#1.0#https##example.com#e.zip",
        ] {
            assert!(
                CacheFile::from(PathBuf::from(name)).is_ok(),
                "must parse: {name}"
            );
        }

        for name in ["readme.txt", "app#1.0", "app.txt"] {
            assert!(
                CacheFile::from(PathBuf::from(name)).is_err(),
                "must be rejected: {name}"
            );
        }
    }

    /// Cache operations against a temporary session: `remove_outdated` keeps
    /// the current version and drops the rest, `remove` is anchored to the
    /// selected app and takes the aria2 sidecar with it, and
    /// `remove_incomplete` clears the `*.download` leftovers (its `.parts` half
    /// has its own test below).
    #[test]
    fn session_operations_keep_current_version_and_sidecars_in_sync() {
        let root = crate::test_utils::tmpdir("cache_ops");
        let session = crate::test_utils::test_session(&root);
        let cache_dir = root.join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();
        for name in [
            "zip#1.0#a.zip",
            "zip#2.0#b.zip",
            "7zip#26.0#c.7z",
            "zip.txt",
            "7zip.txt",
            "partial.download",
        ] {
            std::fs::write(cache_dir.join(name), b"x").unwrap();
        }

        // `cleanup -k`: keep zip 2.0, drop zip 1.0, leave everything else —
        // and report what went, so the CLI can turn `-k` into a visible line.
        let removed = remove_outdated(&session, "zip", "2.0").unwrap();
        assert_eq!(
            removed.iter().map(CacheFile::file_name).collect::<Vec<_>>(),
            ["zip#1.0#a.zip"]
        );
        assert!(!cache_dir.join("zip#1.0#a.zip").exists());
        assert!(cache_dir.join("zip#2.0#b.zip").exists());
        assert!(cache_dir.join("7zip#26.0#c.7z").exists());
        assert!(cache_dir.join("zip.txt").exists(), "sidecar untouched");

        // `cache rm zip`: only zip's file and sidecar go, and the removed
        // cache file is reported back (upstream's per-file `Removing ...`).
        let removed = remove(&session, &["zip".to_owned()]).unwrap();
        assert_eq!(
            removed.iter().map(CacheFile::file_name).collect::<Vec<_>>(),
            ["zip#2.0#b.zip"]
        );
        assert_eq!(removed[0].size(), 1);
        assert!(!cache_dir.join("zip#2.0#b.zip").exists());
        assert!(!cache_dir.join("zip.txt").exists(), "sidecar removed");
        assert!(cache_dir.join("7zip#26.0#c.7z").exists());
        assert!(cache_dir.join("7zip.txt").exists());

        let sweep = remove_incomplete(&session).unwrap();
        assert_eq!(
            sweep,
            IncompleteSweep {
                entries: 1,
                bytes: 1
            }
        );
        assert!(!cache_dir.join("partial.download").exists());
        assert!(cache_dir.join("7zip#26.0#c.7z").exists());
    }

    /// `remove_incomplete` also sweeps the `.parts` directory an interrupted
    /// fragmented download leaves behind — chunks only, with no cache entry for
    /// anything else to find — while a directory that merely ends in `.parts`
    /// and the finished cache files stay untouched.
    #[test]
    fn remove_incomplete_sweeps_part_dirs() {
        let root = crate::test_utils::tmpdir("cache_incomplete_parts");
        let session = crate::test_utils::test_session(&root);
        let cache_dir = root.join("cache");
        let parts = cache_dir.join("authpass#1.9.11_2007#aaaaaaa.exe.parts");
        std::fs::create_dir_all(&parts).unwrap();
        std::fs::write(parts.join("part.0"), vec![0u8; 10]).unwrap();
        std::fs::write(parts.join("part.1"), vec![0u8; 5]).unwrap();
        std::fs::write(cache_dir.join("partial.download"), vec![0u8; 3]).unwrap();
        std::fs::write(cache_dir.join("authpass#1.10#ccccccc.exe"), b"x").unwrap();
        // A directory that is not cache-shaped is the user's, not ours.
        let theirs = cache_dir.join("notes.parts");
        std::fs::create_dir_all(&theirs).unwrap();
        std::fs::write(theirs.join("keep.txt"), b"x").unwrap();

        let sweep = remove_incomplete(&session).unwrap();

        assert_eq!(
            sweep,
            IncompleteSweep {
                entries: 2,
                bytes: 18
            },
            "part dir (10 + 5) + the .download leftover (3)"
        );
        assert!(!parts.exists(), "the parts directory is gone");
        assert!(!cache_dir.join("partial.download").exists());
        assert!(cache_dir.join("authpass#1.10#ccccccc.exe").exists());
        assert!(theirs.join("keep.txt").exists(), "not a cache name: kept");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `cleanup -k` and `cache rm <app>` must reach an odd-version name as well:
    /// they used to miss exactly these files, which only `cache rm *` (the
    /// branch that walks the directory itself) could ever delete.
    #[test]
    fn odd_version_names_are_removable_by_app() {
        let root = crate::test_utils::tmpdir("cache_odd_versions");
        let session = crate::test_utils::test_session(&root);
        let cache_dir = root.join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();
        for name in [
            "authpass#1.9.11_2007#a.exe",
            "authpass#1.9.11_2007+next#b.exe",
            "authpass#1.10#c.exe",
            "authpass.txt",
        ] {
            std::fs::write(cache_dir.join(name), b"x").unwrap();
        }

        // `cleanup -k`: the current version survives, every spelling of the old
        // one goes, in `list` order.
        let removed = remove_outdated(&session, "authpass", "1.10").unwrap();
        assert_eq!(
            removed.iter().map(CacheFile::file_name).collect::<Vec<_>>(),
            [
                "authpass#1.9.11_2007#a.exe",
                "authpass#1.9.11_2007+next#b.exe"
            ]
        );
        assert!(cache_dir.join("authpass#1.10#c.exe").exists());
        assert!(cache_dir.join("authpass.txt").exists(), "sidecar untouched");

        // A broken install has no version to keep, so `-k` drops all of them.
        let removed = remove_all_for_app(&session, "authpass").unwrap();
        assert_eq!(
            removed.iter().map(CacheFile::file_name).collect::<Vec<_>>(),
            ["authpass#1.10#c.exe"]
        );
        assert!(!cache_dir.join("authpass#1.10#c.exe").exists());
        assert!(
            cache_dir.join("authpass.txt").exists(),
            "sidecar untouched: only `cache rm` takes it"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The wildcard `cache rm *` clears the whole cache directory (including
    /// the aria2 sidecars), reports the recognizable cache files in `list`
    /// order, and returns nothing when the cache directory does not exist.
    #[test]
    fn wildcard_remove_reports_cache_files_and_clears_the_dir() {
        let root = crate::test_utils::tmpdir("cache_remove_all");
        let session = crate::test_utils::test_session(&root);
        let cache_dir = root.join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();
        for name in ["zip#2.0#b.zip", "7zip#26.0#c.7z", "zip.txt"] {
            std::fs::write(cache_dir.join(name), b"x").unwrap();
        }

        let removed = remove(&session, &[]).unwrap();
        assert_eq!(
            removed.iter().map(CacheFile::file_name).collect::<Vec<_>>(),
            ["7zip#26.0#c.7z", "zip#2.0#b.zip"],
        );
        assert_eq!(removed.iter().map(CacheFile::size).sum::<u64>(), 2);
        assert!(
            cache_dir.read_dir().unwrap().next().is_none(),
            "dir cleared"
        );

        // A wildcard removal without a cache directory is a no-op.
        std::fs::remove_dir_all(&cache_dir).unwrap();
        assert!(remove(&session, &["*".to_owned()]).unwrap().is_empty());
    }
}
