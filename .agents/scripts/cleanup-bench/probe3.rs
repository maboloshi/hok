//! Throwaway probe #3: what actually happens to cleanup when a file inside an
//! old version directory is held open / running.
//!
//! Cases:
//!   A  classic DeleteFileW on a file held without FILE_SHARE_DELETE
//!   B  opening that file for DELETE (the handle POSIX-disposition needs)
//!   C  rename the whole version dir while a file inside is held
//!   D  RemoveDirectoryW on a dir whose only file is held
//!   E  MoveFileEx(..., MOVEFILE_DELAY_UNTIL_REBOOT) — the "delete at reboot" path
//!   F  POSIX disposition on a file held *with* FILE_SHARE_DELETE
//!   G  running image (cmd.exe) inside the version dir: delete vs rename
//!   H  junction pointing at a held dir: is the link removable?

use std::ffi::c_void;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

type Handle = *mut c_void;
const INVALID_HANDLE_VALUE: Handle = usize::MAX as Handle;
const GENERIC_READ: u32 = 0x8000_0000;
const DELETE: u32 = 0x0001_0000;
const FILE_SHARE_READ: u32 = 0x1;
const FILE_SHARE_ALL: u32 = 0x7;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const MOVEFILE_DELAY_UNTIL_REBOOT: u32 = 0x4;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *mut c_void,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn SetFileInformationByHandle(handle: Handle, class: i32, info: *mut c_void, size: u32) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn DeleteFileW(name: *const u16) -> i32;
    fn RemoveDirectoryW(name: *const u16) -> i32;
    fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
}

fn wide(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain([0]).collect()
}

fn err() -> String {
    match std::io::Error::last_os_error().raw_os_error() {
        Some(c) => format!("err {c} ({})", std::io::Error::from_raw_os_error(c)),
        None => "err ?".to_owned(),
    }
}

/// Open `path` holding it open with `share` (POSIX-disposition needs DELETE).
fn hold(path: &Path, share: u32) -> Result<Handle, String> {
    let w = wide(path);
    let h = unsafe {
        CreateFileW(
            w.as_ptr(),
            GENERIC_READ,
            share,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        Err(err())
    } else {
        Ok(h)
    }
}

fn release(h: Handle) {
    unsafe {
        CloseHandle(h);
    }
}

fn try_delete_file(p: &Path) -> String {
    let w = wide(p);
    if unsafe { DeleteFileW(w.as_ptr()) } != 0 {
        "OK".to_owned()
    } else {
        err()
    }
}

fn try_open_for_delete(p: &Path) -> String {
    let w = wide(p);
    let h = unsafe {
        CreateFileW(
            w.as_ptr(),
            DELETE,
            FILE_SHARE_ALL,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return err();
    }
    let mut flags = 0x1u32 | 0x2; // DELETE | POSIX_SEMANTICS
    let ok = unsafe {
        SetFileInformationByHandle(
            h,
            21,
            &mut flags as *mut u32 as *mut c_void,
            std::mem::size_of::<u32>() as u32,
        )
    };
    let out = if ok != 0 { "OK".to_owned() } else { err() };
    unsafe { CloseHandle(h) };
    out
}

fn fresh(base: &Path, name: &str) -> PathBuf {
    let d = base.join(name);
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn main() {
    let base = PathBuf::from(std::env::args().nth(1).expect("usage: probe3 <scratch-dir>"));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();

    // A + B: file held by an opener that did not share delete.
    let d = fresh(&base, "a1");
    let f = d.join("locked.dll");
    fs::write(&f, b"x").unwrap();
    let h = hold(&f, FILE_SHARE_READ).expect("hold");
    println!("A  DeleteFileW while held (no share delete)      -> {}", try_delete_file(&f));
    println!("B  open for DELETE + POSIX disposition, same file -> {}", try_open_for_delete(&f));
    // C: rename the containing version dir while the file is still held.
    let moved = base.join("a1.moved");
    let ren = fs::rename(&d, &moved);
    println!(
        "C  rename version dir while file inside held      -> {}",
        match ren {
            Ok(()) => "OK".to_owned(),
            Err(e) => format!("err {e}"),
        }
    );
    release(h);
    let _ = fs::remove_dir_all(&moved);

    // D: only file in the dir is held -> dir cannot go away.
    let d = fresh(&base, "a2");
    let f = d.join("only.bin");
    fs::write(&f, b"x").unwrap();
    let h = hold(&f, FILE_SHARE_READ).expect("hold");
    let w = wide(&d);
    let rm = unsafe { RemoveDirectoryW(w.as_ptr()) };
    println!(
        "D  RemoveDirectoryW, only file held               -> {}",
        if rm != 0 { "OK".to_owned() } else { err() }
    );

    // E: the "delete at reboot" registration.
    let mf = unsafe { MoveFileExW(wide(&f).as_ptr(), std::ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT) };
    println!(
        "E  MoveFileEx DELAY_UNTIL_REBOOT (non-elevated)   -> {}",
        if mf != 0 { "OK (registered)".to_owned() } else { err() }
    );
    release(h);
    let _ = fs::remove_dir_all(&d);

    // F: opener shared delete -> both paths succeed on the spot.
    let d = fresh(&base, "a3");
    let f = d.join("shared.bin");
    fs::write(&f, b"x").unwrap();
    let h = hold(&f, FILE_SHARE_ALL).expect("hold");
    println!("F1 DeleteFileW while held (share delete)          -> {}", try_delete_file(&f));
    let f2 = d.join("shared2.bin");
    fs::write(&f2, b"x").unwrap();
    let h2 = hold(&f2, FILE_SHARE_ALL).expect("hold");
    println!("F2 POSIX disposition while held (share delete)    -> {}", try_open_for_delete(&f2));
    release(h);
    release(h2);
    let _ = fs::remove_dir_all(&d);

    // G: the realistic case — the app's own binary is still running.
    let d = fresh(&base, "a4");
    fs::copy("C:\\Windows\\System32\\cmd.exe", d.join("cmd.exe")).unwrap();
    let mut child = Command::new(d.join("cmd.exe"))
        .args(["/C", "ping -n 20 127.0.0.1 > NUL"])
        .spawn()
        .expect("spawn");
    std::thread::sleep(Duration::from_millis(500));
    println!(
        "G1 remove_dir_all with a running image inside    -> {}",
        if fs::remove_dir_all(&d).is_ok() { "OK".to_owned() } else { err() }
    );
    let moved = base.join("a4.moved");
    println!(
        "G2 rename version dir with a running image inside -> {}",
        match fs::rename(&d, &moved) {
            Ok(()) => "OK".to_owned(),
            Err(e) => format!("err {e}"),
        }
    );
    let _ = child.kill();
    let _ = child.wait();
    std::thread::sleep(Duration::from_millis(300));
    println!(
        "G3 delete the renamed dir after the image exits   -> {}",
        if fs::remove_dir_all(&moved).is_ok() { "OK".to_owned() } else { err() }
    );

    // H: junction whose target has a held file — is the link itself removable?
    let target = fresh(&base, "target");
    let tf = target.join("held.bin");
    fs::write(&tf, b"x").unwrap();
    let h = hold(&tf, FILE_SHARE_READ).expect("hold");
    let link = base.join("link");
    let made = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if made {
        println!(
            "H  remove the junction while target file is held  -> {}",
            if fs::remove_dir_all(&link).is_ok() { "OK (link gone)".to_owned() } else { err() }
        );
    }
    release(h);

    let _ = fs::remove_dir_all(&base);
}
