//! hok-shim — Shim launcher for hok package manager.
//!
//! Reads `{name}.shim` to find the real executable path, then launches it.
//! Handles GUI detection (FreeConsole), elevation (ShellExecuteW),
//! job object (KILL_ON_JOB_CLOSE), and Ctrl+C forwarding.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::process::Command;

// ── Windows FFI declarations ──────────────────────────────────────────

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        hwnd: isize, lpOperation: *const u16, lpFile: *const u16,
        lpParameters: *const u16, lpDirectory: *const u16, nShowCmd: i32,
    ) -> isize;
    fn SHGetFileInfoW(
        pszPath: *const u16, dwFileAttributes: u32,
        psfi: *mut u16, cbFileInfo: u32, uFlags: u32,
    ) -> usize;
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateJobObjectW(lpJobAttributes: *const u8, lpName: *const u16) -> isize;
    fn SetInformationJobObject(
        hJob: isize, JobObjectInfoClass: i32,
        lpJobObjectInfo: *const u8, cbJobObjectInfoLength: u32,
    ) -> i32;
    fn AssignProcessToJobObject(hJob: isize, hProcess: isize) -> i32;
    fn FreeConsole() -> i32;
    fn SetConsoleCtrlHandler(HandlerRoutine: isize, Add: i32) -> i32;
}

const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;
const SHGFI_EXETYPE: u32 = 0x2000;

// ── Main ──────────────────────────────────────────────────────────────

fn main() {
    let exe = env::current_exe().expect("current_exe");
    let shim_path = exe.with_extension("shim");

    let (target, extra) = match read_shim(&shim_path) {
        Some(v) => v,
        None => {
            eprintln!("shim: can't read {}", shim_path.display());
            std::process::exit(1);
        }
    };

    let cmd_args: Vec<String> = env::args().skip(1).collect();

    // Create job object to manage child process lifecycle
    let job = create_job();

    // Determine if target is a GUI app
    let is_gui = is_gui_app(&target);
    if is_gui {
        // Hide the terminal window for GUI apps
        unsafe { FreeConsole(); }
    }

    // Ignore Ctrl+C — let the child process handle it
    unsafe { SetConsoleCtrlHandler(-1, 1); }

    match Command::new(&target).args(&extra).args(&cmd_args).spawn() {
        Ok(mut child) => {
            // Assign child to job so it's killed when shim exits
            if let Some(job) = job {
                unsafe {
                    use std::os::windows::io::AsRawHandle;
                    AssignProcessToJobObject(job, child.as_raw_handle() as isize);
                }
            }

            let status = child.wait().expect("wait");
            std::process::exit(status.code().unwrap_or(1));
        }
        Err(e) if e.raw_os_error() == Some(740) => elevate(&target, &extra, &cmd_args),
        Err(e) => {
            eprintln!("shim: {target}: {e}");
            std::process::exit(1);
        }
    }
}

// ── .shim file parsing ────────────────────────────────────────────────

fn read_shim(p: &PathBuf) -> Option<(String, Vec<String>)> {
    let f = File::open(p).ok()?;
    let mut target = None;
    let mut args = Vec::new();
    for line in BufReader::new(f).lines() {
        let l = line.ok()?;
        if let Some(v) = l.strip_prefix("path = ") {
            target = Some(v.trim_matches('"').to_string());
        } else if let Some(v) = l.strip_prefix("args = ") {
            args = v.trim_matches('"').split_whitespace().map(String::from).collect();
        }
    }
    target.map(|t| (resolve(&t), args))
}

fn resolve(raw: &str) -> String {
    if let Some(suffix) = raw.strip_prefix("~\\..\\") {
        if let Ok(exe) = env::current_exe() {
            if let Some(parent) = exe.parent() {
                if let Some(root) = parent.parent() {
                    return root.join(suffix).to_string_lossy().to_string();
                }
            }
        }
    }
    raw.to_string()
}

// ── GUI detection via SHGetFileInfoW ──────────────────────────────────

fn is_gui_app(path: &str) -> bool {
    let wide: Vec<u16> = OsStr::new(path).encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        let ret = SHGetFileInfoW(wide.as_ptr(), 0, std::ptr::null_mut(), 0, SHGFI_EXETYPE);
        // SHGFI_EXETYPE returns nonzero with HIWORD != 0 for GUI apps
        ret != 0 && (ret & 0xFFFF0000) != 0
    }
}

// ── Job object ────────────────────────────────────────────────────────

fn create_job() -> Option<isize> {
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job == 0 || job == -1 {
            return None;
        }

        // Set KILL_ON_JOB_CLOSE so the child is terminated when shim exits
        #[repr(C, packed)]
        struct JobObjectExtendedLimitInformation {
            basic_limit: BasicLimitInformation,
            io_info: [u8; 72],
        }
        #[repr(C, packed)]
        struct BasicLimitInformation {
            quantum: i64,
            affinity: usize,
            limits: u32,
            flags: u32,
            active_process: usize,
            affinity2: usize,
            priority: u32,
            max_work: u32,
        }

        let mut info = JobObjectExtendedLimitInformation {
            basic_limit: BasicLimitInformation {
                quantum: 0, affinity: 0,
                limits: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                flags: 0, active_process: 0, affinity2: 0,
                priority: 0, max_work: 0,
            },
            io_info: [0u8; 72],
        };

        let r = SetInformationJobObject(
            job, JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
            &mut info as *mut _ as *mut u8,
            std::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
        );
        if r == 0 { None } else { Some(job) }
    }
}

// ── Elevation ─────────────────────────────────────────────────────────

fn elevate(target: &str, extra: &[String], cmd: &[String]) {
    let all_args: Vec<String> = extra.iter().cloned().chain(cmd.iter().cloned()).collect();
    let a_wide: Vec<u16> = all_args.join(" ").encode_utf16().chain(std::iter::once(0)).collect();
    let p_wide: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    let v_wide: Vec<u16> = OsStr::new("runas").encode_wide().chain(std::iter::once(0)).collect();

    unsafe {
        let r = ShellExecuteW(0, v_wide.as_ptr(), p_wide.as_ptr(), a_wide.as_ptr(), std::ptr::null(), 1);
        if r as isize <= 32 {
            eprintln!("shim: elevation failed");
            std::process::exit(1);
        }
    }
}
