//! Shell-open utilities (Windows FFI) for the Hok binary.
//!
//! Provides [`open_url()`] and [`open_file()`] for opening URLs in the
//! default browser and files/directories with the system default handler,
//! implemented via `ShellExecuteW` on Windows.
//!
//! # Design
//!
//! - **Windows-only**: These functions are conditionally compiled with
//!   `#[cfg(windows)]` and use raw Win32 FFI.
//! - **Shared backend**: Both `open_url` and `open_file` delegate to a
//!   common [`shell_open()`] function that handles UTF-16 encoding and
//!   `ShellExecuteW` invocation.
//! - **Minimal dependency**: Uses `libscoop::os::encode_wide`
//!   for UTF-16 conversion rather than pulling in a separate crate.

use libscoop::os::encode_wide;
use std::path::Path;

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

// ─── Human-readable size ────────────────────────────────────────────────────

/// Convert bytes to KB/MB/GB representation.
pub fn humansize(length: u64, with_unit: bool) -> String {
    let gb: f64 = 2.0_f64.powf(30_f64);
    let mb: f64 = 2.0_f64.powf(20_f64);
    let kb: f64 = 2.0_f64.powf(10_f64);

    let flength = length as f64;

    if flength > gb {
        let j = (flength / gb).round();

        if with_unit {
            format!("{} GB", j)
        } else {
            j.to_string()
        }
    } else if flength > mb {
        let j = (flength / mb).round();

        if with_unit {
            format!("{} MB", j)
        } else {
            j.to_string()
        }
    } else if flength > kb {
        let j = (flength / kb).round();

        if with_unit {
            format!("{} KB", j)
        } else {
            j.to_string()
        }
    } else if with_unit {
        format!("{} B", flength)
    } else {
        flength.to_string()
    }
}
