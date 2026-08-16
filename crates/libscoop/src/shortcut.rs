//! Windows Start Menu shortcut management.
//!
//! Creates and removes shortcuts in the `Scoop Apps` folder under the
//! Start Menu — per-user (`~\AppData\Roaming\...\Programs\Scoop Apps`)
//! or machine-wide (`C:\ProgramData\...\Programs\Scoop Apps` for global
//! installs), mirroring Scoop's `shortcut_folder`.
//!
//! # Design
//!
//! - **Scope-aware directory**: The shortcut directory is derived from the
//!   session's global flag; target paths resolve against the effective root.
//! - **Per-package shortcuts**: [`add()`] reads `shortcuts` from the
//!   package manifest and creates each entry; [`remove()`] cleans them up.
//! - **Conflict detection**: Existing `.lnk` files are overwritten silently
//!   when they belong to the same package (matching upstream Scoop's
//!   `startmenu_shortcut`, which never warns). A warning is emitted only when
//!   the existing shortcut belongs to a *different* package.
//! - **Zero-dependency ownership probe**: whether an existing `.lnk` belongs
//!   to another package is decided by scanning the raw bytes for an
//!   `apps\<pkg>\` segment pair in two encodings (ANSI/NUL and UTF-16LE),
//!   without a full MS-SHLLINK parse — see [`shortcut_owner_package`].

use std::path::{Path, PathBuf};

use crate::{error::Fallible, internal, package::Package, Event, Session};

/// Return the path to the `Scoop Apps` shortcut folder.
///
/// User scope: `~\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Scoop Apps`
/// Global scope: `C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Scoop Apps`
/// (mirrors Scoop's `shortcut_folder`, which picks `StartMenu` vs `CommonStartMenu`).
fn shortcut_dir(global: bool) -> PathBuf {
    let mut dir = if global {
        std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
    } else {
        // Missing APPDATA (service accounts, stripped environments): fall
        // back to a relative path rather than panicking.
        dirs::config_dir().unwrap_or_default()
    };
    dir.push("Microsoft/Windows/Start Menu/Programs/Scoop Apps");
    internal::path::normalize_path(dir)
}

/// Build the `.lnk` path for a shortcut display name.
///
/// Upstream Scoop appends `.lnk` to the display name verbatim
/// (`startmenu_shortcut`: `"$folder\$shortcutName.lnk"`), so a name like
/// `paint.net` yields `paint.net.lnk` — never `paint.lnk`. This must not go
/// through `Path::set_extension`, which would replace a dotted suffix.
fn shortcut_link_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.lnk"))
}

/// Add shortcut(s) for a given package.
pub fn add(session: &Session, package: &Package) -> Fallible<()> {
    if let Some(shortcuts) = package.manifest().shortcuts() {
        let apps_dir = session.apps_dir();
        let shortcut_dir = shortcut_dir(session.is_global());

        // Ensure shortcut dir exists
        internal::fs::ensure_dir(&shortcut_dir)?;

        // Runtime entry dir: `current` junction, or the version dir under
        // `NO_JUNCTION` (upstream `create_startmenu_shortcuts` receives the
        // `link_current` result).
        let version = package.effective_version();
        let version_dir = session.current_dir_name(&version);

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageShortcutAddStart);
        }

        for shortcut in shortcuts {
            let length = shortcut.len();
            assert!(length > 1);

            // Scoop shortcut format:
            //   [0] = target exe (relative to package dir)
            //   [1] = display name in start menu
            //   [2] = optional arguments
            //   [3] = optional icon path (relative to package dir)
            let target = apps_dir
                .join(package.name())
                .join(version_dir)
                .join(shortcut[0]);
            let target_str = target.to_string_lossy().into_owned();

            let args = shortcut.get(2).map(|s| s.to_string());
            let icon = shortcut.get(3).map(|s| {
                apps_dir
                    .join(package.name())
                    .join(version_dir)
                    .join(s)
                    .to_string_lossy()
                    .into_owned()
            });

            let link_path = shortcut_link_path(&shortcut_dir, shortcut[1]);

            // Overwrite existing .lnk. Warn only when it belongs to a
            // different package; same-package shortcuts (e.g. on update) and
            // shortcuts whose owner cannot be determined are overwritten
            // silently, matching upstream Scoop's `startmenu_shortcut`.
            if link_path.exists() {
                let owner = shortcut_owner_package(&link_path);
                let conflicts_other = owner
                    .as_deref()
                    .is_some_and(|o| !o.eq_ignore_ascii_case(package.name()));
                if conflicts_other {
                    if let Some(tx) = session.emitter() {
                        let _ = tx.send(Event::PackageShortcutConflict(
                            link_path.to_string_lossy().into_owned(),
                        ));
                    }
                }
            }

            create_shortcut(&target_str, &link_path, args, icon)?;

            if let Some(tx) = session.emitter() {
                let name = link_path.file_name().unwrap().to_str().unwrap().to_owned();
                let _ = tx.send(Event::PackageShortcutAddProgress(name));
            }
        }

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageShortcutAddDone);
        }
    }

    Ok(())
}

/// Create a `.lnk` shortcut file using `shortcuts-rs` (pure Rust LNK writer).
fn create_shortcut(
    target: &str,
    link: &Path,
    args: Option<String>,
    icon: Option<String>,
) -> std::io::Result<()> {
    use shortcuts_rs::ShellLink;

    let link_str = link.to_string_lossy();
    // The display name is derived from the .lnk filename (minus .lnk extension)
    let name = link
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_owned());

    // `ShellLink::new` (shortcuts-rs 1.1.1) applies the arguments/name/icon
    // setters *before* overwriting the header link flags, which drops
    // HAS_ARGUMENTS/HAS_NAME/HAS_ICON_LOCATION — the strings then never reach
    // the .lnk (observed with ima-copilot's `--force-device-scale-factor=1.5`).
    // Re-apply the setters afterwards so the flags stick.
    let mut sl = ShellLink::new(target, None, None, None).map_err(std::io::Error::other)?;
    sl.set_arguments(args);
    sl.set_name(name);
    sl.set_icon_location(icon);
    sl.create_lnk(link_str.as_ref())
        .map_err(std::io::Error::other)
}

/// Determine the package that owns an existing `.lnk` file, if any.
///
/// The owner is the path segment right after the `apps` directory in the
/// shortcut's target path (e.g. `git` in `...\apps\git\current\bin\bash.exe`).
/// Instead of parsing the MS-SHLLINK structure, the raw bytes are scanned
/// for an `apps` path segment followed by a package-name segment, in two
/// encodings:
///
/// 1. ANSI/NUL view — every string in a .lnk is NUL-terminated and UTF-16
///    high bytes are 0x00, so splitting the raw bytes on 0x00 yields the
///    path segments as words. Covers WScript.Shell's LinkTargetIDList
///    (upstream Scoop) and the ANSI segments of any writer.
/// 2. UTF-16LE view — bytes re-interpreted as UTF-16LE ASCII (high byte
///    0x00 kept, anything else becomes a segment separator). Covers the
///    UTF-16 `StringData.WorkingDir` written by shortcuts-rs.
///
/// Returns `None` when no `apps\<pkg>\` segment pair can be found (missing/
/// malformed file, target outside a scoop `apps` dir, non-ASCII segments).
/// Callers treat `None` as "unknown" and keep the conservative silent
/// behavior — the same failure mode as upstream Scoop, which never warns.
fn shortcut_owner_package(link: &Path) -> Option<String> {
    let bytes = std::fs::read(link).ok()?;
    owner_from_bytes(&bytes)
}

/// Scan raw `.lnk` bytes for an `apps` path segment followed by a
/// package-name segment, trying both the ANSI/NUL and UTF-16LE views.
fn owner_from_bytes(bytes: &[u8]) -> Option<String> {
    // View 1: ANSI/NUL segments (WScript.Shell IDList, ANSI words).
    if let Some(pkg) = owner_from_segments(bytes) {
        return Some(pkg);
    }
    // View 2: UTF-16LE ASCII view (shortcuts-rs WorkingDir).
    owner_from_segments(&utf16_ascii_view(bytes))
}

/// Re-interpret bytes as UTF-16LE: keep characters whose high byte is 0x00
/// (ASCII), replace anything else with a segment separator.
fn utf16_ascii_view(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        out.push(if pair[1] == 0 { pair[0] } else { 0 });
    }
    out
}

/// Split on NUL and path separators, keep "word" segments (>=2 chars,
/// alphanumeric plus `.`/`-`/`_`), and return the word right after an
/// `apps` word — but only when the segment after it looks like a package
/// version dir (`current` or containing a digit), which anchors the match
/// to a real `apps\<pkg>\<version-dir>\` layout. This rejects lookalikes
/// such as a scoop root nested under an `apps` dir (`D:\apps\scoop\apps\...`)
/// or a non-ASCII package dir that was skipped by the word filter.
///
/// Fuzziness accepted (failure modes are a spurious warning or a
/// conservative silent overwrite, never data loss):
/// - file-size/attribute DWORDs can occasionally decode to a word; the odds
///   of one literally spelling `apps` are negligible.
/// - non-ASCII (e.g. CJK) segments are skipped -> owner `None` -> silent.
fn owner_from_segments(view: &[u8]) -> Option<String> {
    let mut words: Vec<String> = Vec::new();
    for seg in view.split(|&b| b == 0 || b == b'\\' || b == b'/') {
        if seg.is_empty() {
            continue;
        }
        let Ok(s) = std::str::from_utf8(seg) else {
            continue;
        };
        if s.len() >= 2
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        {
            words.push(s.to_ascii_lowercase());
        }
    }
    for w in words.windows(3) {
        if w[0] == "apps" && crate::internal::path::is_version_dir(&w[2]) {
            return Some(w[1].clone());
        }
    }
    None
}

/// Remove shortcut(s) for a given package.
pub fn remove(session: &Session, package: &Package) -> Fallible<()> {
    assert!(package.is_installed());

    if let Some(shortcuts) = package.manifest().shortcuts() {
        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageShortcutRemoveStart);
        }

        for shortcut in shortcuts {
            let length = shortcut.len();
            assert!(length > 1);

            let path = shortcut_link_path(&shortcut_dir(session.is_global()), shortcut[1]);

            if let Some(tx) = session.emitter() {
                let shortcut_name = path.file_name().unwrap().to_str().unwrap().to_owned();
                let _ = tx.send(Event::PackageShortcutRemoveProgress(shortcut_name));
            }

            if path.exists() {
                std::fs::remove_file(&path)?;
            } else {
                if let Some(tx) = session.emitter() {
                    let _ = tx.send(Event::PackageShortcutNotFound(
                        path.to_string_lossy().into_owned(),
                    ));
                }
            }
        }

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageShortcutRemoveDone);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::manifest::Manifest;
    use std::path::Path;

    #[test]
    fn shortcut_link_path_keeps_dots_in_name() {
        let dir = Path::new("C:\\Start Menu\\Programs\\Scoop Apps");
        // dotted name must keep its dot: `paint.net` -> `paint.net.lnk`,
        // never `paint.lnk` (upstream Scoop appends `.lnk` verbatim)
        assert_eq!(
            shortcut_link_path(dir, "paint.net"),
            Path::new("C:\\Start Menu\\Programs\\Scoop Apps\\paint.net.lnk")
        );
        // names with spaces are unaffected
        assert_eq!(
            shortcut_link_path(dir, "Git Bash"),
            Path::new("C:\\Start Menu\\Programs\\Scoop Apps\\Git Bash.lnk")
        );
    }

    #[test]
    fn shortcut_name_survives_manifest_parse_to_link_path() {
        // Full chain: manifest `shortcuts` parsing -> `shortcuts()` -> link
        // path construction must keep `paint.net` intact.
        let json = r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT", "shortcuts": [["PaintDotNet.exe", "paint.net"]]}"#;
        let manifest = Manifest::from_json("paint.net", json).unwrap();
        let shortcuts = manifest.shortcuts().expect("shortcuts");
        assert_eq!(shortcuts[0][1], "paint.net");

        let dir = Path::new("C:\\Start Menu\\Programs\\Scoop Apps");
        assert_eq!(
            shortcut_link_path(dir, shortcuts[0][1]),
            Path::new("C:\\Start Menu\\Programs\\Scoop Apps\\paint.net.lnk")
        );
    }

    #[test]
    fn test_create_shortcut_to_exe() {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let target = Path::new(&system_root).join("System32\\cmd.exe");
        let link = std::env::temp_dir().join("hok_test_shortcut.lnk");
        let target_str = target.to_string_lossy().into_owned();

        let _ = std::fs::remove_file(&link);

        let result = create_shortcut(&target_str, &link, None, None);
        assert!(result.is_ok(), "create_shortcut failed: {:?}", result.err());
        assert!(link.exists(), ".lnk file was not created");

        let bytes = std::fs::read(&link).unwrap();
        assert_eq!(
            &bytes[..4],
            &[0x4C, 0x00, 0x00, 0x00],
            "not a valid LNK header"
        );

        let _ = std::fs::remove_file(&link);
    }

    /// UTF-16LE byte encoding of `s` — StringData strings in a .lnk are
    /// UTF-16LE, so presence in the file proves the writer actually emitted
    /// the field (shortcuts-rs 1.1.1's `ShellLink::new` can clear the header
    /// link flags and silently drop arguments/name/icon).
    fn lnk_contains_utf16(bytes: &[u8], s: &str) -> bool {
        let utf16: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        !utf16.is_empty() && bytes.windows(utf16.len()).any(|w| w == utf16.as_slice())
    }

    #[test]
    fn test_create_shortcut_with_args_and_icon() {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let target = Path::new(&system_root).join("System32\\cmd.exe");
        let link = std::env::temp_dir().join("hok_test_shortcut_args.lnk");
        let target_str = target.to_string_lossy().into_owned();

        let _ = std::fs::remove_file(&link);

        let result = create_shortcut(
            &target_str,
            &link,
            Some("/k echo hello".into()),
            Some(target_str.clone()),
        );
        assert!(
            result.is_ok(),
            "create_shortcut with args/icon failed: {:?}",
            result.err()
        );
        assert!(link.exists(), ".lnk file was not created");

        let bytes = std::fs::read(&link).unwrap();
        assert!(
            lnk_contains_utf16(&bytes, "/k echo hello"),
            "arguments string missing from .lnk"
        );
        assert!(
            lnk_contains_utf16(&bytes, &target_str),
            "icon location string missing from .lnk"
        );

        let _ = std::fs::remove_file(&link);
    }

    #[test]
    fn test_create_shortcut_persists_manifest_args() {
        // Regression: hok must write shortcut arguments from the manifest
        // (e.g. ima-copilot's `--force-device-scale-factor=1.5`) into the .lnk.
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let target = Path::new(&system_root).join("System32\\cmd.exe");
        let link = std::env::temp_dir().join("hok_test_shortcut_manifest_args.lnk");
        let target_str = target.to_string_lossy().into_owned();

        let _ = std::fs::remove_file(&link);

        let args = "--force-device-scale-factor=1.5";
        let result = create_shortcut(&target_str, &link, Some(args.into()), None);
        assert!(result.is_ok(), "create_shortcut failed: {:?}", result.err());

        let bytes = std::fs::read(&link).unwrap();
        assert!(
            lnk_contains_utf16(&bytes, args),
            "shortcut arguments `{args}` were not written into the .lnk"
        );

        let _ = std::fs::remove_file(&link);
    }

    #[test]
    fn test_owner_from_bytes_ansi_view() {
        // ANSI/NUL view: WScript.Shell IDList style NUL-separated segments
        assert_eq!(
            owner_from_bytes(b"\x00scoop\x00apps\x00git\x00current\x00"),
            Some("git".into())
        );
        // `hokapps` must not be mistaken for an `apps` segment (exact segment)
        assert_eq!(
            owner_from_bytes(b"\x00hokapps\x00apps\x00python\x00current\x00"),
            Some("python".into())
        );
        // case-insensitive `apps` marker (words are lowercased)
        assert_eq!(
            owner_from_bytes(b"\x00APPS\x00Git\x00current\x00"),
            Some("git".into())
        );
        // version dir may also be a version number (upstream Scoop writes
        // apps\<pkg>\<version>\ before linking current)
        assert_eq!(
            owner_from_bytes(b"\x00apps\x00git\x002.44.0\x00"),
            Some("git".into())
        );
        // a scoop root nested under an `apps` dir must not misattribute
        // (`apps\scoop\apps\git\current\...` -> first `apps` has no version
        // dir after it)
        assert_eq!(
            owner_from_bytes(b"\x00apps\x00scoop\x00apps\x00git\x00current\x00"),
            Some("git".into())
        );
        // no `apps` segment at all
        assert_eq!(
            owner_from_bytes(b"\x00Program Files\x00Git\x00bin\x00"),
            None
        );
        // `apps` with no version dir after the candidate package
        assert_eq!(owner_from_bytes(b"\x00apps\x00python\x00"), None);
        // backslash-separated ANSI path
        assert_eq!(
            owner_from_bytes(br"D:\scoop\apps\git\current\bin"),
            Some("git".into())
        );
    }

    #[test]
    fn test_owner_from_bytes_utf16_view() {
        // UTF-16LE view: shortcuts-rs `StringData.WorkingDir` is a UTF-16LE
        // full path; low bytes spell the path, high bytes are 0x00.
        let path = "D:\\scoop\\apps\\git\\current\\bin";
        let mut bytes: Vec<u8> = path.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        bytes.extend_from_slice(&[0x00, 0x00]); // NUL terminator
                                                // The ANSI/NUL view sees only single-char fragments here -> misses,
                                                // the UTF-16 view must find it.
        assert_eq!(owner_from_bytes(&bytes), Some("git".into()));

        // Binary junk with non-zero UTF-16 high bytes must not confuse either
        // view (high bytes become segment separators).
        let mut junk: Vec<u8> = vec![0x00, 0xFF, 0x41, 0x80, 0x00, 0x01];
        junk.extend(path.encode_utf16().flat_map(|u| u.to_le_bytes()));
        assert_eq!(owner_from_bytes(&junk), Some("git".into()));
    }

    #[test]
    fn test_shortcut_owner_package() {
        let root = std::env::temp_dir().join("hok_lnk_test");
        let _ = std::fs::remove_dir_all(&root);

        // --- shortcut whose target lives under apps\<pkg> ---
        let target = root.join("apps/git/current/bin/bash.exe");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"MZ").unwrap();
        let link = root.join("Git Bash.lnk");
        create_shortcut(&target.to_string_lossy(), &link, None, None).unwrap();
        assert_eq!(shortcut_owner_package(&link).as_deref(), Some("git"));

        // --- shortcut for a different package ---
        let other = root.join("apps/python/current/python.exe");
        std::fs::create_dir_all(other.parent().unwrap()).unwrap();
        std::fs::write(&other, b"MZ").unwrap();
        let link2 = root.join("Python.lnk");
        create_shortcut(&other.to_string_lossy(), &link2, None, None).unwrap();
        assert_eq!(shortcut_owner_package(&link2).as_deref(), Some("python"));

        // --- shortcut outside any scoop apps dir ---
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let plain_target = Path::new(&system_root).join("System32\\cmd.exe");
        let link3 = root.join("Plain.lnk");
        create_shortcut(&plain_target.to_string_lossy(), &link3, None, None).unwrap();
        assert_eq!(shortcut_owner_package(&link3), None);

        // --- missing file ---
        assert_eq!(shortcut_owner_package(&root.join("missing.lnk")), None);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_shortcut_owner_package_wscript_format() {
        // A .lnk written by WScript.Shell (upstream Scoop's writer): it has no
        // LinkInfo/StringData, the target path lives only in the
        // LinkTargetIdList as NUL-separated path segments. Target in fixture:
        // `D:\scoop\apps\git\current\usr\bin\bash.exe`.
        let bytes = include_bytes!("../tests/fixtures/wscript_git.lnk");
        let link = std::env::temp_dir().join("hok_test_wscript_git.lnk");
        std::fs::write(&link, bytes).unwrap();
        assert_eq!(shortcut_owner_package(&link).as_deref(), Some("git"));
        let _ = std::fs::remove_file(&link);
    }
}
