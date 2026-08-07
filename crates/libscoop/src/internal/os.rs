//! OS utilities — process enumeration and system checks.
//!
//! Uses raw Win32 FFI on Windows to avoid heavy dependencies like `sysinfo`.
//! The old `sysinfo`-based implementation is commented out below.

#![allow(dead_code)]
use std::ffi::c_void;
use std::path::Path;

use crate::error::{Error, Fallible};
use crate::internal::string::encode_wide;

// ─── FFI declarations ─────────────────────────────────────────────────────

#[link(name = "kernel32")]
extern "system" {
    fn CreateToolhelp32Snapshot(dwFlags: u32, th32ProcessID: u32) -> isize; // HANDLE

    fn Process32FirstW(hSnapshot: isize, lppe: *mut PROCESSENTRY32W) -> i32; // BOOL

    fn Process32NextW(hSnapshot: isize, lppe: *mut PROCESSENTRY32W) -> i32; // BOOL

    fn CloseHandle(hObject: isize) -> i32; // BOOL

    fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> isize; // HANDLE

    fn QueryFullProcessImageNameW(
        hProcess: isize,
        dwFlags: u32,
        lpExeName: *mut u16,
        lpdwSize: *mut u32,
    ) -> i32; // BOOL

    fn WaitForSingleObject(hHandle: isize, dwMilliseconds: u32) -> u32;
    fn GetExitCodeProcess(hProcess: isize, lpExitCode: *mut u32) -> i32;
}

const TH32CS_SNAPPROCESS: u32 = 0x00000002;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const PROCESS_TERMINATE: u32 = 0x0001;

/// Process entry structure for Toolhelp32 snapshot.
#[allow(non_snake_case)]
#[repr(C)]
struct PROCESSENTRY32W {
    dwSize: u32,
    cntUsage: u32,
    th32ProcessID: u32,
    th32DefaultHeapID: usize,
    th32ModuleID: u32,
    cntThreads: u32,
    th32ParentProcessID: u32,
    pcPriClassBase: i32,
    dwFlags: u32,
    szExeFile: [u16; 260], // MAX_PATH
}

// ─── Shell32 FFI for admin/privilege checks ─────────────────────────────────

#[link(name = "shell32")]
extern "system" {
    /// Returns a non-zero value if the current process is running under a
    /// user account that has administrator privileges; zero otherwise.
    /// https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-isuseranadmin
    fn IsUserAnAdmin() -> i32;
}

// ─── Public API ────────────────────────────────────────────────────────────

/// Check whether the current process has administrator privileges.
///
/// Uses `IsUserAnAdmin()` from `shell32.dll` on Windows.
/// Always returns `false` on non-Windows platforms.
pub fn is_admin() -> bool {
    #[cfg(target_os = "windows")]
    {
        // SAFETY: IsUserAnAdmin() has no parameters and no failure mode.
        unsafe { IsUserAnAdmin() != 0 }
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

pub fn os_is_arch64() -> bool {
    match std::mem::size_of::<&char>() {
        4 => false,
        8 => true,
        _ => panic!("unexpected os arch"),
    }
}

/// Check if a given executable is available on the system.
pub fn is_program_available(exe: &str) -> bool {
    if let Ok(path) = std::env::var("PATH") {
        for p in path.split(';') {
            let p = Path::new(p).join(exe);
            if std::fs::metadata(p).is_ok() {
                return true;
            }
        }
    }
    false
}

/// Check whether pwsh.exe (PowerShell Core 7+) is available on PATH.
/// Result is cached via OnceLock — only checked once per process lifetime.
pub fn is_pwsh_available() -> bool {
    static CACHED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| {
        // Fast path: check PATH for pwsh.exe without spawning
        if is_program_available("pwsh.exe") {
            return true;
        }
        // Confirm it actually runs (PATH scan can false-positive on dirs)
        std::process::Command::new("pwsh.exe")
            .arg("-NoProfile")
            .arg("-c")
            .arg("$null")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
    })
}

/// Run a program with the given arguments and wait for it to complete.
///
/// Returns the process exit code (or `-1` if the process was terminated by a
/// signal rather than exiting normally).
pub fn run_program(
    program: &Path,
    args: &[&str],
    working_dir: Option<&Path>,
) -> std::io::Result<i32> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }
    let status = cmd.status()?;
    Ok(status.code().unwrap_or(-1))
}

/// A running process whose executable is under a directory.
#[derive(Clone, Debug)]
pub struct RunningProcess {
    /// Executable file stem (name without extension), e.g. `git-bash`.
    pub name: String,
    /// Process id.
    pub pid: u32,
}

/// Component-wise `Path::starts_with` that ignores ASCII case, matching
/// PowerShell's case-insensitive `-like "$processdir\*"` comparison.
fn starts_with_ignore_case(path: &Path, dir: &Path) -> bool {
    let mut path_components = path.components();
    for dir_comp in dir.components() {
        match path_components.next() {
            Some(p) if p
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&dir_comp.as_os_str().to_string_lossy()) => {}
            _ => return false,
        }
    }
    true
}

/// Find all running processes whose executable is under `dir`
/// (e.g. `apps/<app>`), matching Scoop's
/// `Get-Process | Where-Object { $_.Path -like "$processdir\*" }` directory
/// prefix comparison. Returns one entry per running instance.
pub fn running_processes_under(dir: &Path) -> Fallible<Vec<RunningProcess>> {
    let h_snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if h_snapshot == -1 || h_snapshot == 0 {
        // INVALID_HANDLE_VALUE is -1, NULL is also possible
        return Err(Error::Custom("CreateToolhelp32Snapshot failed".into()));
    }

    let mut processes = Vec::new();
    let mut pe = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        cntUsage: 0,
        th32ProcessID: 0,
        th32DefaultHeapID: 0,
        th32ModuleID: 0,
        cntThreads: 0,
        th32ParentProcessID: 0,
        pcPriClassBase: 0,
        dwFlags: 0,
        szExeFile: [0u16; 260],
    };

    let mut ok = unsafe { Process32FirstW(h_snapshot, &mut pe) };
    while ok != 0 {
        let pid = pe.th32ProcessID;
        if pid != 0 {
            // Open process with minimal access needed to query image path
            let h_process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            if h_process != 0 {
                let mut buf = [0u16; 4096];
                let mut size = buf.len() as u32;
                let ret = unsafe {
                    QueryFullProcessImageNameW(h_process, 0, buf.as_mut_ptr(), &mut size)
                };
                unsafe { CloseHandle(h_process) };

                if ret != 0 {
                    let path_str = String::from_utf16_lossy(&buf[..size as usize]);
                    let path = Path::new(&path_str);

                    // Check if exe is under the target app directory
                    if starts_with_ignore_case(path, dir) {
                        // Extract the exe name without extension
                        if let Some(file_stem) = path.file_stem() {
                            processes.push(RunningProcess {
                                name: file_stem.to_string_lossy().into_owned(),
                                pid,
                            });
                        }
                    }
                }
            }
        }

        pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        ok = unsafe { Process32NextW(h_snapshot, &mut pe) };
    }

    unsafe { CloseHandle(h_snapshot) };

    processes.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(processes)
}

// ─── Old sysinfo implementation (kept for reference) ───────────────────────
// use once_cell::sync::Lazy;
// use std::sync::Mutex;
// use sysinfo::ProcessExt;
// use sysinfo::ProcessRefreshKind;
// use sysinfo::System;
// use sysinfo::SystemExt;
//
// static SYSINFO: Lazy<Mutex<System>> = Lazy::new(|| Mutex::new(System::default()));
//
// pub fn running_apps(path: &Path) -> Fallible<Vec<String>> {
//     let mut sys = SYSINFO.lock().map_err(|e| Error::Custom(e.to_string()))?;
//     sys.refresh_processes_specifics(ProcessRefreshKind::new());
//     let mut proc_names = sys
//         .processes()
//         .values()
//         .filter_map(|p| {
//             let exe_path = p.exe();
//             if exe_path.starts_with(path) {
//                 Some(p.name().to_owned())
//             } else {
//                 None
//             }
//         })
//         .collect::<Vec<_>>();
//     proc_names.sort();
//     proc_names.dedup();
//     Ok(proc_names)
// }

// ─── process execution (ShellExecuteExW) ──────────────────────────────

/// Run a GUI executable with arguments and wait for it to complete.
#[cfg(windows)]
pub fn run_gui(exe: &Path, args: &[&str], working_dir: Option<&Path>) -> std::io::Result<i32> {
    use std::mem;
    use std::ptr;

    const SEE_MASK_NOASYNC: u32 = 0x0000_0010;
    const SEE_MASK_NOCLOSEPROCESS: u32 = 0x0000_0040;
    const SW_SHOWNORMAL: i32 = 1;
    const INFINITE: u32 = 0xFFFF_FFFF;

    #[repr(C)]
    #[allow(clippy::upper_case_acronyms)]
    struct SHELLEXECUTEINFOW {
        cb_size: u32,
        f_mask: u32,
        hwnd: *mut c_void,
        lp_verb: *const u16,
        lp_file: *const u16,
        lp_parameters: *const u16,
        lp_directory: *const u16,
        n_show: i32,
        h_inst_app: *mut c_void,
        lp_id_list: *mut c_void,
        lp_class: *const u16,
        hkey_class: *mut c_void,
        dw_hot_key: u32,
        h_icon: *mut c_void,
        h_process: *mut c_void,
    }

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteExW(p_exec_info: *mut SHELLEXECUTEINFOW) -> i32;
    }

    let exe_wide = encode_wide(&exe.to_string_lossy());
    let args_wide = encode_wide(&args.join(" "));
    let dir_wide = working_dir
        .map(|p| encode_wide(&p.to_string_lossy()))
        .unwrap_or_default();
    let verb = encode_wide("open");

    let mut info = SHELLEXECUTEINFOW {
        cb_size: mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        f_mask: SEE_MASK_NOASYNC | SEE_MASK_NOCLOSEPROCESS,
        hwnd: ptr::null_mut(),
        lp_verb: verb.as_ptr(),
        lp_file: exe_wide.as_ptr(),
        lp_parameters: args_wide.as_ptr(),
        lp_directory: if dir_wide.is_empty() {
            ptr::null()
        } else {
            dir_wide.as_ptr()
        },
        n_show: SW_SHOWNORMAL,
        h_inst_app: ptr::null_mut(),
        lp_id_list: ptr::null_mut(),
        lp_class: ptr::null(),
        hkey_class: ptr::null_mut(),
        dw_hot_key: 0,
        h_icon: ptr::null_mut(),
        h_process: ptr::null_mut(),
    };

    let ret = unsafe { ShellExecuteExW(&mut info) };
    if ret == 0 {
        return Err(std::io::Error::last_os_error());
    }

    unsafe {
        let wait = WaitForSingleObject(info.h_process as isize, INFINITE);
        if wait != 0 {
            let _ = CloseHandle(info.h_process as isize);
            return Err(std::io::Error::last_os_error());
        }
        let mut exit_code: u32 = 0;
        GetExitCodeProcess(info.h_process as isize, &mut exit_code);
        CloseHandle(info.h_process as isize);
        Ok(exit_code as i32)
    }
}

// ─── shell-open (ShellExecuteW) ─────────────────────────────────────────

/// Open a URL in the default system browser.
#[cfg(windows)]
pub fn open_url(url: &str) -> std::io::Result<()> {
    shell_open(url)
}

/// Open a file or directory with the system default handler.
#[cfg(windows)]
pub fn open_file(path: &Path) -> std::io::Result<()> {
    shell_open(&path.as_os_str().to_string_lossy())
}

/// Shell-open a path via `ShellExecuteW` (shared by `open_url` / `open_file`).
// Safety: `file` is converted to a null-terminated UTF-16 string.
#[cfg(windows)]
fn shell_open(file: &str) -> std::io::Result<()> {
    let wide = encode_wide(file);
    let verb = encode_wide("open");

    // Safety: lpOperation and lpFile point to valid null-terminated UTF-16
    // strings. lpParameters and lpDirectory are null. hwnd is null (no parent).
    let ret = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1, // SW_SHOWNORMAL
        )
    };

    if ret as isize <= 32 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

// Safety: lpOperation, lpFile, lpParameters, lpDirectory must point to
// null-terminated UTF-16 strings, or be null. hwnd must be a valid window
// handle or null.
#[cfg(windows)]
#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        hwnd: *mut std::ffi::c_void,
        lp_operation: *const u16,
        lp_file: *const u16,
        lp_parameters: *const u16,
        lp_directory: *const u16,
        n_show_cmd: i32,
    ) -> isize;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn starts_with_ignore_case_matches_components() {
        let dir = Path::new(r"C:\scoop\apps\git");
        assert!(starts_with_ignore_case(Path::new(r"C:\scoop\apps\git\git.exe"), dir));
        assert!(starts_with_ignore_case(Path::new(r"C:\SCOOP\Apps\GIT\current\git-bash.exe"), dir));
        // Same prefix but different component: must not match (Path::starts_with semantics).
        assert!(!starts_with_ignore_case(Path::new(r"C:\scoop\apps\git2\git.exe"), dir));
        assert!(!starts_with_ignore_case(Path::new(r"C:\scoop\apps\other\x.exe"), dir));
        // Shorter path than dir.
        assert!(!starts_with_ignore_case(Path::new(r"C:\scoop\apps"), dir));
    }
}
