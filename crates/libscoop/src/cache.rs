//! Scoop download cache file representation.
//!
//! Cache files follow the naming convention `app#version#filenamified_url`.
//! The [`CacheFile`] struct parses and validates this pattern, providing
//! typed accessors for `package_name()`, `version()`, and `path()`.

use std::path::{Path, PathBuf};

use crate::constant::REGEX_CACHE_FILE;
use crate::error::{Error, Fallible};

/// Scoop cache file representation
#[derive(Clone, Debug)]
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
fn remove_entry(session: &crate::Session, path: &Path) {
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    if let Err(e) = result
        && e.kind() != std::io::ErrorKind::NotFound
    {
        session.output().error(hok_i18n::t!(
            "cmd.failed_remove_cache",
            path = path.display(),
            error = e
        ));
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
/// # Errors
///
/// I/O errors will be returned if the cache directory is not readable.
pub fn remove_outdated(session: &crate::Session, app: &str, keep_version: &str) -> Fallible<()> {
    for f in list(session, &[app.to_owned()])? {
        if f.version() != keep_version {
            remove_entry(session, f.path());
        }
    }
    Ok(())
}

/// Remove leftover partial downloads (`*.download`).
///
/// Upstream runs this once after `cleanup -k` has swept every app
/// (`scoop-cleanup.ps1:79-81`).
///
/// # Errors
///
/// I/O errors will be returned if the cache directory is not readable.
pub fn remove_incomplete(session: &crate::Session) -> Fallible<()> {
    let cache_dir = session.config().cache_path().to_path_buf();
    let Ok(entries) = cache_dir.read_dir() else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "download") {
            remove_entry(session, &path);
        }
    }
    Ok(())
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

    /// Cache operations against a temporary session: `remove_outdated` keeps
    /// the current version and drops the rest, `remove` is anchored to the
    /// selected app and takes the aria2 sidecar with it, and
    /// `remove_incomplete` clears `*.download` leftovers.
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

        // `cleanup -k`: keep zip 2.0, drop zip 1.0, leave everything else.
        remove_outdated(&session, "zip", "2.0").unwrap();
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

        remove_incomplete(&session).unwrap();
        assert!(!cache_dir.join("partial.download").exists());
        assert!(cache_dir.join("7zip#26.0#c.7z").exists());
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
