//! Throwaway benchmark #2: which Windows deletion path is fastest for
//! cleanup-shaped trees, and what a deferred (rename-then-delete) design costs.
//!
//! Variants, all on the same generated tree:
//! - std       : `std::fs::remove_dir_all` (what hok uses today)
//! - del       : manual walk, classic `DeleteFileW` / `RemoveDirectoryW`
//! - posix     : manual walk, `FileDispositionInfoEx` with POSIX semantics
//!               (skips the rename-to-delete-pending step of classic delete)
//! - rename    : `fs::rename` of each old-version dir into a staging dir
//!               (the user-visible cost of a deferred-delete design)
//! - *_par4    : the same work with 4 threads

use std::ffi::c_void;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

type Handle = *mut c_void;
const INVALID_HANDLE_VALUE: Handle = usize::MAX as Handle;
const DELETE: u32 = 0x0001_0000;
const FILE_SHARE_ALL: u32 = 0x7;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_DISPOSITION_INFO_EX: i32 = 21;
const DISPOSITION_DELETE_POSIX: u32 = 0x1 | 0x2;

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
    fn SetFileInformationByHandle(
        handle: Handle,
        class: i32,
        info: *mut c_void,
        size: u32,
    ) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn DeleteFileW(name: *const u16) -> i32;
    fn RemoveDirectoryW(name: *const u16) -> i32;
    fn SetThreadExecutionState(flags: u32) -> u32;
}

/// Keep the machine awake: a suspend mid-run counts as elapsed wall time and
/// silently poisons every `Instant` measurement below.
fn keep_awake() {
    const ES_CONTINUOUS: u32 = 0x8000_0000;
    const ES_SYSTEM_REQUIRED: u32 = 0x1;
    const ES_DISPLAY_REQUIRED: u32 = 0x2;
    unsafe {
        SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED);
    }
}

fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain([0]).collect()
}

/// Unlink one entry with `FileDispositionInfoEx` + POSIX semantics.
fn posix_unlink(path: &Path) -> std::io::Result<()> {
    let w = wide(path);
    unsafe {
        let h = CreateFileW(
            w.as_ptr(),
            DELETE,
            FILE_SHARE_ALL,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        let mut flags = DISPOSITION_DELETE_POSIX;
        let ok = SetFileInformationByHandle(
            h,
            FILE_DISPOSITION_INFO_EX,
            &mut flags as *mut u32 as *mut c_void,
            std::mem::size_of::<u32>() as u32,
        );
        CloseHandle(h);
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Classic (rename-to-delete-pending) unlink, per entry.
fn classic_unlink(path: &Path) -> std::io::Result<()> {
    let w = wide(path);
    let is_dir = fs::symlink_metadata(path).map(|m| m.is_dir()).unwrap_or(false);
    unsafe {
        let ok = if is_dir {
            RemoveDirectoryW(w.as_ptr())
        } else {
            DeleteFileW(w.as_ptr())
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

fn rmtree(path: &Path, unlink: fn(&Path) -> std::io::Result<()>) -> std::io::Result<()> {
    let md = fs::symlink_metadata(path)?;
    if md.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let child = entry.path();
            if entry.file_type()?.is_dir() {
                rmtree(&child, unlink)?;
            } else {
                unlink(&child)?;
            }
        }
    }
    unlink(path)
}

fn body(root: &Path, apps: usize, files: usize) -> Vec<PathBuf> {
    let payload = [b'x'; 32];
    let _ = fs::remove_dir_all(root);
    let mut dirs = Vec::new();
    for a in 0..apps {
        let d = root.join(format!("app{a}")).join("0.9.0");
        fs::create_dir_all(&d).unwrap();
        for f in 0..files {
            File::create(d.join(format!("f{f}.bin")))
                .unwrap()
                .write_all(&payload)
                .unwrap();
        }
        dirs.push(d);
    }
    dirs
}

fn timed(label: &str, f: impl FnOnce()) {
    let t = Instant::now();
    f();
    println!("{label}={}ms", t.elapsed().as_millis());
}

fn par(units: &[PathBuf], threads: usize, work: fn(&Path)) {
    let idx = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = idx.fetch_add(1, Ordering::Relaxed);
                if i >= units.len() {
                    break;
                }
                work(&units[i]);
            });
        }
    });
}

fn main() {
    let base = PathBuf::from(std::env::args().nth(1).expect("usage: bench2 <scratch-dir>"));
    let apps = 12;
    let files = 1500;
    keep_awake();

    println!("== {} apps x {} files ({}) ==", apps, files, base.display());

    let dirs = body(&base, apps, files);
    timed("std_serial", || {
        for d in &dirs {
            fs::remove_dir_all(d).unwrap();
        }
    });

    let dirs = body(&base, apps, files);
    timed("classic_serial", || {
        for d in &dirs {
            rmtree(d, classic_unlink).unwrap();
        }
    });

    let dirs = body(&base, apps, files);
    timed("posix_serial", || {
        for d in &dirs {
            rmtree(d, posix_unlink).unwrap();
        }
    });

    let dirs = body(&base, apps, files);
    timed("posix_par2", || par(&dirs, 2, |p| rmtree(p, posix_unlink).unwrap()));

    let dirs = body(&base, apps, files);
    timed("posix_par4", || par(&dirs, 4, |p| rmtree(p, posix_unlink).unwrap()));

    let dirs = body(&base, apps, files);
    timed("std_par4", || {
        par(&dirs, 4, |p| {
            fs::remove_dir_all(p).unwrap();
        })
    });

    // Deferred design: what the user waits for if we only rename into staging.
    let dirs = body(&base, apps, files);
    let staging = base.join("_trash");
    fs::create_dir_all(&staging).unwrap();
    timed("rename_to_staging", || {
        for d in &dirs {
            let target = staging.join(d.parent().unwrap().file_name().unwrap());
            fs::rename(d, target).unwrap();
        }
    });
    let staged: Vec<PathBuf> = fs::read_dir(&staging)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    timed("staged_posix_par4", || {
        par(&staged, 4, |p| rmtree(p, posix_unlink).unwrap())
    });

    // Safety check: a junction inside an old-version dir must have the link
    // removed while its target survives (the walk must never descend into it).
    let outside = base.parent().unwrap().join("bench2-outside-target");
    let _ = fs::remove_dir_all(&outside);
    fs::create_dir_all(&outside).unwrap();
    File::create(outside.join("keep.bin"))
        .unwrap()
        .write_all(b"keep")
        .unwrap();
    let dirs = body(&base, 1, 10);
    let victim = &dirs[0];
    let junction = victim.join("_persist_link");
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&outside)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    rmtree(victim, posix_unlink).unwrap();
    println!(
        "junction_created={made} victim_gone={} target_alive={}",
        !victim.exists(),
        outside.join("keep.bin").exists()
    );
    let _ = fs::remove_dir_all(&outside);

    let _ = fs::remove_dir_all(&base);
}
