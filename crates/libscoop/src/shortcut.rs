use std::sync::LazyLock;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;

use crate::{error::Fallible, internal, package::Package, Event, Session};

static SCOOP_SHORTCUT_DIR: LazyLock<PathBuf> = LazyLock::new(shortcut_dir);

/// Return the path to the shortcut directory.
///
/// `~\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Scoop Apps`
fn shortcut_dir() -> PathBuf {
    let mut dir = dirs::config_dir().unwrap();
    dir.push("Microsoft/Windows/Start Menu/Programs/Scoop Apps");
    internal::path::normalize_path(dir)
}

/// Add shortcut(s) for a given package.
#[cfg(windows)]
pub fn add(session: &Session, package: &Package) -> Fallible<()> {
    if let Some(shortcuts) = package.manifest().shortcuts() {
        let config = session.config();
        let apps_dir = config.root_path().join("apps");

        // Ensure shortcut dir exists
        internal::fs::ensure_dir(&*SCOOP_SHORTCUT_DIR)?;

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageShortcutAddStart);
        }

        for shortcut in shortcuts {
            let length = shortcut.len();
            assert!(length > 1);

            // shortcut[0] = target exe relative to package dir, e.g. "bin\prog.exe"
            // shortcut[1] = display name in start menu
            let target = apps_dir.join(package.name()).join("current").join(shortcut[0]);
            let mut link_path = SCOOP_SHORTCUT_DIR.join(shortcut[1]);
            link_path.set_extension("lnk");

            create_shortcut(&target, &link_path)?;

            if let Some(tx) = session.emitter() {
                let name = link_path.file_name().unwrap().to_str().unwrap().to_owned();
                let _ = tx.send(Event::PackageShortcutAddProgress(name));
            }
        }

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageShortcutAddDone);
        }
    }

    Ok(())
}

/// Create a `.lnk` shortcut file via IShellLink COM interface.
///
/// This is a raw FFI call to shell32 / ole32 with zero extra dependencies.
#[cfg(windows)]
fn create_shortcut(target: &Path, link: &Path) -> std::io::Result<()> {
    // CLSID_ShellLink: {00021401-0000-0000-C000-000000000046}
    const CLSID_SHELLLINK: GUID = GUID {
        data1: 0x00021401,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };

    // IID_IShellLinkW: {000214F9-0000-0000-C000-000000000046}
    const IID_ISHELLLINKW: GUID = GUID {
        data1: 0x000214F9,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };

    // IID_IPersistFile: {0000010B-0000-0000-C000-000000000046}
    const IID_IPERSISTFILE: GUID = GUID {
        data1: 0x0000010B,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };

    #[repr(C)]
    struct GUID {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitialize(pvReserved: *const std::ffi::c_void) -> i32;
        fn CoCreateInstance(
            rclsid: *const GUID,
            pUnkOuter: *mut std::ffi::c_void,
            dwClsContext: u32,
            riid: *const GUID,
            ppv: *mut *mut std::ffi::c_void,
        ) -> i32;
        fn CoUninitialize();
    }

    // IShellLinkW vtable
    #[allow(dead_code)]
    type IShellLinkW = *mut *mut IShellLinkWVtbl;
    #[repr(C)]
    struct IShellLinkWVtbl {
        // IUnknown
        query_interface: unsafe extern "system" fn(*mut std::ffi::c_void, *const GUID, *mut *mut std::ffi::c_void) -> i32,
        add_ref: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
        release: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
        // IShellLinkW
        set_path: unsafe extern "system" fn(*mut std::ffi::c_void, *const u16, *mut *mut std::ffi::c_void, u32) -> i32,
        get_path: *mut std::ffi::c_void,
        find_target: *mut std::ffi::c_void,
        get_arg_list: *mut std::ffi::c_void,
        set_arg_list: unsafe extern "system" fn(*mut std::ffi::c_void, *const u16) -> i32,
        get_description: *mut std::ffi::c_void,
        set_description: unsafe extern "system" fn(*mut std::ffi::c_void, *const u16) -> i32,
        get_working_dir: *mut std::ffi::c_void,
        set_working_dir: unsafe extern "system" fn(*mut std::ffi::c_void, *const u16) -> i32,
        get_show_cmd: *mut std::ffi::c_void,
        set_show_cmd: unsafe extern "system" fn(*mut std::ffi::c_void, i32) -> i32,
        // ... remaining methods not needed
    }

    // IPersistFile vtable
    #[allow(dead_code)]
    type IPersistFile = *mut *mut IPersistFileVtbl;
    #[repr(C)]
    struct IPersistFileVtbl {
        // IUnknown
        query_interface: unsafe extern "system" fn(*mut std::ffi::c_void, *const GUID, *mut *mut std::ffi::c_void) -> i32,
        add_ref: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
        release: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
        // IPersist
        get_class_id: *mut std::ffi::c_void,
        // IPersistFile
        is_dirty: *mut std::ffi::c_void,
        load: *mut std::ffi::c_void,
        save: unsafe extern "system" fn(*mut std::ffi::c_void, *const u16, i32) -> i32,
        save_completed: *mut std::ffi::c_void,
        get_cur_file: *mut std::ffi::c_void,
    }

    let hr = unsafe { CoInitialize(ptr::null()) };
    if hr < 0 && hr != -2147221008 /* RPC_E_CHANGED_MODE */ {
        return Err(std::io::Error::last_os_error());
    }

    let mut p_sl: *mut std::ffi::c_void = ptr::null_mut();
    let hr = unsafe {
        CoCreateInstance(
            &CLSID_SHELLLINK,
            ptr::null_mut(),
            1, // CLSCTX_INPROC_SERVER
            &IID_ISHELLLINKW,
            &mut p_sl,
        )
    };
    if hr < 0 {
        unsafe { CoUninitialize() };
        return Err(std::io::Error::from_raw_os_error(hr));
    }

    let mut result = Ok(());

    unsafe {
        let vtbl = *(p_sl as *mut *mut IShellLinkWVtbl) ;

        // Set target path
        let target_wide: Vec<u16> = OsStr::new(target.as_os_str())
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let hr = ((*vtbl).set_path)(p_sl, target_wide.as_ptr(), ptr::null_mut(), 0);
        if hr < 0 {
            result = Err(std::io::Error::from_raw_os_error(hr));
        }

        // Set description
        if result.is_ok() {
            let desc_wide: Vec<u16> = OsStr::new("Scoop installed application")
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let hr = ((*vtbl).set_description)(p_sl, desc_wide.as_ptr());
            if hr < 0 {
                result = Err(std::io::Error::from_raw_os_error(hr));
            }
        }

        // Get IPersistFile
        if result.is_ok() {
            let mut p_pf: *mut std::ffi::c_void = ptr::null_mut();
            let hr = ((*vtbl).query_interface)(p_sl, &IID_IPERSISTFILE, &mut p_pf);
            if hr < 0 {
                result = Err(std::io::Error::from_raw_os_error(hr));
            } else {
                let pf_vtbl = *(p_pf as *mut *mut IPersistFileVtbl);

                // Save .lnk file
                let link_wide: Vec<u16> = OsStr::new(link.as_os_str())
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                let hr = ((*pf_vtbl).save)(p_pf, link_wide.as_ptr(), 1); // TRUE = remember
                if hr < 0 {
                    result = Err(std::io::Error::from_raw_os_error(hr));
                }

                // Release IPersistFile
                let _ = ((*pf_vtbl).release)(p_pf);
            }
        }

        // Release IShellLinkW
        let _ = ((*vtbl).release)(p_sl);
    }

    unsafe { CoUninitialize() };
    result
}

/// Remove shortcut(s) for a given package.
#[cfg(not(windows))]
pub fn add(_session: &Session, _package: &Package) -> Fallible<()> {
    Ok(())
}

/// Remove shortcut(s) for a given package.
pub fn remove(session: &Session, package: &Package) -> Fallible<()> {
    assert!(package.is_installed());

    if let Some(shortcuts) = package.manifest().shortcuts() {
        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageShortcutRemoveStart);
        }

        for shortcut in shortcuts {
            let length = shortcut.len();
            assert!(length > 1);

            let mut path = SCOOP_SHORTCUT_DIR.join(shortcut[1]);
            path.set_extension("lnk");

            if let Some(tx) = session.emitter() {
                let shortcut_name = path.file_name().unwrap().to_str().unwrap().to_owned();
                let _ = tx.send(Event::PackageShortcutRemoveProgress(shortcut_name));
            }

            let _ = std::fs::remove_file(&path);
        }

        if let Some(tx) = session.emitter() {
            let _ = tx.send(Event::PackageShortcutRemoveDone);
        }
    }
    Ok(())
}
