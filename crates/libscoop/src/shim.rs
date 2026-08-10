//! Shim generation for Scoop packages.
//!
//! A "shim" is a tiny executable that acts as a proxy for the real program,
//! adjusting environment variables (especially `PATH`) and arguments before
//! launching the target. This module handles creating and removing shims
//! for installed packages.
//!
//! # Design
//!
//! - **Embedded shim binary**: The actual shim launcher is compiled as a
//!   separate crate (`hok-shim`) and embedded into the library at build time
//!   via `include!(concat!(env!("OUT_DIR"), "/embedded_shim.rs"))`.
//! - **Shim info file (`.shim`)**: Alongside each shim, a metadata file is
//!   written containing the target path, arguments, and shim type. This is
//!   read by `hok-shim-ref` at runtime.
//! - **Conflict resolution**: Mirrors upstream Scoop's `warn_on_overwrite`:
//!   an existing shim is preserved by renaming it to `{name}.{ext}.{owner}`
//!   (e.g. `foo.ps1.scoop`) and a warning is emitted only when it belongs to
//!   a *different* package; same-package updates overwrite silently.
//! - **Known gaps vs Scoop**: JAR shims are simpler (no `pushd`/`popd`);
//!   GUI `.exe` detection (PE subsystem) is not implemented.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use tracing::debug;

use crate::{error::Fallible, internal, package::Package, Event, Session};

include!(concat!(env!("OUT_DIR"), "/embedded_shim.rs"));

/// Generate an alternative filename for conflict resolution.
///
/// If the file has a non-empty extension: `stem.ext.pkg` (e.g. `foo.ps1.scoop`)
/// If the file has no extension:         `stem.pkg` (e.g. `foo.scoop`)
fn alt_filename(path: &Path, pkg: &str) -> PathBuf {
    let stem = path.file_stem().unwrap().to_str().unwrap();
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if !ext.is_empty() => path.with_file_name(format!("{}.{}.{}", stem, ext, pkg)),
        _ => path.with_file_name(format!("{}.{}", stem, pkg)),
    }
}

/// Determine the package an existing shim file belongs to by parsing the
/// target path it points at (upstream Scoop's `get_app_name_from_shim`).
///
/// Both shim styles are recognized:
/// - hok metadata / relative: `path = "~\\..\\apps\\<pkg>\\current\\..."` (.shim)
/// - upstream absolute: `path = "C:\\...\\apps\\<pkg>\\current\\..."` (.shim)
///   and `@rem C:\\...\\apps\\<pkg>\\...` comment lines (.cmd/.ps1 shims)
///
/// Returns `None` when the owner cannot be determined (unreadable file, or no
/// `apps\\<pkg>\\` segment) — upstream returns an empty string in that case
/// and the caller falls back to warn + overwrite.
fn shim_owner_package(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    owner_from_content(&content)
}

fn owner_from_content(content: &str) -> Option<String> {
    // Tokenize like the shortcut ownership probe: split on path separators,
    // keep "word" segments, then look for `apps -> <pkg> -> version-dir`.
    // The version-dir anchor (`current` or containing a digit) rejects
    // lookalikes such as a scoop root nested under an `apps` dir
    // (`D:\apps\scoop\apps\git\current\...` -> `git`, not `scoop`).
    let lower = content.to_ascii_lowercase();
    let mut words: Vec<String> = Vec::new();
    for seg in lower.split(['\\', '/', '\0']) {
        if seg.is_empty() {
            continue;
        }
        if seg.len() >= 2
            && seg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        {
            words.push(seg.to_owned());
        }
    }
    for w in words.windows(3) {
        if w[0] == "apps" && crate::internal::path::is_version_dir(&w[2]) {
            return Some(w[1].clone());
        }
    }
    None
}

/// Warn about and back up an existing shim file before overwriting it,
/// mirroring upstream Scoop's `warn_on_overwrite`:
///
/// - no existing file: nothing to do
/// - same package (e.g. a reinstall/update): silent overwrite
/// - another package: warn, then preserve the existing shim by renaming it to
///   `{name}.{ext}.{owner}` (suffix = the *old* owner, matching upstream's
///   `$shim.$shim_app`); a stale backup under the *new* package name is
///   removed first, as upstream does
/// - unparseable owner: warn + overwrite without a rename — upstream's empty
///   `$shim_app` also fails to produce a valid rename (a trailing-dot name)
///   and silently proceeds to overwrite
fn handle_existing_shim(session: &Session, path: &Path, pkg_name: &str) -> Fallible<()> {
    if !path.exists() {
        return Ok(());
    }

    let owner = shim_owner_package(path);
    let belongs_to_self = owner
        .as_deref()
        .is_some_and(|o| o.eq_ignore_ascii_case(pkg_name));
    if belongs_to_self {
        return Ok(());
    }

    if let Some(tx) = session.emitter() {
        let _ = tx.send(Event::PackageShimConflict(
            path.to_string_lossy().into_owned(),
        ));
    }

    if let Some(owner) = owner {
        // upstream: `Remove-Item "$shim.$path_app"` first, then
        // `Rename-Item $shim "$shim.$shim_app"`
        let stale = alt_filename(path, pkg_name);
        let _ = std::fs::remove_file(&stale);
        let backup = alt_filename(path, &owner);
        if backup.exists() {
            let _ = std::fs::remove_file(&backup);
        }
        // best-effort backup (upstream: -ErrorAction SilentlyContinue); on
        // failure the new shim simply overwrites the old one
        let _ = std::fs::rename(path, &backup);
    }
    Ok(())
}

/// Whether `name` is one of the main shim variants of `fname` (e.g. `foo.shim`
/// for `foo`), which are handled by their own loop iterations in `remove()`
/// rather than being backups — mirrors upstream `rm_shim`'s
/// `-Exclude '*.shim', '*.cmd', '*.ps1'`.
fn is_main_shim_variant(fname: &str, name: &str) -> bool {
    [".shim", ".cmd", ".ps1"]
        .iter()
        .any(|e| name == format!("{fname}{e}"))
}

#[derive(Debug)]
pub struct Shim<'a> {
    name: &'a str,
    real_name: &'a str,
    ty: ShimType,
    args: Option<Vec<&'a str>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ShimType {
    /// Bash script
    ///
    /// A shim will be treated as a Bash script if it does not have a file
    /// extension.
    Bash,

    /// Batch script
    ///
    /// A shim will be treated as a Batch script if it has a `.bat`/`.cmd` file
    /// extension.
    Batch,

    /// Executable
    ///
    /// A shim will be treated as an executable if it has a `.exe`/`.com` file
    /// extension.
    Exe,

    /// Java JAR
    ///
    /// A shim will be treated as a Java JAR if it has a `.jar` file extension.
    Java,

    /// PowerShell script
    ///
    /// A shim will be treated as a PowerShell script if it has a `.ps1` file
    /// extension.
    PowerShell,

    /// Python script
    ///
    /// A shim will be treated as a Python script if it has a `.py` file
    /// extension.
    Python,
}

impl Shim<'_> {
    pub fn new(def: Vec<&str>) -> Shim<'_> {
        let length = def.len();
        assert_ne!(length, 0);

        let real_name = def[0];
        let name = if length == 1 {
            internal::path::leaf_base(real_name).unwrap_or(real_name)
        } else {
            def[1]
        };

        let args = if length < 2 {
            None
        } else {
            Some(def[2..].to_vec())
        };

        let ty = Path::new(real_name)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| match ext.to_lowercase().as_str() {
                "bat" | "cmd" => ShimType::Batch,
                "exe" | "com" => ShimType::Exe,
                "jar" => ShimType::Java,
                "ps1" => ShimType::PowerShell,
                "py" => ShimType::Python,
                _ => ShimType::Bash,
            })
            .unwrap_or(ShimType::Bash);

        Shim {
            name,
            real_name,
            ty,
            args,
        }
    }
}

/// Add shims for a package.
pub fn add(session: &Session, package: &Package) -> Fallible<()> {
    let shims_dir = session.shims_dir();
    internal::fs::ensure_dir(&shims_dir)?;

    if let Some(bins) = package.manifest().bin() {
        let pkg_name = package.name();
        // Runtime entry dir: `current` junction, or the version dir under
        // `NO_JUNCTION` (upstream `create_shims` receives the `link_current`
        // result as `$dir`).
        let version_dir = session.current_dir_name(package.version());

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageShimAddStart(pkg_name.to_owned()));
        }

        for def in bins.into_iter() {
            let shim = Shim::new(def);

            if shim.ty == ShimType::Exe {
                // TODO: GUI detection — read PE subsystem of target; if GUI, skip hok-shim.exe
                // and use ShellExecuteExW path, or mark shim.exe as GUI subsystem.
                // Currently all .exe shims launch with a console window.

                // Use the embedded hok-shim.exe as the native .exe shim
                let shim_exe = shims_dir.join(format!("{}.exe", shim.name));
                let shim_meta = shims_dir.join(format!("{}.shim", shim.name));

                // Upstream Scoop overwrites the .exe stub unconditionally
                // (Copy-Item -Force — it is a generic binary with no owner);
                // only the .shim metadata file carries ownership and gets the
                // `warn_on_overwrite` treatment.
                handle_existing_shim(session, &shim_meta, pkg_name)?;

                // Write the embedded shim
                if let Err(e) = std::fs::write(&shim_exe, HOK_SHIM_BYTES) {
                    // The stub cannot be overwritten while it is running
                    // (e.g. `hok update hok` — the running hok was launched
                    // through shims\hok.exe). The stub is version-independent
                    // (it points at the `current` junction), so when the
                    // target is the binary currently running, silently keep
                    // the existing stub — it resolves to the new version
                    // after `link_current`. Any other failure still aborts.
                    let is_running_self = std::env::current_exe()
                        .ok()
                        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
                        .is_some_and(|stem| stem.eq_ignore_ascii_case(shim.name));
                    if is_running_self && shim_exe.exists() {
                        debug!(
                            "shim stub '{}' is in use by the running process; keeping it",
                            shim_exe.display()
                        );
                    } else {
                        return Err(e.into());
                    }
                }

                // Write .shim metadata file
                let target_rel = format!(
                    r#"~\..\apps\{}\{}\{}"#,
                    pkg_name, version_dir, shim.real_name
                );
                let meta_content = if let Some(args) = &shim.args {
                    format!(
                        "path = \"{target_rel}\"\r\nargs = \"{}\"\r\n",
                        args.join(" ")
                    )
                } else {
                    format!("path = \"{target_rel}\"\r\n")
                };
                std::fs::write(&shim_meta, meta_content.as_bytes())?;

                if let Some(tx) = session.emitter() {
                    let _ = tx.send(Event::PackageShimAddProgress(
                        shim_exe.file_name().unwrap().to_string_lossy().to_string(),
                    ));
                }
            } else {
                // Use script-based shim (.cmd, .ps1, etc.)
                let batches = generate_shim_batches(&shim, pkg_name, version_dir);

                for (path, content) in batches {
                    let full_path = shims_dir.join(&path);
                    // Only the ownership-carrier file gets the
                    // `warn_on_overwrite` treatment (upstream checks the
                    // `.ps1` shim itself, not the `.cmd` wrapper, which is
                    // overwritten unconditionally); same-package updates of
                    // the carrier are overwritten silently.
                    let is_carrier = shim.ty != ShimType::PowerShell
                        || path.extension().and_then(|e| e.to_str()) != Some("cmd");
                    if is_carrier {
                        handle_existing_shim(session, &full_path, pkg_name)?;
                    }

                    std::fs::write(&full_path, content.as_bytes())?;

                    if let Some(tx) = session.emitter() {
                        let name = full_path.file_name().unwrap().to_string_lossy().to_string();
                        let _ = tx.send(Event::PackageShimAddProgress(name));
                    }
                }
            }
        }

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageShimAddDone);
        }
    }

    Ok(())
}

/// Generate shim files content for a given shim definition.
///
/// Returns a list of `(target_path, content)` pairs.
fn generate_shim_batches(shim: &Shim, pkg_name: &str, version_dir: &str) -> Vec<(PathBuf, String)> {
    let mut result = Vec::new();

    // The target path relative to the shims dir: ..\apps\pkgname\<version_dir>\real_name
    let target_rel = format!(r#"..\apps\{}\{}\{}"#, pkg_name, version_dir, shim.real_name);

    let arg_suffix = shim
        .args
        .as_ref()
        .map(|a| format!(" {}", a.join(" ")))
        .unwrap_or_default();

    match shim.ty {
        ShimType::Exe => {
            // .cmd file: batch redirect to the target executable
            // (hok doesn't include Scoop's pre-built shim.exe stub,
            //  so .cmd wrapper is used for CLI access)
            let content = format!("@echo off\r\n\"%~dp0{}\"{} %*\r\n", target_rel, arg_suffix);
            result.push((PathBuf::from(format!("{}.cmd", shim.name)), content));

            // .shim metadata file (Scoop-compatible format)
            let shim_meta = format!(
                "path = \"~\\..\\apps\\{}\\{}\\{}\"\r\n",
                pkg_name, version_dir, shim.real_name
            );
            result.push((PathBuf::from(format!("{}.shim", shim.name)), shim_meta));
        }
        ShimType::Batch | ShimType::Bash => {
            // .cmd file: direct batch redirect
            let content = format!("@echo off\r\n\"%~dp0{}\"{} %*\r\n", target_rel, arg_suffix);
            result.push((PathBuf::from(format!("{}.cmd", shim.name)), content));
        }
        ShimType::PowerShell => {
            // .ps1 shim: PowerShell script
            let target_backslash = target_rel.replace('/', "\\");
            let arg_str = shim.args.as_ref().map(|a| a.join(" ")).unwrap_or_default();
            let ps_content = format!(
                "if ($MyInvocation.ExpectingInput) {{ $input | & \"$PSScriptRoot\\{0}\" {1} @args }} else {{ & \"$PSScriptRoot\\{0}\" {1} @args }}\r\nexit $LASTEXITCODE\r\n",
                target_backslash,
                arg_str,
            );
            result.push((PathBuf::from(format!("{}.ps1", shim.name)), ps_content));

            // .cmd wrapper: batch that calls PowerShell with pwsh fallback
            let cmd_content = format!(
                "@echo off\r\nwhere /q pwsh.exe\r\nif %errorlevel% equ 0 (\r\n    pwsh -NoProfile -ExecutionPolicy Bypass -File \"%~dp0{0}.ps1\" %*\r\n) else (\r\n    powershell -NoProfile -ExecutionPolicy Bypass -File \"%~dp0{0}.ps1\" %*\r\n)\r\n",
                shim.name,
            );
            result.push((PathBuf::from(format!("{}.cmd", shim.name)), cmd_content));
        }
        ShimType::Java => {
            // .cmd file: calls java -jar
            // TODO: add @pushd %~dp0 before java and @popd after (some jars depend on CWD)
            let content = format!(
                "@echo off\r\njava -jar \"%~dp0{}\"{} %*\r\n",
                target_rel, arg_suffix
            );
            result.push((PathBuf::from(format!("{}.cmd", shim.name)), content));
        }
        ShimType::Python => {
            // .cmd file: calls python
            let content = format!(
                "@echo off\r\npython \"%~dp0{}\"{} %*\r\n",
                target_rel, arg_suffix
            );
            result.push((PathBuf::from(format!("{}.cmd", shim.name)), content));
        }
    }

    result
}

/// Remove shims for a package.
pub fn remove(session: &Session, package: &Package) -> Fallible<()> {
    assert!(package.is_installed());

    let shims_dir = session.shims_dir();

    if let Some(bins) = package.manifest().bin() {
        let pkg_name = package.name();
        let shims_dir_entries = shims_dir
            .read_dir()?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageShimRemoveStart);
        }

        for shim in bins.into_iter().map(Shim::new) {
            let exts = match shim.ty {
                // Upstream rm_shim never removes the .exe stub directly: the
                // .shim metadata is the ownership carrier, and the stub is
                // removed only when the .shim had no backup to restore.
                ShimType::Exe => vec!["shim"],
                ShimType::PowerShell => vec!["cmd", "ps1", ""],
                _ => vec!["cmd", ""],
            };

            for ext in exts.into_iter() {
                // Build the main file path: {name}.{ext}. The extension is
                // appended verbatim (matching `generate_shim_batches`), so a
                // dotted name like `foo.bar` yields `foo.bar.cmd`, never
                // `foo.cmd` — removal must find what `add` created.
                let shim_path = if ext.is_empty() {
                    shims_dir.join(shim.name)
                } else {
                    shims_dir.join(format!("{}.{}", shim.name, ext))
                };

                // Alt file path (created by another package's conflict): {name}.{ext}.{pkg}
                let alt_path = alt_filename(&shim_path, pkg_name);

                if alt_path.exists() {
                    if let Some(tx) = session.emitter() {
                        let shim_name = alt_path.file_name().unwrap().to_string_lossy().to_string();
                        let _ = tx.send(Event::PackageShimRemoveProgress(shim_name));
                    }

                    std::fs::remove_file(&alt_path)?;
                } else if shim_path.exists() {
                    if let Some(tx) = session.emitter() {
                        let shim_name =
                            shim_path.file_name().unwrap().to_string_lossy().to_string();
                        let _ = tx.send(Event::PackageShimRemoveProgress(shim_name));
                    }

                    std::fs::remove_file(&shim_path)?;

                    // restore alter shim from another package
                    let fname = shim_path.file_name().unwrap().to_str().unwrap();
                    let mut alt_shims = shims_dir_entries
                        .iter()
                        .flat_map(|entry| {
                            let path = entry.path();
                            let name = path.file_name().unwrap().to_str().unwrap();

                            // matches `{fname}.{owner}` backups, excluding the
                            // main shim variants (`.shim`/`.cmd`/`.ps1` are
                            // handled by their own loop iterations — upstream
                            // `rm_shim` excludes them with `-Exclude`)
                            if name.starts_with(fname)
                                && name != fname
                                && !is_main_shim_variant(fname, name)
                            {
                                Some(entry)
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>();

                    if alt_shims.is_empty() {
                        // upstream: when the removed `.shim` had no backup to
                        // restore, its `.exe` stub is orphaned — remove it too
                        if ext == "shim" {
                            let exe_path = shims_dir.join(format!("{}.exe", shim.name));
                            if exe_path.exists() {
                                if let Some(tx) = session.emitter() {
                                    let name =
                                        exe_path.file_name().unwrap().to_string_lossy().to_string();
                                    let _ = tx.send(Event::PackageShimRemoveProgress(name));
                                }
                                std::fs::remove_file(&exe_path)?;
                            }
                        }
                        continue;
                    }

                    // sort by modified time, so the latest one will be used
                    // when there are multiple alter shims for the same shim
                    if alt_shims.len() > 1 {
                        alt_shims.sort_by_key(|de| {
                            std::cmp::Reverse(de.metadata().unwrap().modified().unwrap())
                        });
                    }

                    let alt_shim = alt_shims.first().unwrap();
                    let alt_old_path = alt_shim.path();
                    let alt_new_path = alt_old_path.with_file_name(fname);
                    std::fs::rename(&alt_old_path, &alt_new_path)?;
                }
            }
        }

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageShimRemoveDone);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Event;
    use crate::package::manifest::Manifest;
    use crate::package::{InstallState, InstallStateInstalled, Package};
    use crate::test_utils;

    /// Build a package whose manifest declares a single `bin` entry.
    fn make_package(name: &str, bin: &str) -> Package {
        let json = format!(
            r#"{{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT", "bin": {}}}"#,
            bin
        );
        Package::from(name, "test", Manifest::from_json(name, &json).unwrap())
    }

    fn mark_installed(pkg: &Package) {
        pkg.fill_install_state(InstallState::Installed(InstallStateInstalled {
            version: "1.0.0".into(),
            bucket: Some("test".into()),
            arch: "64bit".into(),
            held: false,
            url: None,
        }));
    }

    fn conflict_count(rx: &flume::Receiver<Event>) -> usize {
        rx.try_iter()
            .filter(|e| matches!(e, Event::PackageShimConflict(_)))
            .count()
    }

    #[test]
    fn test_owner_from_content() {
        // hok relative .shim metadata
        assert_eq!(
            owner_from_content(r#"path = "~\..\apps\git\current\usr\bin\bash.exe""#),
            Some("git".into())
        );
        // upstream absolute .shim metadata
        assert_eq!(
            owner_from_content(r#"path = "C:\Users\me\scoop\apps\python\current\python.exe""#),
            Some("python".into())
        );
        // upstream .cmd comment line
        assert_eq!(
            owner_from_content("@rem C:\\Users\\me\\scoop\\apps\\git\\current\\bin\\git.exe"),
            Some("git".into())
        );
        // hok PowerShell shim body
        assert_eq!(
            owner_from_content(
                "if ($MyInvocation.ExpectingInput) { $input | & \"$PSScriptRoot\\..\\apps\\python\\current\\Scripts\\foo.ps1\"  @args }"
            ),
            Some("python".into())
        );
        // case-insensitive `apps` marker; owner returned lowercased
        assert_eq!(
            owner_from_content(r#"path = "~\..\APPS\Git\current\bin\git.exe""#),
            Some("git".into())
        );
        // a scoop root nested under an `apps` dir must not misattribute
        assert_eq!(
            owner_from_content(r#"path = "D:\apps\scoop\apps\git\current\bin\git.exe""#),
            Some("git".into())
        );
        // version dir may be a version number
        assert_eq!(
            owner_from_content(r#"path = "~\..\apps\git\2.44.0\bin\git.exe""#),
            Some("git".into())
        );
        // no `apps` segment at all
        assert_eq!(owner_from_content("not a shim at all"), None);
        // `apps` with no version dir after the candidate package
        assert_eq!(owner_from_content(r#"path = "~\..\apps\python\""#), None);
    }

    #[test]
    fn test_add_exe_shim_creates_files() {
        let root = test_utils::tmpdir("shim_add_basic");
        let session = test_utils::test_session(&root);
        let pkg = make_package("foo", r#"[["main.exe", "foo"]]"#);

        add(&session, &pkg).unwrap();

        let shims = root.join("shims");
        assert!(shims.join("foo.exe").exists(), "stub missing");
        let meta = std::fs::read_to_string(shims.join("foo.shim")).unwrap();
        assert!(
            meta.contains(r"apps\foo\current\main.exe"),
            "unexpected meta: {meta}"
        );
    }

    #[test]
    fn test_add_same_package_update_overwrites_silently() {
        let root = test_utils::tmpdir("shim_add_update");
        let session = test_utils::test_session(&root);
        let rx = session.event_bus().receiver();
        let pkg = make_package("foo", r#"[["main.exe", "foo"]]"#);

        add(&session, &pkg).unwrap();
        add(&session, &pkg).unwrap(); // update: same package

        let shims = root.join("shims");
        assert!(shims.join("foo.shim").exists());
        // no backup was created, no conflict was warned
        assert!(!shims.join("foo.shim.foo").exists());
        assert_eq!(conflict_count(&rx), 0);
    }

    #[test]
    fn test_add_other_package_renames_and_warns() {
        let root = test_utils::tmpdir("shim_add_conflict");
        let session = test_utils::test_session(&root);
        let rx = session.event_bus().receiver();
        add(&session, &make_package("git", r#"[["git.exe", "git"]]"#)).unwrap();
        add(&session, &make_package("bgit", r#"[["bgit.exe", "git"]]"#)).unwrap();

        let shims = root.join("shims");
        // old shim preserved under the *old* owner suffix (upstream $shim.$shim_app)
        assert!(shims.join("git.shim.git").exists(), "backup missing");
        assert!(shims.join("git.exe").exists(), "stub missing");
        // new shim now points at the overwriting package
        let meta = std::fs::read_to_string(shims.join("git.shim")).unwrap();
        assert!(
            meta.contains(r"apps\bgit\current\bgit.exe"),
            "unexpected meta: {meta}"
        );
        assert_eq!(conflict_count(&rx), 1);
    }

    #[test]
    fn test_add_unknown_owner_warns_and_overwrites() {
        let root = test_utils::tmpdir("shim_add_unknown");
        let session = test_utils::test_session(&root);
        let rx = session.event_bus().receiver();
        let shims = root.join("shims");
        std::fs::create_dir_all(&shims).unwrap();
        std::fs::write(shims.join("foo.shim"), "not a scoop shim at all").unwrap();

        add(&session, &make_package("foo", r#"[["main.exe", "foo"]]"#)).unwrap();

        let meta = std::fs::read_to_string(shims.join("foo.shim")).unwrap();
        assert!(
            meta.contains(r"apps\foo\current\main.exe"),
            "unexpected meta: {meta}"
        );
        // unknown owner -> warned, but no rename (upstream also fails to rename)
        assert!(!shims.join("foo.shim.foo").exists());
        assert_eq!(conflict_count(&rx), 1);
    }

    #[test]
    fn test_add_script_shim_conflict_renames_carrier_only() {
        let root = test_utils::tmpdir("shim_add_ps_conflict");
        let session = test_utils::test_session(&root);
        let rx = session.event_bus().receiver();
        add(
            &session,
            &make_package("alpha", r#"[["tool.ps1", "tool"]]"#),
        )
        .unwrap();
        add(
            &session,
            &make_package("beta", r#"[["tool2.ps1", "tool"]]"#),
        )
        .unwrap();

        let shims = root.join("shims");
        // the .ps1 carrier is backed up under the old owner...
        assert!(shims.join("tool.ps1.alpha").exists(), "ps1 backup missing");
        // ...but the .cmd wrapper is overwritten unconditionally (upstream)
        assert!(!shims.join("tool.cmd.alpha").exists());
        assert!(shims.join("tool.cmd").exists());
        assert_eq!(conflict_count(&rx), 1);
    }

    #[test]
    fn test_add_shim_no_junction_points_at_version_dir() {
        let root = test_utils::tmpdir("shim_add_no_junction");
        let config_path = root.join("hok.json");
        let root_escaped = root.to_string_lossy().replace('\\', "\\\\");
        let cache_escaped = root.join("cache").to_string_lossy().replace('\\', "\\\\");
        std::fs::write(
            &config_path,
            format!(
                r#"{{"root_path": "{}", "cache_path": "{}", "no_junction": true}}"#,
                root_escaped, cache_escaped
            ),
        )
        .unwrap();
        let session = crate::Session::new_with(&config_path).unwrap();
        let pkg = make_package("foo", r#"[["main.exe", "foo"]]"#);

        add(&session, &pkg).unwrap();

        let shims = root.join("shims");
        let meta = std::fs::read_to_string(shims.join("foo.shim")).unwrap();
        assert!(
            meta.contains(r"apps\foo\1.0.0\main.exe"),
            "no_junction shim should point at the version dir: {meta}"
        );
    }

    #[test]
    fn test_remove_other_package_restores_backup() {
        let root = test_utils::tmpdir("shim_remove_restore");
        let session = test_utils::test_session(&root);
        let pkg_x = make_package("git", r#"[["git.exe", "git"]]"#);
        let pkg_y = make_package("bgit", r#"[["bgit.exe", "git"]]"#);
        add(&session, &pkg_x).unwrap();
        add(&session, &pkg_y).unwrap();

        // removing the overwriter (bgit) must restore git's shim
        mark_installed(&pkg_y);
        remove(&session, &pkg_y).unwrap();

        let shims = root.join("shims");
        assert!(shims.join("git.shim").exists(), "main shim missing");
        let meta = std::fs::read_to_string(shims.join("git.shim")).unwrap();
        assert!(
            meta.contains(r"apps\git\current\git.exe"),
            "expected git's shim restored: {meta}"
        );
        assert!(shims.join("git.exe").exists(), "stub must remain");
    }

    #[test]
    fn test_remove_overwritten_package_keeps_owner_shim() {
        let root = test_utils::tmpdir("shim_remove_overwritten");
        let session = test_utils::test_session(&root);
        let pkg_x = make_package("git", r#"[["git.exe", "git"]]"#);
        let pkg_y = make_package("bgit", r#"[["bgit.exe", "git"]]"#);
        add(&session, &pkg_x).unwrap();
        add(&session, &pkg_y).unwrap();

        // removing the overwritten package (git) must only drop its backup
        mark_installed(&pkg_x);
        remove(&session, &pkg_x).unwrap();

        let shims = root.join("shims");
        assert!(
            !shims.join("git.shim.git").exists(),
            "backup should be gone"
        );
        let meta = std::fs::read_to_string(shims.join("git.shim")).unwrap();
        assert!(
            meta.contains(r"apps\bgit\current\bgit.exe"),
            "bgit's shim must stay: {meta}"
        );
        assert!(shims.join("git.exe").exists(), "stub stays");
    }

    #[test]
    fn test_add_remove_dotted_shim_names() {
        // `bin` entries support two forms: a plain string or an array. A
        // dotted name must survive parsing (`leaf_base` strips only the
        // final `.exe`; the explicit array form keeps `def[1]` verbatim)
        // and stay consistent between add and remove — the `.shim` metadata
        // must be found and removed as `foo.bar.shim`, never `foo.shim`.
        for (bin_json, expect) in [
            (r#"[["main.exe", "foo.bar"]]"#, "foo.bar"), // array form
            (r#"["foo.bar.exe"]"#, "foo.bar"),           // string form
        ] {
            let root = test_utils::tmpdir("shim_remove_dotted");
            let session = test_utils::test_session(&root);
            let pkg = make_package("foo", bin_json);

            add(&session, &pkg).unwrap();
            let shims = root.join("shims");
            assert!(
                shims.join(format!("{expect}.exe")).exists(),
                "add must create {expect}.exe stub for {bin_json}"
            );
            assert!(
                shims.join(format!("{expect}.shim")).exists(),
                "add must create {expect}.shim for {bin_json}"
            );

            mark_installed(&pkg);
            remove(&session, &pkg).unwrap();
            assert!(
                !shims.join(format!("{expect}.shim")).exists(),
                "remove must delete {expect}.shim for {bin_json}"
            );
            assert!(
                !shims.join(format!("{expect}.exe")).exists(),
                "remove must delete the orphaned {expect}.exe stub for {bin_json}"
            );
        }
    }
}
