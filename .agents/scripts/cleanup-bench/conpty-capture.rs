//! conpty-capture — run a command under a Windows pseudo console (ConPTY) and
//! write the VT stream a real terminal would receive to a file.
//!
//! Why a windows-subsystem binary: a console-subsystem host hands its own std
//! handles to the child, and the child then keeps using those pipes instead of
//! the pseudoconsole — so `is_terminal()` stays false and nothing is captured
//! (verified: the child reported `[Console]::IsOutputRedirected = True` and its
//! output leaked into the host's pipes). With no console of its own, the child
//! attaches to the pseudoconsole and `GetStdHandle` returns console handles,
//! which is how Windows Terminal hosts its shells.
//!
//! Build: rustc -O --edition 2024 conpty-capture.rs -o conpty-capture.exe
//! Run:   conpty-capture.exe <cols> <rows> <capture-out> <summary-out> <command...>

#![windows_subsystem = "windows"]

use std::ffi::c_void;
use std::fs;
use std::io::Write;
use std::ptr;
use std::sync::{Arc, Mutex};
use std::thread;

type Handle = *mut c_void;

/// Raw handles are not `Send`, so the reader thread gets this wrapper.
///
/// The reader calls [`SendHandle::raw`] instead of destructuring the field:
/// precise closure capture would otherwise capture the bare `*mut c_void`.
struct SendHandle(Handle);
unsafe impl Send for SendHandle {}

impl SendHandle {
    fn raw(&self) -> Handle {
        self.0
    }
}

#[repr(C)]
struct Coord {
    x: i16,
    y: i16,
}

#[repr(C)]
struct StartupInfoW {
    cb: u32,
    lp_reserved: *mut u16,
    lp_desktop: *mut u16,
    lp_title: *mut u16,
    dw_x: u32,
    dw_y: u32,
    dw_x_size: u32,
    dw_y_size: u32,
    dw_x_count_chars: u32,
    dw_y_count_chars: u32,
    dw_fill_attribute: u32,
    dw_flags: u32,
    w_show_window: u16,
    cb_reserved2: u16,
    lp_reserved2: *mut u8,
    h_std_input: Handle,
    h_std_output: Handle,
    h_std_error: Handle,
}

#[repr(C)]
struct StartupInfoExW {
    startup_info: StartupInfoW,
    lp_attribute_list: *mut c_void,
}

#[repr(C)]
struct ProcessInformation {
    h_process: Handle,
    h_thread: Handle,
    process_id: u32,
    thread_id: u32,
}

const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x0002_0016;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreatePseudoConsole(size: Coord, input: Handle, output: Handle, flags: u32, pc: *mut Handle) -> i32;
    fn ClosePseudoConsole(pc: Handle);
    fn CreatePipe(read: *mut Handle, write: *mut Handle, attrs: *mut c_void, size: u32) -> i32;
    fn InitializeProcThreadAttributeList(list: *mut c_void, count: u32, flags: u32, size: *mut usize) -> i32;
    fn UpdateProcThreadAttribute(
        list: *mut c_void,
        flags: u32,
        attr: usize,
        value: *mut c_void,
        size: usize,
        prev: *mut c_void,
        ret: *mut usize,
    ) -> i32;
    fn CreateProcessW(
        app: *const u16,
        cmd: *mut u16,
        pa: *mut c_void,
        ta: *mut c_void,
        inherit: i32,
        flags: u32,
        env: *mut c_void,
        cwd: *const u16,
        si: *mut StartupInfoExW,
        pi: *mut ProcessInformation,
    ) -> i32;
    fn ReadFile(h: Handle, buf: *mut u8, n: u32, read: *mut u32, ov: *mut c_void) -> i32;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    fn GetExitCodeProcess(h: Handle, code: *mut u32) -> i32;
    fn CloseHandle(h: Handle) -> i32;
}

fn run(cols: i16, rows: i16, cmd_line: &str, out_path: &str) -> Result<(i32, usize), String> {
    unsafe {
        let (mut in_read, mut in_write): (Handle, Handle) = (ptr::null_mut(), ptr::null_mut());
        let (mut out_read, mut out_write): (Handle, Handle) = (ptr::null_mut(), ptr::null_mut());
        if CreatePipe(&mut in_read, &mut in_write, ptr::null_mut(), 0) == 0 {
            return Err("CreatePipe(in) failed".to_owned());
        }
        if CreatePipe(&mut out_read, &mut out_write, ptr::null_mut(), 0) == 0 {
            return Err("CreatePipe(out) failed".to_owned());
        }

        let mut pc: Handle = ptr::null_mut();
        let hr = CreatePseudoConsole(Coord { x: cols, y: rows }, in_read, out_write, 0, &mut pc);
        if hr != 0 {
            return Err(format!("CreatePseudoConsole hr={hr:#x}"));
        }

        let mut size: usize = 0;
        InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size);
        let layout = std::alloc::Layout::from_size_align(size, 8).map_err(|e| e.to_string())?;
        let list = std::alloc::alloc(layout) as *mut c_void;
        if InitializeProcThreadAttributeList(list, 1, 0, &mut size) == 0 {
            return Err("InitializeProcThreadAttributeList failed".to_owned());
        }
        if UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
            pc,
            std::mem::size_of::<Handle>(),
            ptr::null_mut(),
            ptr::null_mut(),
        ) == 0
        {
            return Err("UpdateProcThreadAttribute failed".to_owned());
        }

        let mut si: StartupInfoExW = std::mem::zeroed();
        si.startup_info.cb = std::mem::size_of::<StartupInfoExW>() as u32;
        si.lp_attribute_list = list;

        // Which standard handles the child receives decides whether it sees the
        // pseudoconsole or the host's pipes (`CONPTY_MODE` exists to A/B that:
        // the host inherits its own std handles and CreateProcess would hand
        // them to the child, so the pseudoconsole must win).
        const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
        match std::env::var("CONPTY_MODE").as_deref() {
            Ok("null") => {
                si.startup_info.dw_flags |= STARTF_USESTDHANDLES;
                si.startup_info.h_std_input = ptr::null_mut();
                si.startup_info.h_std_output = ptr::null_mut();
                si.startup_info.h_std_error = ptr::null_mut();
            }
            Ok("invalid") => {
                si.startup_info.dw_flags |= STARTF_USESTDHANDLES;
                si.startup_info.h_std_input = usize::MAX as Handle;
                si.startup_info.h_std_output = usize::MAX as Handle;
                si.startup_info.h_std_error = usize::MAX as Handle;
            }
            _ => {}
        }

        let mut pi: ProcessInformation = std::mem::zeroed();
        let mut cmd: Vec<u16> = cmd_line.encode_utf16().chain([0]).collect();
        if CreateProcessW(
            ptr::null(),
            cmd.as_mut_ptr(),
            ptr::null_mut(),
            ptr::null_mut(),
            0,
            EXTENDED_STARTUPINFO_PRESENT,
            ptr::null_mut(),
            ptr::null(),
            &mut si,
            &mut pi,
        ) == 0
        {
            return Err(format!("CreateProcessW err {}", std::io::Error::last_os_error()));
        }
        CloseHandle(pi.h_thread);
        // The pseudoconsole keeps its own references to the pipe ends.
        CloseHandle(in_read);
        CloseHandle(out_write);

        let data = Arc::new(Mutex::new(Vec::<u8>::new()));
        let sink = Arc::clone(&data);
        let read_handle = SendHandle(out_read);
        let reader = thread::spawn(move || {
            let h = read_handle.raw();
            let mut buf = vec![0u8; 65536];
            loop {
                let mut read: u32 = 0;
                if ReadFile(h, buf.as_mut_ptr(), buf.len() as u32, &mut read, ptr::null_mut()) == 0
                    || read == 0
                {
                    break;
                }
                sink.lock().unwrap().extend_from_slice(&buf[..read as usize]);
            }
        });

        WaitForSingleObject(pi.h_process, 600_000);
        let mut code: u32 = 0;
        GetExitCodeProcess(pi.h_process, &mut code);
        CloseHandle(pi.h_process);
        // Let the reader drain, then drop the pseudoconsole to unblock it.
        thread::sleep(std::time::Duration::from_millis(300));
        ClosePseudoConsole(pc);
        let _ = reader.join();
        CloseHandle(out_read);
        CloseHandle(in_write);

        let bytes = data.lock().unwrap().clone();
        fs::write(out_path, &bytes).map_err(|e| e.to_string())?;
        Ok((code as i32, bytes.len()))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 6 {
        std::process::exit(2);
    }
    let cols: i16 = args[1].parse().unwrap_or(100);
    let rows: i16 = args[2].parse().unwrap_or(20);
    let out_path = args[3].clone();
    let summary_path = args[4].clone();
    let cmd_line = args[5..].join(" ");

    let line = match run(cols, rows, &cmd_line, &out_path) {
        Ok((code, bytes)) => format!("exit={code}\nbytes={bytes}\n"),
        Err(e) => format!("error={e}\n"),
    };
    if let Ok(mut f) = fs::File::create(&summary_path) {
        let _ = f.write_all(line.as_bytes());
    }
}
