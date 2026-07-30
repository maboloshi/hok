use std::path::{Path, PathBuf};
use libscoop::internal::os::encode_wide;

// ─── Shell open (Windows FFI) ──────────────────────────────────────────────

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

// ─── URL utility functions ─────────────────────────────────────────────────
// These are shared URL helpers extracted for reuse across commands.

/// Extract the filename portion of a URL (last path segment).
#[allow(dead_code)]
pub fn url_remote_filename(url: &str) -> String {
    let decoded = url_decoded(url);
    decoded.rsplit('/').next().unwrap_or(&decoded).to_string()
}

/// Extract basename from a URL (filename without extension).
#[allow(dead_code)]
pub fn url_basename(url: &str) -> String {
    let filename = url.rsplit('/').next().unwrap_or(url);
    let dot = filename.rfind('.');
    match dot {
        Some(pos) => filename[..pos].to_string(),
        None => filename.to_string(),
    }
}

/// Decode URL percent-encoding.
#[allow(dead_code)]
pub fn url_decoded(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
                continue;
            }
            result.push('%');
            result.push_str(&hex);
        } else {
            result.push(c);
        }
    }
    result
}

// ─── Directory walking ──────────────────────────────────────────────────────

/// Recursively collect all `.json` files under a directory.
///
/// Uses an explicit stack to avoid deep recursion on deeply nested trees.
pub fn walkdir_files(dir: &PathBuf) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.clone()];
    while let Some(current) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&current) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().map_or(false, |e| e == "json") {
                    files.push(path);
                }
            }
        }
    }
    files
}
