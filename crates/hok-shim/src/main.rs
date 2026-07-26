//! hok-shim — Shim launcher for hok (no_std, no alloc, fixed buffers).
//!
//! Fully conformant with the Scoop Shim File Format specification:
//! https://github.com/ScoopInstaller/Shim
//!
//! Reads `{exe}.shim` → finds target path → launches with CreateProcessW.
//! Handles GUI detection, elevation, job object, Ctrl+C forwarding.

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(test, allow(dead_code))]

#[cfg(test)]
extern crate std;

// ── Panic handler ─────────────────────────────────────────────────────

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe { write_stderr(b"shim: panic\n"); }
    loop {}
}

// ── C runtime intrinsics ────────────────────────────────────────────

// Provide memcpy/memset/memcmp for compiler-generated calls.
// Duplicate with CRT symbols — resolved via /FORCE:MULTIPLE in debug.

#[cfg(not(test))]
#[no_mangle]
pub unsafe extern "C" fn memcpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    for i in 0..n { *dst.add(i) = *src.add(i); }
    dst
}
#[cfg(not(test))]
#[no_mangle]
pub unsafe extern "C" fn memset(dst: *mut u8, c: i32, n: usize) -> *mut u8 {
    for i in 0..n { *dst.add(i) = c as u8; }
    dst
}
#[cfg(not(test))]
#[no_mangle]
pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    for i in 0..n {
        let diff = (*a.add(i) as i32) - (*b.add(i) as i32);
        if diff != 0 { return diff; }
    }
    0
}

// ── FFI ───────────────────────────────────────────────────────────────

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleFileNameW(hModule: isize, lpFilename: *mut u16, nSize: u32) -> u32;
    fn GetCommandLineW() -> *mut u16;
    fn GetFullPathNameW(lpFileName: *const u16, nBufferLength: u32, lpBuffer: *mut u16, lpFilePart: *mut *mut u16) -> u32;
    fn ExpandEnvironmentStringsW(lpSrc: *const u16, lpDst: *mut u16, nSize: u32) -> u32;
    fn CreateFileW(lpFileName: *const u16, dwDesiredAccess: u32, dwShareMode: u32, lpSecurityAttributes: *const u8, dwCreationDisposition: u32, dwFlagsAndAttributes: u32, hTemplateFile: isize) -> isize;
    fn ReadFile(hFile: isize, lpBuffer: *mut u8, nNumberOfBytesToRead: u32, lpNumberOfBytesRead: *mut u32, lpOverlapped: *const u8) -> i32;
    fn CloseHandle(hObject: isize) -> i32;
    fn SetEnvironmentVariableW(lpName: *const u16, lpValue: *const u16) -> i32;
    fn CreateProcessW(lpApplicationName: *const u16, lpCommandLine: *mut u16, lpProcessAttributes: *const u8, lpThreadAttributes: *const u8, bInheritHandles: i32, dwCreationFlags: u32, lpEnvironment: *const u8, lpCurrentDirectory: *const u16, lpStartupInfo: *mut u8, lpProcessInformation: *mut u8) -> i32;
    fn WaitForSingleObject(hHandle: isize, dwMilliseconds: u32) -> u32;
    fn GetExitCodeProcess(hProcess: isize, lpExitCode: *mut u32) -> i32;
    fn CreateJobObjectW(lpJobAttributes: *const u8, lpName: *const u16) -> isize;
    fn SetInformationJobObject(hJob: isize, JobObjectInfoClass: i32, lpJobObjectInfo: *const u8, cbJobObjectInfoLength: u32) -> i32;
    fn AssignProcessToJobObject(hJob: isize, hProcess: isize) -> i32;
    fn FreeConsole() -> i32;
    fn AttachConsole(dwProcessId: u32) -> i32;
    fn SetConsoleCtrlHandler(HandlerRoutine: isize, Add: i32) -> i32;
    fn GetStdHandle(nStdHandle: u32) -> isize;
    fn WriteFile(hFile: isize, lpBuffer: *const u8, nNumberOfBytesToWrite: u32, lpNumberOfBytesWritten: *mut u32, lpOverlapped: *const u8) -> i32;
    fn ResumeThread(hThread: isize) -> u32;
    fn GetLastError() -> u32;
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteExW(lpExecInfo: *mut u8) -> i32;
}

// ── Constants ──────────────────────────────────────────────────────────

const MAX_U16: usize = 512;
const EVARS_MAX: usize = 8;
const FILE_BUF: usize = 4096;
const GENERIC_READ: u32 = 0x8000_0000;
const FILE_SHARE_READ: u32 = 1;
const OPEN_EXISTING: u32 = 3;
const INVALID_HANDLE: isize = -1;
const NULL_HANDLE: isize = 0;
const ERROR_ELEVATION_REQUIRED: u32 = 740;
const STD_ERROR_HANDLE: u32 = 0xFFFF_FFF4;
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;
const CREATE_SUSPENDED: u32 = 4;
const INFINITE: u32 = 0xFFFF_FFFF;
const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
const SEE_MASK_NOCLOSEPROCESS: u32 = 0x0000_0040;

// ── U16Buf: fixed-size UTF-16 string buffer ──────────────────────────

#[derive(Copy, Clone)]
struct U16Buf {
    buf: [u16; MAX_U16],
    len: usize,
}

impl U16Buf {
    fn new() -> Self { U16Buf { buf: [0; MAX_U16], len: 0 } }
    fn ptr(&self) -> *const u16 { self.buf.as_ptr() }
    fn mut_ptr(&mut self) -> *mut u16 { self.buf.as_mut_ptr() }
    fn slice(&self) -> &[u16] { unsafe { core::slice::from_raw_parts(self.ptr(), self.len) } }

    fn from_slice(s: &[u16]) -> Self {
        let n = s.len().min(MAX_U16 - 1);
        let mut b = Self::new();
        unsafe { core::ptr::copy_nonoverlapping(s.as_ptr(), b.mut_ptr(), n); }
        b.buf[n] = 0; b.len = n; b
    }

    fn set_utf8(&mut self, s: &[u8]) {
        let utf8 = core::str::from_utf8(s).unwrap_or("");
        let mut i = 0;
        for c in utf8.encode_utf16() {
            if i >= MAX_U16 - 1 { break; }
            self.buf[i] = c; i += 1;
        }
        self.buf[i] = 0; self.len = i;
    }
}

// ── Low-level helpers ─────────────────────────────────────────────────

unsafe fn write_stderr(msg: &[u8]) {
    let h = GetStdHandle(STD_ERROR_HANDLE);
    if h != NULL_HANDLE && h != INVALID_HANDLE {
        let mut written: u32 = 0;
        WriteFile(h, msg.as_ptr(), msg.len() as u32, &mut written, core::ptr::null());
    }
}

unsafe fn u16_strlen(p: *const u16, max: usize) -> usize {
    let mut i = 0;
    while i < max && *p.add(i) != 0 { i += 1; }
    i
}

unsafe fn u16_to_buf(ptr: *const u16) -> U16Buf {
    let len = u16_strlen(ptr, MAX_U16 - 1);
    let mut b = U16Buf::new();
    core::ptr::copy_nonoverlapping(ptr, b.mut_ptr(), len);
    b.buf[len] = 0; b.len = len; b
}

/// Skip argv[0] from a command line string. Returns everything after the
/// first argument (argv[0] can be quoted or unquoted).
unsafe fn skip_argv0(cmd: *mut u16) -> U16Buf {
    let mut p = 0usize;
    if *cmd.add(p) == b'"' as u16 {
        p += 1;
        while *cmd.add(p) != 0 && *cmd.add(p) != b'"' as u16 { p += 1; }
        if *cmd.add(p) == b'"' as u16 { p += 1; }
    } else {
        while *cmd.add(p) != 0 && *cmd.add(p) != b' ' as u16 { p += 1; }
    }
    while *cmd.add(p) == b' ' as u16 { p += 1; }
    u16_to_buf(cmd.add(p))
}

/// Case-insensitive comparison of two ASCII byte slices.
pub(crate) fn u16_ieq(a: &[u8], b: &[u8]) -> bool {
    // Compare ASCII/UTF-8 byte slices case-insensitively.
    // Only used for field names which are ASCII.
    if a.len() != b.len() { return false; }
    for i in 0..a.len() {
        let ca = if a[i] >= b'A' && a[i] <= b'Z' { a[i] + 32 } else { a[i] };
        let cb = if b[i] >= b'A' && b[i] <= b'Z' { b[i] + 32 } else { b[i] };
        if ca != cb { return false; }
    }
    true
}

/// Check if a byte slice matches any in a list of known field name patterns.
pub(crate) fn is_known_field(key: &[u8]) -> bool {
    u16_ieq(key, b"path") || u16_ieq(key, b"args") || u16_ieq(key, b"cwd")
        || u16_ieq(key, b"workdir") || u16_ieq(key, b"elevate") || u16_ieq(key, b"runas")
}

// ── .shim file I/O ────────────────────────────────────────────────────

unsafe fn read_shim_file(path: &U16Buf) -> ([u8; FILE_BUF], usize) {
    let mut buf = [0u8; FILE_BUF];
    let h = CreateFileW(path.ptr(), GENERIC_READ, FILE_SHARE_READ, core::ptr::null(), OPEN_EXISTING, 0, NULL_HANDLE);
    if h == INVALID_HANDLE || h == NULL_HANDLE { return (buf, 0); }
    let mut read: u32 = 0;
    ReadFile(h, buf.as_mut_ptr(), FILE_BUF as u32, &mut read, core::ptr::null());
    let n = read as usize;
    CloseHandle(h);
    (buf, n)
}

// ── UTF-8 parser for .shim content ──────────────────────────────────

pub(crate) fn next_line(content: &[u8], start: usize) -> Option<(&[u8], usize)> {
    if start >= content.len() { return None; }
    for i in start..content.len() {
        if content[i] == b'\n' { return Some((&content[start..i], i + 1)); }
    }
    Some((&content[start..], content.len()))
}

pub(crate) fn trim_spaces(s: &[u8]) -> &[u8] {
    let mut a = 0;
    while a < s.len() && (s[a] == b' ' || s[a] == b'\t') { a += 1; }
    let mut b = s.len();
    while b > a && (s[b-1] == b' ' || s[b-1] == b'\t' || s[b-1] == b'\r') { b -= 1; }
    &s[a..b]
}

pub(crate) fn parse_key_value(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let trimmed = trim_spaces(line);
    if trimmed.is_empty() { return None; }
    // Comments: # ; //
    if trimmed[0] == b'#' || trimmed[0] == b';' { return None; }
    if trimmed.len() >= 2 && trimmed[0] == b'/' && trimmed[1] == b'/' { return None; }

    let eq_pos = trimmed.windows(3).position(|w| w == b" = ")?;
    let key = trim_spaces(&trimmed[..eq_pos]);
    let raw_val = trim_spaces(&trimmed[eq_pos + 3..]);

    let val = if raw_val.len() >= 2 && raw_val[0] == b'"' && raw_val[raw_val.len()-1] == b'"' {
        &raw_val[1..raw_val.len()-1]
    } else { raw_val };

    Some((key, val))
}

// ── Env var expansion ────────────────────────────────────────────────

/// Expand `%ENV%` using ExpandEnvironmentStringsW.
unsafe fn expand_env(src: &U16Buf) -> U16Buf {
    let required = ExpandEnvironmentStringsW(src.ptr(), core::ptr::null_mut(), 0);
    if required == 0 || required as usize > MAX_U16 { return *src; }
    let mut buf = [0u16; MAX_U16];
    let actual = ExpandEnvironmentStringsW(src.ptr(), buf.as_mut_ptr(), required);
    if actual == 0 { return *src; }
    u16_to_buf(buf.as_ptr())
}

/// Replace all `%~dp0` with target exe directory (trailing backslash).
pub(crate) fn expand_dp0(val: &mut U16Buf, target_dir: &U16Buf) {
    let dp0: &[u16] = &[b'%' as u16, b'~' as u16, b'd' as u16, b'p' as u16, b'0' as u16];
    let td = target_dir.slice();
    loop {
        let slice = val.slice();
        if slice.len() < 5 { break; }
        let mut found = None;
        for i in 0..=slice.len() - 5 {
            if &slice[i..i+5] == dp0 { found = Some(i); break; }
        }
        let Some(pos) = found else { break };
        let mut buf = [0u16; MAX_U16];
        let mut out = 0usize;
        for &c in &slice[..pos] { if out >= MAX_U16 - 2 { break; } buf[out] = c; out += 1; }
        for &c in td.iter() { if out >= MAX_U16 - 2 { break; } buf[out] = c; out += 1; }
        for &c in &slice[pos+5..] { if out >= MAX_U16 - 2 { break; } buf[out] = c; out += 1; }
        buf[out] = 0;
        val.buf = buf; val.len = out;
    }
}

// ── Path resolution ───────────────────────────────────────────────────

unsafe fn get_shim_dir() -> U16Buf {
    let mut buf = [0u16; MAX_U16];
    let len = GetModuleFileNameW(NULL_HANDLE, buf.as_mut_ptr(), MAX_U16 as u32);
    if len == 0 || len as usize >= MAX_U16 { return U16Buf::new(); }
    let mut last_sep = 0usize;
    for i in 0..len as usize { if buf[i] == b'\\' as u16 || buf[i] == b'/' as u16 { last_sep = i; } }
    U16Buf::from_slice(&buf[..last_sep])
}

unsafe fn get_exe_stem() -> U16Buf {
    let mut buf = [0u16; MAX_U16];
    let len = GetModuleFileNameW(NULL_HANDLE, buf.as_mut_ptr(), MAX_U16 as u32);
    if len == 0 || len as usize >= MAX_U16 { return U16Buf::new(); }
    let mut file_start = 0usize;
    for i in 0..len as usize { if buf[i] == b'\\' as u16 || buf[i] == b'/' as u16 { file_start = i + 1; } }
    let mut ext_end = len as usize;
    for i in (file_start..len as usize).rev() { if buf[i] == b'.' as u16 { ext_end = i; break; } }
    U16Buf::from_slice(&buf[file_start..ext_end])
}

unsafe fn get_shim_path(dir: &U16Buf, stem: &U16Buf) -> U16Buf {
    let mut buf = [0u16; MAX_U16]; let mut pos = 0;
    for &c in dir.slice().iter() { if pos < MAX_U16 - 6 { buf[pos] = c; pos += 1; } }
    buf[pos] = b'\\' as u16; pos += 1;
    for &c in stem.slice().iter() { if pos < MAX_U16 - 6 { buf[pos] = c; pos += 1; } }
    let ext = [b'.' as u16, b's' as u16, b'h' as u16, b'i' as u16, b'm' as u16, 0];
    for &c in ext.iter() { if pos < MAX_U16 { buf[pos] = c; pos += 1; } }
    buf[pos-1] = 0; U16Buf::from_slice(&buf[..pos-1])
}

/// Get the directory portion of a resolved target path (for `%~dp0` expansion).
pub(crate) fn target_dir_of(resolved: &U16Buf) -> U16Buf {
    let slice = resolved.slice();
    let mut last_sep = 0usize;
    for i in 0..slice.len() {
        if slice[i] == b'\\' as u16 || slice[i] == b'/' as u16 { last_sep = i + 1; }
    }
    U16Buf::from_slice(&slice[..last_sep])
}

unsafe fn resolve_target(raw: &U16Buf, shim_dir: &U16Buf) -> U16Buf {
    let raw_slice = raw.slice();
    if raw_slice.len() < 5 { return *raw; }
    let prefix: &[u16] = &[b'~' as u16, b'\\' as u16, b'.' as u16, b'.' as u16, b'\\' as u16];
    if &raw_slice[..5] == prefix {
        let mut buf = [0u16; MAX_U16]; let mut pos = 0;
        for &c in shim_dir.slice().iter() { if pos < MAX_U16 - 3 { buf[pos] = c; pos += 1; } }
        for &c in [b'\\' as u16, b'.' as u16, b'.' as u16, b'\\' as u16].iter() { buf[pos] = c; pos += 1; }
        for &c in raw_slice[5..].iter() { if pos < MAX_U16 - 2 { buf[pos] = c; pos += 1; } }
        buf[pos] = 0; return U16Buf::from_slice(&buf[..pos]);
    }
    let mut resolved = [0u16; MAX_U16];
    let mut file_part: *mut u16 = core::ptr::null_mut();
    let len = GetFullPathNameW(raw.ptr(), MAX_U16 as u32, resolved.as_mut_ptr(), &mut file_part);
    if len > 0 && (len as usize) < MAX_U16 { return u16_to_buf(resolved.as_ptr()); }
    *raw
}

// ── GUI detection via PE header (no shell32 dependency) ────────────

/// Check if the target executable has GUI subsystem by reading its PE header.
/// Reads the IMAGE_OPTIONAL_HEADER->Subsystem field: 2 = GUI, 3 = Console.
unsafe fn is_gui_app(path: &U16Buf) -> bool {
    let h = CreateFileW(path.ptr(), GENERIC_READ, FILE_SHARE_READ, core::ptr::null(), OPEN_EXISTING, 0, NULL_HANDLE);
    if h == INVALID_HANDLE || h == NULL_HANDLE { return false; }

    let mut buf = [0u8; 512];
    let mut read: u32 = 0;
    ReadFile(h, buf.as_mut_ptr(), 512, &mut read, core::ptr::null());
    CloseHandle(h);

    if read < 64 { return false; }
    // Check MZ signature
    if buf[0] != b'M' || buf[1] != b'Z' { return false; }
    // PE offset at 0x3C
    let pe_off = *(buf.as_ptr().add(0x3C) as *const u32) as usize;
    if pe_off as u32 > read - 0x5E { return false; }
    // Check PE signature
    if buf[pe_off] != b'P' || buf[pe_off + 1] != b'E' { return false; }
    // Subsystem is at PE offset + 0x5C
    // Structure: PE sig(4) + COFF hdr(20) + OptionalHdr(0x44 to subsystem field)
    // = 4 + 20 + 0x44 = 0x5C from PE start
    let subsystem = core::ptr::read_unaligned(buf.as_ptr().add(pe_off + 0x5C) as *const u16);
    subsystem == 2 // IMAGE_SUBSYSTEM_WINDOWS_GUI
}

// ── Job object ────────────────────────────────────────────────────────

unsafe fn create_job() -> isize {
    let job = CreateJobObjectW(core::ptr::null(), core::ptr::null());
    if job == NULL_HANDLE || job == INVALID_HANDLE { return NULL_HANDLE; }
    // JOBOBJECT_EXTENDED_LIMIT_INFORMATION on x64 = 0x60 bytes
    // BasicLimitInformation.LimitFlags offset: 0x10 (x64) / 0x0C (x86)
    #[cfg(target_pointer_width = "64")]
    const LF_OFF: usize = 0x10;
    #[cfg(target_pointer_width = "32")]
    const LF_OFF: usize = 0x0C;
    let mut info = [0u8; 0x60];
    *(info.as_mut_ptr().add(LF_OFF) as *mut u32) = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let r = SetInformationJobObject(job, JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
        info.as_ptr(), info.len() as u32);
    if r == 0 { NULL_HANDLE } else { job }
}

// ── Command line building ─────────────────────────────────────────────

/// Build `{quoted_path} {shim_args} {user_tail}` with proper quoting.
unsafe fn build_cmdline(path: &U16Buf, shim_args: &U16Buf, user_tail: &U16Buf, out: &mut [u16; MAX_U16 * 4]) -> usize {
    let mut pos = 0usize;
    let max = MAX_U16 * 4 - 4;

    // Quote path
    let ps = path.slice();
    let needs_q = ps.iter().any(|&c| c == b' ' as u16);
    if needs_q { out[pos] = b'"' as u16; pos += 1; }
    for &c in ps.iter() {
        if pos >= max { break; }
        if c == b'"' as u16 { out[pos] = b'\\' as u16; pos += 1; }
        out[pos] = c; pos += 1;
    }
    if needs_q { out[pos] = b'"' as u16; pos += 1; }

    // Shim args
    let a_slice = shim_args.slice();
    if a_slice.len() > 0 && a_slice[0] != 0 {
        out[pos] = b' ' as u16; pos += 1;
        for &c in a_slice.iter() {
            if pos >= max { break; } if c == 0 { break; }
            out[pos] = c; pos += 1;
        }
    }

    // User args (tail after argv[0], appended as-is)
    let ut = user_tail.slice();
    if ut.len() > 0 && ut[0] != 0 {
        out[pos] = b' ' as u16; pos += 1;
        for &c in ut.iter() {
            if pos >= max { break; }
            if c == 0 { break; }
            out[pos] = c; pos += 1;
        }
    }

    out[pos] = 0; pos
}

// ── Process creation ──────────────────────────────────────────────────

unsafe fn spawn(path: &U16Buf, args: &U16Buf, user_tail: &U16Buf, cwd: &U16Buf, elevate_req: bool) -> (isize, bool) {
    let mut cmdline_buf = [0u16; MAX_U16 * 4];
    build_cmdline(path, args, user_tail, &mut cmdline_buf);

    if elevate_req { return (NULL_HANDLE, true); }

    let mut si: [u8; 104] = [0u8; 104];
    *(si.as_mut_ptr() as *mut u32) = 104;
    let mut pi: [u8; 24] = [0u8; 24];

    // Build a mutable command line for CreateProcessW
    let mut cmd_mut = cmdline_buf;

    let cwd_ptr = if cwd.len > 0 { cwd.ptr() } else { core::ptr::null() };

    let r = CreateProcessW(
        core::ptr::null(), cmd_mut.as_mut_ptr(),
        core::ptr::null(), core::ptr::null(), 1,
        CREATE_SUSPENDED, core::ptr::null(), cwd_ptr,
        si.as_mut_ptr(), pi.as_mut_ptr(),
    );

    if r != 0 {
        let proc_h = *(pi.as_ptr() as *const isize);
        let thread_h = *(pi.as_ptr() as *const isize).add(1);
        ResumeThread(thread_h);
        return (proc_h, false);
    }

    if GetLastError() == ERROR_ELEVATION_REQUIRED {
        return (NULL_HANDLE, true);
    }

    (NULL_HANDLE, false)
}

/// Launch with elevation via ShellExecuteExW and wait for exit.
/// Returns the child's exit code.
unsafe fn do_elevate(path: &U16Buf, args: &U16Buf, user_tail: &U16Buf, cwd: &U16Buf) -> u32 {
    let verb: &[u16] = &[b'r' as u16, b'u' as u16, b'n' as u16, b'a' as u16, b's' as u16, 0];
    let verb_buf = U16Buf::from_slice(verb);
    let mut cmdline = [0u16; MAX_U16 * 4];
    build_cmdline(path, args, user_tail, &mut cmdline);
    let cwd_ptr = if cwd.len > 0 { cwd.ptr() } else { core::ptr::null() };

    // SHELLEXECUTEINFOW (104 bytes on x64)
    #[repr(C)]
    struct SEI { cbSize: u32, fMask: u32, hwnd: isize, lpVerb: *const u16,
                 lpFile: *const u16, lpParameters: *const u16, lpDirectory: *const u16,
                 nShow: i32, hInstApp: isize, lpIDList: isize, lpClass: *const u16,
                 hkeyClass: isize, dwHotKey: u32, hMonitor: isize, hProcess: isize, }

    let mut sei = SEI {
        cbSize: core::mem::size_of::<SEI>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        hwnd: 0, lpVerb: verb_buf.ptr(), lpFile: path.ptr(),
        lpParameters: cmdline.as_ptr(), lpDirectory: cwd_ptr, nShow: 1,
        hInstApp: 0, lpIDList: 0, lpClass: core::ptr::null(),
        hkeyClass: 0, dwHotKey: 0, hMonitor: 0, hProcess: 0,
    };

    if ShellExecuteExW(&mut sei as *mut _ as *mut u8) == 0 {
            return 1;
    }

    let proc_h = sei.hProcess;
    if proc_h == NULL_HANDLE || proc_h == INVALID_HANDLE { return 1; }

    WaitForSingleObject(proc_h, INFINITE);
    let mut exit_code: u32 = 1;
    GetExitCodeProcess(proc_h, &mut exit_code);
    CloseHandle(proc_h);
    exit_code
}

// ── Entry ─────────────────────────────────────────────────────────────

/// Ignore Ctrl+C/Ctrl+Break so the child process receives the signal.
/// A real handler (not NULL) is required — NULL tells Windows to REMOVE
/// all handlers, which can cause process termination on Ctrl+C.
unsafe extern "system" fn ignore_ctrl_c(_: u32) -> i32 { 1 }

unsafe fn entry() -> i32 {
    // Swallow Ctrl+C — the child inherits the console and handles it
    SetConsoleCtrlHandler(ignore_ctrl_c as isize, 1);

    // ── Parse .shim file ───────────────────────────────────────────
    let shim_dir = get_shim_dir();
    let exe_stem = get_exe_stem();
    let shim_path = get_shim_path(&shim_dir, &exe_stem);

    let (content, content_len) = read_shim_file(&shim_path);
    if content_len == 0 { write_stderr(b"shim: cannot read .shim file\n"); return 1; }

    let mut content_slice = &content[..content_len];

    // Strip UTF-8 BOM (\xEF\xBB\xBF) if present
    if content_slice.len() >= 3 && content_slice[0] == 0xEF && content_slice[1] == 0xBB && content_slice[2] == 0xBF {
        content_slice = &content_slice[3..];
    }

    let mut target = U16Buf::new();
    let mut args_u16 = U16Buf::new();
    let mut cwd_u16 = U16Buf::new();
    let mut elevate = false;
    let mut env_names: [U16Buf; EVARS_MAX] = [U16Buf::new(); EVARS_MAX];
    let mut env_vals: [U16Buf; EVARS_MAX] = [U16Buf::new(); EVARS_MAX];
    let mut env_count = 0usize;
    let mut has_target = false;

    let mut offset = 0usize;
    while let Some((line, next)) = next_line(content_slice, offset) {
        if let Some((key, val)) = parse_key_value(line) {
            if u16_ieq(key, b"path") {
                target.set_utf8(val);
                has_target = true;
            } else if u16_ieq(key, b"args") {
                args_u16.set_utf8(val);
            } else if u16_ieq(key, b"cwd") || u16_ieq(key, b"workdir") {
                cwd_u16.set_utf8(val);
            } else if u16_ieq(key, b"elevate") || u16_ieq(key, b"runas") {
                // Only true/1/yes means elevate
                if val.len() == 1 && val[0] == b'1' { elevate = true; }
                else if u16_ieq(val, b"true") || u16_ieq(val, b"yes") { elevate = true; }
            } else if env_count < EVARS_MAX && !is_known_field(key) {
                // Environment variable override (case-insensitive key)
                env_names[env_count].set_utf8(key);
                env_vals[env_count].set_utf8(val);
                env_count += 1;
            }
        }
        offset = next;
    }

    if !has_target { write_stderr(b"shim: no 'path' in .shim file\n"); return 1; }

    let resolved = resolve_target(&target, &shim_dir);
    let target_dir = target_dir_of(&resolved);

    // Expand env vars in path, args, cwd
    let target_exp = expand_env(&resolved);
    let mut args_exp = expand_env(&args_u16);
    let mut cwd_exp = U16Buf::new();

    // Expand %~dp0 in args and cwd (NOT in path, per spec)
    expand_dp0(&mut args_exp, &target_dir);

    if cwd_u16.len > 0 {
        cwd_exp = expand_env(&cwd_u16);
        expand_dp0(&mut cwd_exp, &target_dir);
    }

    // Expand env vars in env override values
    let mut env_vals_exp: [U16Buf; EVARS_MAX] = [U16Buf::new(); EVARS_MAX];
    for i in 0..env_count {
        env_vals_exp[i] = expand_env(&env_vals[i]);
    }

    // ── Get user command-line arguments ──────────────────────────
    // Skip argv[0] (shim's own path); return everything else as-is.
    let user_tail = skip_argv0(GetCommandLineW());
    let has_user_args = user_tail.len > 0;

    // ── Set environment variable overrides ──────────────────────
    for i in 0..env_count {
        SetEnvironmentVariableW(env_names[i].ptr(), env_vals_exp[i].ptr());
    }

    // ── GUI detection + console handling ──────────────────────────
    if is_gui_app(&target_exp) {
        // GUI app: hide console unless there are args (user wants terminal output)
        if has_user_args || args_exp.len > 0 {
            AttachConsole(ATTACH_PARENT_PROCESS);
        } else {
            FreeConsole();
        }
    } else {
        // Console app: attach to parent console so child inherits it.
        // Without this, since shim is a GUI binary, Windows allocates a
        // new console for the child (costly ~400ms).
        AttachConsole(ATTACH_PARENT_PROCESS);
    }

    // ── Job object ─────────────────────────────────────────────
    let job = create_job();

    // ── Launch ─────────────────────────────────────────────────
    let (proc_h, needs_elevate) = spawn(&target_exp, &args_exp, &user_tail, &cwd_exp, elevate);
    if needs_elevate {
        return do_elevate(&target_exp, &args_exp, &user_tail, &cwd_exp) as i32;
    }
    if proc_h == NULL_HANDLE { return 1; }

    if job != NULL_HANDLE { AssignProcessToJobObject(job, proc_h); }

    WaitForSingleObject(proc_h, INFINITE);
    let mut exit_code: u32 = 1;
    GetExitCodeProcess(proc_h, &mut exit_code);
    CloseHandle(proc_h);
    if job != NULL_HANDLE { CloseHandle(job); }
    exit_code as i32
}

// ── Exported entry points ─────────────────────────────────────────────

#[cfg(not(test))]
#[no_mangle]
pub extern "system" fn WinMain() -> ! {
    let code = unsafe { entry() };
    exit_process(code)
}

#[cfg(not(test))]
#[no_mangle]
pub extern "system" fn mainCRTStartup() -> ! {
    let code = unsafe { entry() };
    exit_process(code)
}

#[cfg(not(test))]
fn exit_process(code: i32) -> ! {
    unsafe {
        extern "system" { fn ExitProcess(uExitCode: u32); }
        ExitProcess(code as u32);
        loop {}
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::*;
    use super::*;

    #[test]
    fn test_ieq_equal() { assert!(u16_ieq(b"hello", b"hello")); }
    #[test]
    fn test_ieq_case() { assert!(u16_ieq(b"PATH", b"path")); }
    #[test]
    fn test_ieq_mixed_case() { assert!(u16_ieq(b"RunAs", b"runas")); }
    #[test]
    fn test_ieq_diff_len() { assert!(!u16_ieq(b"path", b"paths")); }
    #[test]
    fn test_ieq_different() { assert!(!u16_ieq(b"path", b"args")); }

    #[test]
    fn test_known_path() { assert!(is_known_field(b"path")); }
    #[test]
    fn test_known_args() { assert!(is_known_field(b"args")); }
    #[test]
    fn test_known_cwd() { assert!(is_known_field(b"cwd")); }
    #[test]
    fn test_known_workdir() { assert!(is_known_field(b"workdir")); }
    #[test]
    fn test_known_elevate() { assert!(is_known_field(b"elevate")); }
    #[test]
    fn test_known_runas() { assert!(is_known_field(b"runas")); }
    #[test]
    fn test_known_case() { assert!(is_known_field(b"PATH")); }
    #[test]
    fn test_known_unknown() { assert!(!is_known_field(b"custom")); }

    #[test]
    fn test_trim_none() { assert_eq!(trim_spaces(b"hello"), b"hello"); }
    #[test]
    fn test_trim_leading() { assert_eq!(trim_spaces(b"  hello"), b"hello"); }
    #[test]
    fn test_trim_trailing() { assert_eq!(trim_spaces(b"hello  "), b"hello"); }
    #[test]
    fn test_trim_tab() { assert_eq!(trim_spaces(b"\thello\t"), b"hello"); }
    #[test]
    fn test_trim_cr() { assert_eq!(trim_spaces(b"hello\r"), b"hello"); }
    #[test]
    fn test_trim_empty() { assert_eq!(trim_spaces(b""), b""); }
    #[test]
    fn test_trim_spaces_only() { assert_eq!(trim_spaces(b"   "), b""); }

    #[test]
    fn test_next_line_simple() {
        let content = b"line1\nline2\n";
        let (l, n) = next_line(content, 0).unwrap();
        assert_eq!(l, b"line1"); assert_eq!(n, 6);
        let (l, n) = next_line(content, n).unwrap();
        assert_eq!(l, b"line2"); assert_eq!(n, 12);
        assert!(next_line(content, n).is_none());
    }
    #[test]
    fn test_next_line_no_trailing_nl() {
        let content = b"line1\nline2";
        let (l, _) = next_line(content, 0).unwrap();
        assert_eq!(l, b"line1");
        let (l, _) = next_line(content, 6).unwrap();
        assert_eq!(l, b"line2");
    }
    #[test]
    fn test_next_line_empty() { assert!(next_line(b"", 0).is_none()); }

    #[test]
    fn test_parse_path() {
        let (k, v) = parse_key_value(b"path = C:\\test.exe").unwrap();
        assert_eq!(k, b"path"); assert_eq!(v, b"C:\\test.exe");
    }
    #[test]
    fn test_parse_quoted() {
        let (k, v) = parse_key_value(b"path = \"C:\\Program Files\\app.exe\"").unwrap();
        assert_eq!(k, b"path"); assert_eq!(v, b"C:\\Program Files\\app.exe");
    }
    #[test]
    fn test_parse_args() {
        let (k, v) = parse_key_value(b"args = --verbose --all").unwrap();
        assert_eq!(k, b"args"); assert_eq!(v, b"--verbose --all");
    }
    #[test]
    fn test_parse_comment_hash() { assert!(parse_key_value(b"# this is a comment").is_none()); }
    #[test]
    fn test_parse_comment_semicolon() { assert!(parse_key_value(b"; comment").is_none()); }
    #[test]
    fn test_parse_comment_slash() { assert!(parse_key_value(b"// comment").is_none()); }
    #[test]
    fn test_parse_empty() { assert!(parse_key_value(b"  ").is_none()); }
    #[test]
    fn test_parse_no_equals() { assert!(parse_key_value(b"invalid").is_none()); }
    #[test]
    fn test_parse_extra_spaces() {
        let (k, v) = parse_key_value(b"  path  =  \"value\"  ").unwrap();
        assert_eq!(k, b"path"); assert_eq!(v, b"value");
    }

    #[test]
    fn test_target_dir_simple() {
        let p = U16Buf::from_slice(&utf16("C:\\apps\\7zip\\current\\7z.exe"));
        let d = target_dir_of(&p);
        assert_eq!(u16_to_string(&d), "C:\\apps\\7zip\\current\\");
    }
    #[test]
    fn test_target_dir_root() {
        let p = U16Buf::from_slice(&utf16("C:\\test.exe"));
        let d = target_dir_of(&p);
        assert_eq!(u16_to_string(&d), "C:\\");
    }

    #[test]
    fn test_expand_dp0_in_args() {
        let mut val = U16Buf::from_slice(&utf16(r#""%~dp0sub\file""#));
        let target_dir = U16Buf::from_slice(&utf16(r"C:\apps\7zip\current\"));
        expand_dp0(&mut val, &target_dir);
        let result = u16_to_string(&val);
        assert_eq!(result, r#""C:\apps\7zip\current\sub\file""#);
    }
    #[test]
    fn test_expand_dp0_no_match() {
        let mut val = U16Buf::from_slice(&utf16(r"--flag value"));
        let target_dir = U16Buf::from_slice(&utf16(r"C:\dir\"));
        expand_dp0(&mut val, &target_dir);
        assert_eq!(u16_to_string(&val), "--flag value");
    }

    fn utf16(s: &str) -> Vec<u16> { s.encode_utf16().collect() }
    fn u16_to_string(b: &U16Buf) -> String {
        let slice = b.slice();
        String::from_utf16_lossy(slice)
    }
}

