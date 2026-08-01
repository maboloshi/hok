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
//! - **Conflict resolution**: When two packages provide the same shim name,
//!   the second one gets an alternative filename via [`alt_filename()`]
//!   (e.g. `foo.ps1.scoop` for a PowerShell script from the "scoop" package).
//! - **Known gaps vs Scoop**: JAR shims are simpler (no `pushd`/`popd`);
//!   GUI `.exe` detection (PE subsystem) is not implemented.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

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
    let shims_dir = session.effective_root_path().join("shims");
    internal::fs::ensure_dir(&shims_dir)?;

    if let Some(bins) = package.manifest().bin() {
        let pkg_name = package.name();

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

                // Handle conflicts: if .exe or .shim already exist (from another package),
                // rename them to alt names before writing ours
                if shim_exe.exists() {
                    let alt_exe =
                        shim_exe.with_file_name(format!("{}.{}.exe", shim.name, pkg_name));
                    std::fs::rename(&shim_exe, &alt_exe)?;
                }
                if shim_meta.exists() {
                    let alt_meta =
                        shim_meta.with_file_name(format!("{}.{}.shim", shim.name, pkg_name));
                    std::fs::rename(&shim_meta, &alt_meta)?;
                }

                // Write the embedded shim
                std::fs::write(&shim_exe, HOK_SHIM_BYTES)?;

                // Write .shim metadata file
                let target_rel = format!(r#"~\..\apps\{}\current\{}"#, pkg_name, shim.real_name);
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
                let batches = generate_shim_batches(&shim, pkg_name);

                for (path, content) in batches {
                    let full_path = shims_dir.join(&path);
                    // Check if the shim already exists (from another package)
                    let dest = if full_path.exists() {
                        // Add package name suffix to avoid conflict
                        alt_filename(&full_path, pkg_name)
                    } else {
                        full_path
                    };

                    std::fs::write(&dest, content.as_bytes())?;

                    if let Some(tx) = session.emitter() {
                        let name = dest.file_name().unwrap().to_string_lossy().to_string();
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
fn generate_shim_batches(shim: &Shim, pkg_name: &str) -> Vec<(PathBuf, String)> {
    let mut result = Vec::new();

    // The target path relative to the shims dir: ..\apps\pkgname\current\real_name
    let target_rel = format!(r#"..\apps\{}\current\{}"#, pkg_name, shim.real_name);

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
                "path = \"~\\..\\apps\\{}\\current\\{}\"\r\n",
                pkg_name, shim.real_name
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

    let shims_dir = session.effective_root_path().join("shims");

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
                ShimType::Exe => vec!["exe", "shim"],
                ShimType::PowerShell => vec!["cmd", "ps1", ""],
                _ => vec!["cmd", ""],
            };

            for ext in exts.into_iter() {
                // Build the main file path: {name}.{ext}
                let mut shim_path = shims_dir.join(shim.name);
                if !ext.is_empty() {
                    shim_path.set_extension(ext);
                }

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

                            if name.starts_with(fname) && name != fname {
                                Some(entry)
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>();

                    if alt_shims.is_empty() {
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
