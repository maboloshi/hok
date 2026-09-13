//! OS utilities — process enumeration and system checks.
//!
//! Uses raw Win32 FFI on Windows to avoid heavy dependencies like `sysinfo`.

// This module is entirely Win32 FFI (Toolhelp32 snapshot, ShellExecuteExW,
// ShellExecuteW, admin checks) — the necessary-FFI exemption to the crate-level
// `unsafe_code = deny` guardrail. Only `os.rs` and `permission.rs` carry a local
// allow; everything else in the crate must stay unsafe-free.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::path::Path;

use crate::error::{Error, Fallible};
use crate::internal::string::encode_wide;

// ─── FFI declarations ─────────────────────────────────────────────────────

#[link(name = "kernel32")]
unsafe extern "system" {
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
unsafe extern "system" {
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

/// The owner account name of a file, mirroring PowerShell's
/// `(Get-Acl $path).Owner.Split('\')[-1]`.
///
/// This is the `Updated by` fallback of `scoop info`
/// (libexec/scoop-info.ps1:132) for manifest files whose bucket is not a git
/// repository: the account name only, without the `DOMAIN\` prefix.
///
/// Returns `None` on non-Windows platforms, when the file's security
/// descriptor cannot be read, or when the owner SID cannot be resolved.
pub fn file_owner_name(path: &Path) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        owner_name_windows(path)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        None
    }
}

/// Windows half of [`file_owner_name`]: read the owner SID from the file's
/// security descriptor and resolve it to an account name.
#[cfg(target_os = "windows")]
fn owner_name_windows(path: &Path) -> Option<String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{LookupAccountSidW, OWNER_SECURITY_INFORMATION};

    let path_wide = encode_wide(&path.to_string_lossy());

    // SAFETY: `path_wide` is a valid null-terminated UTF-16 string and every
    // out-parameter is a valid pointer. On success `sd` is a security
    // descriptor allocated by the API that owns `owner`; both are released
    // with `LocalFree` before returning.
    unsafe {
        let mut owner: *mut c_void = std::ptr::null_mut();
        let mut sd: *mut c_void = std::ptr::null_mut();
        let status = GetNamedSecurityInfoW(
            path_wide.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut sd,
        );
        if status != 0 || owner.is_null() {
            LocalFree(sd);
            return None;
        }

        let mut name = [0u16; 256];
        let mut name_len = name.len() as u32;
        let mut domain = [0u16; 256];
        let mut domain_len = domain.len() as u32;
        let mut sid_use = 0; // SID_NAME_USE
        let ok = LookupAccountSidW(
            std::ptr::null(),
            owner,
            name.as_mut_ptr(),
            &mut name_len,
            domain.as_mut_ptr(),
            &mut domain_len,
            &mut sid_use,
        );
        LocalFree(sd);
        if ok == 0 {
            return None;
        }

        // `cchName` is the character count, excluding the terminating NUL.
        Some(String::from_utf16_lossy(
            &name[..(name_len as usize).min(name.len())],
        ))
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

/// Whether pwsh.exe (PowerShell Core 7+) is on PATH.
///
/// Static lookup only — no process is spawned to "confirm it runs". A shell
/// that exists on PATH but cannot actually start (broken binary, directory
/// placeholder) is left to surface at the real spawn point, where the failure
/// is translated into a clear error (see `operations::script::ps_launch_error`).
pub fn is_pwsh_available() -> bool {
    is_program_available("pwsh.exe")
}

/// Construct a PowerShell `Command` (pwsh preferred), pre-configured with
/// `-NoProfile -ExecutionPolicy Bypass`. Callers add `-File` / `-Command`,
/// env vars, and stdout/stderr handling as needed.
pub fn ps_command() -> std::process::Command {
    let ps_exe = if is_pwsh_available() {
        "pwsh.exe"
    } else {
        "powershell.exe"
    };
    let mut cmd = std::process::Command::new(ps_exe);
    cmd.arg("-NoProfile").arg("-ExecutionPolicy").arg("Bypass");
    cmd
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
            Some(p)
                if p.as_os_str()
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
    unsafe extern "system" {
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
unsafe extern "system" {
    fn ShellExecuteW(
        hwnd: *mut std::ffi::c_void,
        lp_operation: *const u16,
        lp_file: *const u16,
        lp_parameters: *const u16,
        lp_directory: *const u16,
        n_show_cmd: i32,
    ) -> isize;
}

// ─── Diagnostic probes (upstream `lib/diagnostic.ps1` + `scoop-checkup.ps1`) ─

/// The Windows version as `(major, build)`.
///
/// `major` is `CurrentMajorVersionNumber`, which only exists on Windows 10 and
/// later — an older system reports 0, which is exactly upstream's "too old for
/// configurable long paths" case. `build` is `CurrentBuildNumber`. Both are 0
/// when the registry cannot be read.
pub fn windows_version() -> (u32, u32) {
    #[cfg(target_os = "windows")]
    {
        use winreg::RegKey;
        use winreg::enums::HKEY_LOCAL_MACHINE;

        let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        else {
            return (0, 0);
        };
        let major = key
            .get_value::<u32, _>("CurrentMajorVersionNumber")
            .unwrap_or(0);
        let build = key
            .get_value::<String, _>("CurrentBuildNumber")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        (major, build)
    }
    #[cfg(not(target_os = "windows"))]
    {
        (0, 0)
    }
}

/// Whether the `HKEY_LOCAL_MACHINE\<path>` registry key exists.
pub fn registry_key_exists(path: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        use winreg::RegKey;
        use winreg::enums::HKEY_LOCAL_MACHINE;

        RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(path).is_ok()
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        false
    }
}

/// Read a `REG_DWORD` value from `HKEY_LOCAL_MACHINE\<path>`.
///
/// `None` covers both "key/value missing" and "unreadable", so callers must
/// treat it as *unknown* rather than *disabled*.
pub fn registry_dword(path: &str, name: &str) -> Option<u32> {
    #[cfg(target_os = "windows")]
    {
        use winreg::RegKey;
        use winreg::enums::HKEY_LOCAL_MACHINE;

        RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(path)
            .and_then(|key| key.get_value::<u32, _>(name))
            .ok()
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (path, name);
        None
    }
}

/// The immediate subkey names of `HKEY_LOCAL_MACHINE\<path>`.
///
/// Used for the Defender `Exclusions\Paths` list, where each subkey name *is*
/// an excluded path. `None` when the key is missing or unreadable.
pub fn registry_subkeys(path: &str) -> Option<Vec<String>> {
    #[cfg(target_os = "windows")]
    {
        use winreg::RegKey;
        use winreg::enums::HKEY_LOCAL_MACHINE;

        let key = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(path).ok()?;
        Some(
            key.enum_keys()
                .filter_map(std::result::Result::ok)
                .collect(),
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        None
    }
}

/// The `SERVICE_STATUS` structure `QueryServiceStatus` fills in.
#[cfg(target_os = "windows")]
#[repr(C)]
struct ServiceStatus {
    service_type: u32,
    current_state: u32,
    controls_accepted: u32,
    win32_exit_code: u32,
    service_specific_exit_code: u32,
    checkpoint: u32,
    wait_hint: u32,
}

// Safety: every parameter/return is an opaque SCM or service handle (SC_HANDLE)
// or a pointer to a valid `SERVICE_STATUS`.
#[cfg(target_os = "windows")]
#[link(name = "advapi32")]
unsafe extern "system" {
    fn OpenSCManagerW(
        machine_name: *const u16,
        database_name: *const u16,
        desired_access: u32,
    ) -> isize;
    fn OpenServiceW(sc_manager: isize, service_name: *const u16, desired_access: u32) -> isize;
    fn QueryServiceStatus(service: isize, status: *mut ServiceStatus) -> i32;
    fn CloseServiceHandle(handle: isize) -> i32;
}

#[cfg(target_os = "windows")]
const SC_MANAGER_CONNECT: u32 = 0x0001;
#[cfg(target_os = "windows")]
const SERVICE_QUERY_STATUS: u32 = 0x0004;
#[cfg(target_os = "windows")]
const SERVICE_RUNNING: u32 = 0x0000_0004;

/// Whether the Windows service `name` is currently running.
///
/// `None` means the state could not be determined (service manager or service
/// unavailable) — callers must treat it as *unknown*, not *stopped*.
pub fn is_service_running(name: &str) -> Option<bool> {
    #[cfg(target_os = "windows")]
    {
        // SAFETY: null machine/database select the local service manager; the
        // service name is a valid null-terminated UTF-16 string; `status` is a
        // valid out-parameter; every opened handle is closed before returning.
        unsafe {
            let manager = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
            if manager == 0 {
                return None;
            }
            let name_wide = encode_wide(name);
            let service = OpenServiceW(manager, name_wide.as_ptr(), SERVICE_QUERY_STATUS);
            CloseServiceHandle(manager);
            if service == 0 {
                return None;
            }
            let mut status: ServiceStatus = std::mem::zeroed();
            let ok = QueryServiceStatus(service, &mut status);
            CloseServiceHandle(service);
            if ok == 0 {
                return None;
            }
            Some(status.current_state == SERVICE_RUNNING)
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = name;
        None
    }
}

// Safety: `root_path_name` is a null-terminated UTF-16 mount point; the
// out-parameters point to buffers valid for their stated lengths or are null.
#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetVolumePathNameW(
        file_name: *const u16,
        volume_path_name: *mut u16,
        buffer_length: u32,
    ) -> i32;
    fn GetVolumeInformationW(
        root_path_name: *const u16,
        volume_name_buffer: *mut u16,
        volume_name_size: u32,
        volume_serial_number: *mut u32,
        maximum_component_length: *mut u32,
        file_system_flags: *mut u32,
        file_system_name_buffer: *mut u16,
        file_system_name_size: u32,
    ) -> i32;
}

/// Whether the volume backing `path` is formatted as NTFS.
///
/// `None` when the filesystem name cannot be queried (missing path, unmounted
/// volume, rejected call) — callers treat that as *unknown*, so the diagnostic
/// only reports an actual non-NTFS result.
pub fn volume_is_ntfs(path: &Path) -> Option<bool> {
    #[cfg(target_os = "windows")]
    {
        // Resolve the mount point first: `GetVolumeInformationW` wants a root
        // path ("C:\"), while the configured Scoop root is an arbitrary
        // directory.
        let path_wide = encode_wide(&path.to_string_lossy());
        let mut root = [0u16; 260];
        // SAFETY: both buffers are valid for their stated lengths and the input
        // is a valid null-terminated UTF-16 string.
        let resolved =
            unsafe { GetVolumePathNameW(path_wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) };
        if resolved == 0 {
            return None;
        }

        let mut file_system = [0u16; 64];
        // SAFETY: `root` is a null-terminated UTF-16 mount point; the optional
        // out-parameters are null and `file_system` is valid for its length.
        let ok = unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                file_system.as_mut_ptr(),
                file_system.len() as u32,
            )
        };
        if ok == 0 {
            return None;
        }

        let len = file_system
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(file_system.len());
        Some(String::from_utf16_lossy(&file_system[..len]).eq_ignore_ascii_case("NTFS"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[cfg(windows)]
    #[test]
    fn file_owner_name_resolves_an_account_without_the_domain() {
        let dir = std::env::temp_dir().join(format!("hok-owner-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("owned.txt");
        std::fs::write(&file, "x").unwrap();

        let owner = file_owner_name(&file).expect("the owner resolves");
        assert!(!owner.is_empty());
        // Upstream keeps only the last `\`-separated segment.
        assert!(
            !owner.contains('\\'),
            "domain prefix must be stripped: {owner}"
        );
        assert!(file_owner_name(&dir.join("missing.txt")).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn starts_with_ignore_case_matches_components() {
        let dir = Path::new(r"C:\scoop\apps\git");
        assert!(starts_with_ignore_case(
            Path::new(r"C:\scoop\apps\git\git.exe"),
            dir
        ));
        assert!(starts_with_ignore_case(
            Path::new(r"C:\SCOOP\Apps\GIT\current\git-bash.exe"),
            dir
        ));
        // Same prefix but different component: must not match (Path::starts_with semantics).
        assert!(!starts_with_ignore_case(
            Path::new(r"C:\scoop\apps\git2\git.exe"),
            dir
        ));
        assert!(!starts_with_ignore_case(
            Path::new(r"C:\scoop\apps\other\x.exe"),
            dir
        ));
        // Shorter path than dir.
        assert!(!starts_with_ignore_case(Path::new(r"C:\scoop\apps"), dir));
    }
}
