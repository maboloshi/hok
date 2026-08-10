//! PowerShell script execution & installer-variable expansion primitives.

use std::path::Path;
use tracing::debug;

use crate::package::Package;
use crate::{error::Fallible, internal, Error, Session};

/// Execute a PowerShell script defined in a package manifest.
///
/// `script_lines` is an array of PowerShell command lines that will be joined
/// and executed via `powershell.exe`. The function is a no-op if `script_lines`
/// is `None`.
///
/// Environment variables set for the script:
/// - `SCOOP` — the Scoop root directory
/// - `SCOOP_APP_DIR` — the package's installation directory (`$dir`)
/// - `SCOOP_APP_ORIGINAL_DIR` — the real (versioned) install directory
///   (`$original_dir`), set when it differs from `$dir` (post_install runs
///   with `$dir` = the `current` junction, mirroring upstream `link_current`)
/// - `SCOOP_PACKAGE_NAME` — the package name
/// - `SCOOP_PACKAGE_VERSION` — the installed version
/// - `version` — same as SCOOP_PACKAGE_VERSION (Scoop convention)
pub fn run_script(
    session: &Session,
    package: &Package,
    working_dir: &Path,
    original_dir: Option<&Path>,
    stage: &str,
    cmd: &str,
    script_lines: Option<Vec<&str>>,
) -> Fallible<()> {
    let lines = match script_lines {
        Some(l) if !l.is_empty() => l,
        _ => return Ok(()),
    };

    debug!(
        "run_script: {} stage={} ({} lines)",
        package.name(),
        stage,
        lines.len()
    );

    let script = lines.join("\r\n");

    // Embed PS helper scripts so package scripts can use functions
    // like Expand-InnoArchive, Expand-7zipArchive, Get-HelperPath, etc.
    const CORE_PS1: &str = include_str!("../../../../../asset_scripts/core.ps1");
    const DECOMPRESS_PS1: &str = include_str!("../../../../../asset_scripts/decompress.ps1");
    let preamble = format!(
        r#"
# Hok embedded helpers (always available)
{core}
{decompress}
# Fallback: load Scoop lib if installed
$hokScoopLib = Join-Path $env:SCOOP "apps\scoop\current\lib"
if (Test-Path $hokScoopLib) {{
    Get-ChildItem $hokScoopLib -Filter *.ps1 | ForEach-Object {{ . $_.FullName -ErrorAction SilentlyContinue }}
}}

# Notify hok about undefined commands (missing helpers)
trap {{
    if ($_.Exception -is [System.Management.Automation.CommandNotFoundException]) {{
        Write-Host "[[HOK_MISSING_HELPER]]$($_.Exception.CommandName)"
    }}
    continue
}}

# Scoop-compatible variables for package scripts
$dir = $env:SCOOP_APP_DIR
$original_dir = if ($env:SCOOP_APP_ORIGINAL_DIR) {{ $env:SCOOP_APP_ORIGINAL_DIR }} else {{ $dir }}
$scoopdir = $env:SCOOP
$bucketsdir = Join-Path $scoopdir "buckets"
$persist_dir = Join-Path $scoopdir "persist" $env:SCOOP_PACKAGE_NAME
$version = $env:SCOOP_PACKAGE_VERSION
$app = $env:SCOOP_PACKAGE_NAME
$bucket = $env:SCOOP_PACKAGE_BUCKET
$architecture = "{arch}"
$global = {is_global}
$cmd = $env:SCOOP_PACKAGE_CMD
"#,
        core = CORE_PS1,
        decompress = DECOMPRESS_PS1,
        arch = crate::internal::os::scoop_arch(),
        is_global = session.is_global()
    );
    let full_script = format!("{preamble}\r\n{script}");

    // Write script to a temp file in the working dir
    let script_path = working_dir.join(format!("{}.ps1", stage));
    if let Some(parent) = script_path.parent() {
        internal::fs::ensure_dir(parent)?;
    }
    std::fs::write(&script_path, &full_script)?;

    // Build environment variables
    let root_path = session.effective_root_path();
    let pkg_dir = working_dir.to_path_buf(); // $dir = version dir (not current)
    let original_dir = original_dir.unwrap_or(working_dir);

    let version = package.version();

    // Create marker file for P2 extraction routing
    let marker_path = working_dir.join("hok_extract_markers.txt");
    let _ = std::fs::remove_file(&marker_path); // clean from previous runs

    // Ensure both temp files are cleaned up on return, even via `?`
    let _guard = TempFileGuard::new(vec![script_path.clone(), marker_path.clone()]);

    // Prefer pwsh.exe (PowerShell Core, faster startup)
    let mut ps = crate::internal::os::ps_command();
    ps.arg("-File")
        .arg(&script_path)
        .env("SCOOP", root_path.as_os_str())
        .env("SCOOP_APP_DIR", pkg_dir.as_os_str())
        .env("SCOOP_APP_ORIGINAL_DIR", original_dir.as_os_str())
        .env("SCOOP_PACKAGE_NAME", package.name())
        .env("SCOOP_PACKAGE_VERSION", version)
        .env("SCOOP_PACKAGE_BUCKET", package.bucket())
        .env("SCOOP_PACKAGE_CMD", cmd)
        .env("version", version)
        .env("HOK_EXTRACT_FILE", marker_path.as_os_str());

    let status = ps.status().map_err(|e| {
        Error::Custom(format!(
            "failed to run {} script for '{}': {}",
            stage,
            package.name(),
            e
        ))
    })?;

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        return Err(Error::Custom(format!(
            "{} script for '{}' exited with code {}",
            stage,
            package.name(),
            code
        )));
    }

    // Process extraction markers (P2: Rust native extraction)
    super::extract_markers(session, working_dir);

    Ok(())
}

/// Expand Scoop-style variables (`$dir`, `$scoopdir`, `$persist_dir`, etc.)
/// in installer/uninstaller args and `env_set` values, replacing them with
/// the actual filesystem paths.
///
/// This mirrors the variable definitions in `run_script`'s PowerShell preamble,
/// so that `installer.file` and `uninstaller.file` (which run via `run_gui`
/// rather than through PowerShell) get equivalent variable expansion.
pub fn expand_scoop_vars(
    args: &[&str],
    session: &Session,
    pkg: &Package,
    working_dir: &Path,
    cmd: &str,
) -> Vec<String> {
    args.iter()
        .map(|arg| expand_scoop_str(arg, session, pkg, working_dir, cmd))
        .collect()
}

/// Expand Scoop-style variables in a single string (see [`expand_scoop_vars`]).
pub fn expand_scoop_str(
    s: &str,
    session: &Session,
    pkg: &Package,
    working_dir: &Path,
    cmd: &str,
) -> String {
    let root_path = session.effective_root_path();
    let persist_dir = session.persist_dir(pkg.name());
    let buckets_dir = session.buckets_dir();
    let version = pkg.version();
    let app = pkg.name();
    let bucket = pkg.bucket();

    let working_dir_str = working_dir.to_string_lossy().to_string();
    let root_path_str = root_path.to_string_lossy().to_string();
    let persist_dir_str = persist_dir.to_string_lossy().to_string();
    let buckets_dir_str = buckets_dir.to_string_lossy().to_string();

    let mut s = s.to_string();
    // Longer/replace more specific patterns first to avoid partial overlap
    s = s.replace("$original_dir", &working_dir_str);
    s = s.replace("$persist_dir", &persist_dir_str);
    s = s.replace("$bucketsdir", &buckets_dir_str);
    s = s.replace("$scoopdir", &root_path_str);
    s = s.replace("$architecture", crate::internal::os::scoop_arch());
    s = s.replace("$version", version);
    s = s.replace("$app", app);
    s = s.replace("$bucket", bucket);
    s = s.replace(
        "$global",
        if session.is_global() { "true" } else { "false" },
    );
    s = s.replace("$cmd", cmd);
    s = s.replace("$dir", &working_dir_str);
    s
}

/// Run a manifest `installer.file` / `uninstaller.file` executable via
/// `run_gui`, with installer-variable-expanded arguments (see
/// [`expand_scoop_vars`]).
///
/// Mirrors Scoop's `Invoke-InstallerFile` / `Invoke-UninstallerFile`: the
/// binary runs detached (GUI-style) with `working_dir` as its working
/// directory. `stage` (`"installer"` / `"uninstaller"`) is used in error
/// messages; `cmd` (`"install"` / `"uninstall"`) is substituted for `$cmd`.
pub fn run_installer_file(
    session: &Session,
    package: &Package,
    working_dir: &Path,
    stage: &str,
    cmd: &str,
    file: &str,
    raw_args: &[&str],
) -> Fallible<()> {
    debug!("run_installer_file: {} - {}.file", package.name(), stage);
    let exe_path = working_dir.join(file);
    let expanded = expand_scoop_vars(raw_args, session, package, working_dir, cmd);
    let args: Vec<&str> = expanded.iter().map(|s| s.as_str()).collect();
    internal::os::run_gui(&exe_path, &args, Some(working_dir)).map_err(|e| {
        Error::Custom(format!(
            "failed to run {} '{}' for '{}': {}",
            stage,
            file,
            package.name(),
            e
        ))
    })?;
    Ok(())
}

/// Removes the given file paths when dropped. Ensures cleanup even when
/// the calling function returns early via `?`.
struct TempFileGuard(Vec<std::path::PathBuf>);

impl TempFileGuard {
    fn new(paths: Vec<std::path::PathBuf>) -> Self {
        Self(paths)
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that TempFileGuard removes files on Drop.
    #[test]
    fn test_temp_file_guard_cleanup() {
        let dir = crate::test_utils::tmpdir("temp_file_guard");
        let path1 = dir.join("test1.txt");
        let path2 = dir.join("test2.txt");

        std::fs::write(&path1, b"hello").unwrap();
        std::fs::write(&path2, b"world").unwrap();
        assert!(path1.exists());
        assert!(path2.exists());

        {
            let _guard = TempFileGuard::new(vec![path1.clone(), path2.clone()]);
            // Guard is alive — files should still exist
            assert!(path1.exists());
            assert!(path2.exists());
        }
        // Guard dropped — files should be removed
        assert!(!path1.exists());
        assert!(!path2.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Helper to create a test environment for expand_scoop_vars tests.
    /// Cleans up the temp dir on drop via the returned guard.
    struct TestDirGuard(std::path::PathBuf);
    impl Drop for TestDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn setup_expand_scoop_vars_test(
        test_name: &str,
    ) -> (crate::Session, Package, std::path::PathBuf, TestDirGuard) {
        let tmp = crate::test_utils::tmpdir(&format!("expand_scoop_vars_{}", test_name));
        let guard = TestDirGuard(tmp.clone());
        let root = &tmp;

        // Write minimal hok config
        let config_path = root.join("hok.json");
        let root_escaped = root.to_string_lossy().replace('\\', "\\\\");
        std::fs::write(
            &config_path,
            format!(r#"{{"root_path": "{}"}}"#, root_escaped),
        )
        .unwrap();

        let session = crate::Session::new_with(&config_path).unwrap();
        let manifest = crate::package::Manifest::from_json(
            "test-pkg",
            r#"{"version": "1.0.0", "homepage": "https://example.com", "license": "MIT"}"#,
        )
        .unwrap();
        let pkg = Package::from("test-pkg", "test-bucket", manifest);
        let working_dir = root.join("apps").join("test-pkg").join("1.0.0");

        (session, pkg, working_dir, guard)
    }

    /// Test that $dir is expanded to the working directory path.
    #[test]
    fn test_expand_dir_var() {
        let (session, pkg, working_dir, _tmp) = setup_expand_scoop_vars_test("dir");
        let args = vec!["/DIR=\"$dir\""];
        let expanded = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded.len(), 1);
        assert_eq!(
            expanded[0],
            format!("/DIR=\"{}\"", working_dir.to_string_lossy())
        );
    }

    /// Test that $scoopdir is expanded to the Scoop root path.
    #[test]
    fn test_expand_scoopdir_var() {
        let (session, pkg, working_dir, tmp) = setup_expand_scoop_vars_test("scoopdir");
        let root = &tmp.0;
        let args = vec!["$scoopdir"];
        let expanded = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0], root.to_string_lossy().to_string());
    }

    /// Test that $persist_dir is expanded correctly.
    #[test]
    fn test_expand_persist_dir_var() {
        let (session, pkg, working_dir, tmp) = setup_expand_scoop_vars_test("persist");
        let root = &tmp.0;
        let expected = root.join("persist").join("test-pkg");
        let args = vec!["$persist_dir"];
        let expanded = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0], expected.to_string_lossy().to_string());
    }

    /// Test that $version, $app, and $bucket are expanded.
    #[test]
    fn test_expand_identity_vars() {
        let (session, pkg, working_dir, _tmp) = setup_expand_scoop_vars_test("identity");
        let args = vec!["$version", "$app", "$bucket"];
        let expanded = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded[0], "1.0.0");
        assert_eq!(expanded[1], "test-pkg");
        assert_eq!(expanded[2], "test-bucket");
    }

    /// Test that $cmd is expanded to "install" or "uninstall" accordingly.
    #[test]
    fn test_expand_cmd_var() {
        let (session, pkg, working_dir, _tmp) = setup_expand_scoop_vars_test("cmd");
        let args = vec!["$cmd"];
        let expanded_install = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        let expanded_uninstall =
            expand_scoop_vars(&args, &session, &pkg, &working_dir, "uninstall");
        assert_eq!(expanded_install[0], "install");
        assert_eq!(expanded_uninstall[0], "uninstall");
    }

    /// Test that all variables together in a realistic installer arg string are expanded.
    #[test]
    fn test_expand_all_vars_in_args() {
        let (session, pkg, working_dir, tmp) = setup_expand_scoop_vars_test("all_vars");
        let root = &tmp.0;
        let args = vec![
            "/VERYSILENT",
            "/DIR=\"$dir\"",
            "/D=\"$persist_dir\"",
            "/SCOOP=\"$scoopdir\"",
        ];
        let expanded = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded.len(), 4);
        assert_eq!(expanded[0], "/VERYSILENT");
        assert_eq!(
            expanded[1],
            format!("/DIR=\"{}\"", working_dir.to_string_lossy())
        );
        assert_eq!(
            expanded[2],
            format!(
                "/D=\"{}\"",
                root.join("persist").join("test-pkg").to_string_lossy()
            )
        );
        assert_eq!(
            expanded[3],
            format!("/SCOOP=\"{}\"", root.to_string_lossy())
        );
    }

    /// Test that $original_dir produces the same value as $dir.
    #[test]
    fn test_expand_original_dir_var() {
        let (session, pkg, working_dir, _tmp) = setup_expand_scoop_vars_test("original_dir");
        let args = vec!["$original_dir"];
        let expanded = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded[0], working_dir.to_string_lossy().to_string());
    }

    /// Test that $architecture is expanded (not empty, one of the known values).
    #[test]
    fn test_expand_architecture_var() {
        let (session, pkg, working_dir, _tmp) = setup_expand_scoop_vars_test("arch");
        let args = vec!["$architecture"];
        let expanded = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        assert!(
            expanded[0] == "64bit" || expanded[0] == "32bit" || expanded[0] == "arm64",
            "expected 64bit/32bit/arm64, got {}",
            expanded[0]
        );
    }

    /// Test that variable names inside longer words do NOT get replaced (false positive).
    #[test]
    fn test_expand_no_false_positive() {
        let (session, pkg, working_dir, _tmp) = setup_expand_scoop_vars_test("false_positive");
        // "directory" contains "dir" but should not be replaced
        let args = vec!["directory", "scoopdirectory"];
        let expanded = expand_scoop_vars(&args, &session, &pkg, &working_dir, "install");
        assert_eq!(expanded[0], "directory");
        assert_eq!(expanded[1], "scoopdirectory");
    }

    /// Notes-style expansion: a post-install note expands the full Scoop
    /// variable set (mirror Scoop's `show_notes` → `substitute`, which
    /// replaces all params, not just the three path vars).
    #[test]
    fn test_expand_note_style() {
        let (session, pkg, working_dir, tmp) = setup_expand_scoop_vars_test("note");
        let root = &tmp.0;
        let note =
            "Run \"$dir\\Setup-UVEnv.ps1 $persist_dir\" v$version ($app from $bucket, $scoopdir)";
        let expanded = expand_scoop_str(note, &session, &pkg, &working_dir, "install");
        assert_eq!(
            expanded,
            format!(
                "Run \"{}\\Setup-UVEnv.ps1 {}\" v1.0.0 (test-pkg from test-bucket, {})",
                working_dir.to_string_lossy(),
                root.join("persist").join("test-pkg").to_string_lossy(),
                root.to_string_lossy()
            )
        );
    }
}
