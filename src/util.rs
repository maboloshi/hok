use std::path::Path;

/// Open a URL in the default system browser.
///
/// On Windows uses `ShellExecuteW` directly (zero extra dependencies).
#[cfg(windows)]
pub fn open_url(url: &str) -> std::io::Result<()> {
    let wide = encode_wide(url);
    let verb = encode_wide("open");

    let ret = unsafe {
        shell_execute_w(
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

/// Open a file or directory with the system default handler.
///
/// On Windows uses `ShellExecuteW`. Respects the `$EDITOR` environment variable
/// when set (for text files), otherwise opens with the OS default program.
#[cfg(windows)]
pub fn open_file(path: &Path) -> std::io::Result<()> {
    let wide = encode_wide(&path.as_os_str().to_string_lossy());
    let verb = encode_wide("open");

    let ret = unsafe {
        shell_execute_w(
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

#[cfg(windows)]
fn encode_wide(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
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

/// Alias for `ShellExecuteW` that doesn't collide with the macro-like linkage.
#[cfg(windows)]
unsafe fn shell_execute_w(
    hwnd: *mut std::ffi::c_void,
    lp_operation: *const u16,
    lp_file: *const u16,
    lp_parameters: *const u16,
    lp_directory: *const u16,
    n_show_cmd: i32,
) -> isize {
    ShellExecuteW(hwnd, lp_operation, lp_file, lp_parameters, lp_directory, n_show_cmd)
}

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
