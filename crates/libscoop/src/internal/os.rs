//! OS utilities — process enumeration and system checks.
//!
//! Uses raw Win32 FFI on Windows to avoid heavy dependencies like `sysinfo`.
//! The old `sysinfo`-based implementation is commented out below.

#![allow(dead_code)]
use std::path::Path;

use crate::error::{Error, Fallible};

// ─── FFI declarations ─────────────────────────────────────────────────────

#[link(name = "kernel32")]
extern "system" {
    fn CreateToolhelp32Snapshot(
        dwFlags: u32,
        th32ProcessID: u32,
    ) -> isize; // HANDLE

    fn Process32FirstW(
        hSnapshot: isize,
        lppe: *mut PROCESSENTRY32W,
    ) -> i32; // BOOL

    fn Process32NextW(
        hSnapshot: isize,
        lppe: *mut PROCESSENTRY32W,
    ) -> i32; // BOOL

    fn CloseHandle(hObject: isize) -> i32; // BOOL

    fn OpenProcess(
        dwDesiredAccess: u32,
        bInheritHandle: i32,
        dwProcessId: u32,
    ) -> isize; // HANDLE

    fn QueryFullProcessImageNameW(
        hProcess: isize,
        dwFlags: u32,
        lpExeName: *mut u16,
        lpdwSize: *mut u32,
    ) -> i32; // BOOL
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

// ─── Public API ────────────────────────────────────────────────────────────

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

/// Find all running processes whose executable is under `apps_dir`.
///
/// On Unix this always returns an empty vec (returns [`Ok`]).
pub fn running_apps(apps_dir: &Path) -> Fallible<Vec<String>> {
    #[cfg(not(windows))]
    {
        let _ = apps_dir;
        Ok(vec![])
    }

    #[cfg(windows)]
    {
        running_apps_win(apps_dir)
    }
}

// ─── Windows implementation ────────────────────────────────────────────────

#[cfg(windows)]
fn running_apps_win(apps_dir: &Path) -> Fallible<Vec<String>> {
    let h_snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if h_snapshot == -1 || h_snapshot == 0 {
        // INVALID_HANDLE_VALUE is -1, NULL is also possible
        return Err(Error::Custom("CreateToolhelp32Snapshot failed".into()));
    }

    let mut names = Vec::new();
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
            let h_process =
                unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
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

                    // Check if exe is under the Scoop apps directory
                    if path.starts_with(apps_dir) {
                        // Extract the exe name without extension
                        if let Some(file_stem) = path.file_stem() {
                            names.push(file_stem.to_string_lossy().into_owned());
                        }
                    }
                }
            }
        }

        pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        ok = unsafe { Process32NextW(h_snapshot, &mut pe) };
    }

    unsafe { CloseHandle(h_snapshot) };

    names.sort();
    names.dedup();
    Ok(names)
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
