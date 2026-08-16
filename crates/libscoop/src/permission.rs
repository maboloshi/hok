//! Windows ACL primitives for Scoop roots.
//!
//! Standalone domain service holding the *execution* of permission
//! adjustments: grant directory access for well-known groups. The
//! persist-specific guard (which root, when) lives in `persist` — keeping
//! this module free of `Session`/`Package`/persist semantics so other
//! flows (process shutdown, uninstall cleanup, ...) can reuse it for any
//! directory.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use tracing::debug;

use crate::error::Fallible;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::{
    ACL, CreateWellKnownSid, DACL_SECURITY_INFORMATION, OBJECT_INHERIT_ACE, WinBuiltinUsersSid,
};
use windows_sys::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, GRANT_ACCESS,
    SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_WRITE_ATTRIBUTES, FILE_WRITE_EA,
};

/// Maximum size of a SID (`SECURITY_MAX_SID_SIZE`).
const SECURITY_MAX_SID_SIZE: usize = 68;

/// Grant the built-in Users group (SID S-1-5-32-545) `Write` +
/// `ObjectInherit` on `root` — the ACE upstream Scoop creates in
/// `persist_permission` (lib/install.ps1:522-531:
/// `FileSystemAccessRule(..., 'Write', 'ObjectInherit', 'none', 'Allow')`).
/// Idempotent: `SetAccessRule`-style merge adds/replaces the ACE without
/// disturbing the rest of the DACL.
///
/// Callers decide *when* (e.g. the `global && persist && is_admin` guard
/// of `persist_permission`) and *which root*; this only executes the grant.
pub fn grant_users_write_inherit(root: &Path) -> Fallible<()> {
    if !root.exists() {
        return Ok(());
    }

    let path_wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();

    unsafe {
        // 1. Users well-known SID (S-1-5-32-545). Stack buffer avoids a
        // ConvertStringSidToSidW + LocalFree round trip.
        let mut sid_buf = [0u8; SECURITY_MAX_SID_SIZE];
        let mut sid_len = SECURITY_MAX_SID_SIZE as u32;
        if CreateWellKnownSid(
            WinBuiltinUsersSid,
            std::ptr::null_mut(),
            sid_buf.as_mut_ptr().cast(),
            &mut sid_len,
        ) == 0 {
            return Err(crate::Error::Custom(format!(
                "failed to resolve the Users SID (S-1-5-32-545) for {}",
                root.display()
            )));
        }

        // 2. Read the current DACL so the grant merges instead of replacing
        //    the whole ACL (SetAccessRule semantics).
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut sd: *mut core::ffi::c_void = std::ptr::null_mut();
        let status = GetNamedSecurityInfoW(
            path_wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut sd,
        );
        if status != 0 {
            LocalFree(sd);
            return Err(crate::Error::Custom(format!(
                "failed to read the ACL of {} (error {status})",
                root.display()
            )));
        }

        // 3. Build the grant: .NET FileSystemRights.Write = WriteData |
        //    AppendData | WriteExtendedAttributes | WriteAttributes;
        //    InheritanceFlags = ObjectInherit (child files only).
        let trustee = TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: sid_buf.as_mut_ptr().cast::<u16>(),
        };
        let access = EXPLICIT_ACCESS_W {
            grfAccessPermissions:
                FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY | FILE_WRITE_EA | FILE_WRITE_ATTRIBUTES,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: OBJECT_INHERIT_ACE,
            Trustee: trustee,
        };

        let mut new_dacl: *mut ACL = std::ptr::null_mut();
        let status = SetEntriesInAclW(1, &access, dacl, &mut new_dacl);
        if status != 0 {
            LocalFree(sd);
            return Err(crate::Error::Custom(format!(
                "failed to build the merged ACL for {} (error {status})",
                root.display()
            )));
        }

        // 4. Apply the merged DACL back.
        let status = SetNamedSecurityInfoW(
            path_wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            new_dacl,
            std::ptr::null(),
        );
        LocalFree(new_dacl.cast());
        LocalFree(sd);
        if status != 0 {
            return Err(crate::Error::Custom(format!(
                "failed to grant the Users group write permission on {} (error {status})",
                root.display()
            )));
        }
    }

    debug!(
        "granted Users (S-1-5-32-545) write+inherit permission on {}",
        root.display()
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, ACL};
    use windows_sys::Win32::Security::Authorization::{
        GetEffectiveRightsFromAclW, GetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_USER,
        TRUSTEE_W,
    };

    /// Grant on a scratch directory, then read the DACL back and verify the
    /// Users group actually holds the write rights — guards against a grant
    /// that silently no-ops (wrong SID, wrong rights mask, ...).
    #[test]
    fn grant_users_write_inherit_sets_dacl() {
        let dir = std::env::temp_dir().join(format!("hok-acl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        grant_users_write_inherit(&dir).unwrap();

        let path_wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut sd: *mut core::ffi::c_void = std::ptr::null_mut();
        unsafe {
            let status = GetNamedSecurityInfoW(
                path_wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut sd,
            );
            assert_eq!(status, 0, "GetNamedSecurityInfoW failed: {status}");

            let mut sid_buf = [0u8; SECURITY_MAX_SID_SIZE];
            let mut sid_len = SECURITY_MAX_SID_SIZE as u32;
            assert_ne!(
                CreateWellKnownSid(
                    WinBuiltinUsersSid,
                    std::ptr::null_mut(),
                    sid_buf.as_mut_ptr().cast(),
                    &mut sid_len,
                ),
                0,
                "CreateWellKnownSid failed"
            );

            let trustee = TRUSTEE_W {
                pMultipleTrustee: std::ptr::null_mut(),
                MultipleTrusteeOperation: 0,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_USER,
                ptstrName: sid_buf.as_mut_ptr().cast::<u16>(),
            };
            let mut rights: u32 = 0;
            let status = GetEffectiveRightsFromAclW(dacl, &trustee, &mut rights);
            assert_eq!(status, 0, "GetEffectiveRightsFromAclW failed: {status}");
            assert_ne!(
                rights & (FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY),
                0,
                "Users group missing write rights on {} (rights {rights:#x})",
                dir.display()
            );

            LocalFree(sd);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Calling the grant twice must not accumulate duplicate Users ACEs —
    /// upstream `SetAccessRule` replaces the matching ACE instead.
    #[test]
    fn grant_users_write_inherit_is_idempotent() {
        let dir =
            std::env::temp_dir().join(format!("hok-acl-idem-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        grant_users_write_inherit(&dir).unwrap();
        grant_users_write_inherit(&dir).unwrap();

        let path_wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut sd: *mut core::ffi::c_void = std::ptr::null_mut();
        unsafe {
            let status = GetNamedSecurityInfoW(
                path_wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut sd,
            );
            assert_eq!(status, 0, "GetNamedSecurityInfoW failed: {status}");

            let mut sid_buf = [0u8; SECURITY_MAX_SID_SIZE];
            let mut sid_len = SECURITY_MAX_SID_SIZE as u32;
            assert_ne!(
                CreateWellKnownSid(
                    WinBuiltinUsersSid,
                    std::ptr::null_mut(),
                    sid_buf.as_mut_ptr().cast(),
                    &mut sid_len,
                ),
                0,
                "CreateWellKnownSid failed"
            );
            let users_sid = &sid_buf[..sid_len as usize];

            use windows_sys::Win32::Security::{GetAce, GetAclInformation, AclSizeInformation};
            let mut size_info = windows_sys::Win32::Security::ACL_SIZE_INFORMATION {
                AceCount: 0,
                AclBytesInUse: 0,
                AclBytesFree: 0,
            };
            let size_info_ptr: *mut core::ffi::c_void =
                (&mut size_info as *mut windows_sys::Win32::Security::ACL_SIZE_INFORMATION)
                    .cast();
            assert_ne!(
                GetAclInformation(
                    dacl,
                    size_info_ptr,
                    std::mem::size_of::<windows_sys::Win32::Security::ACL_SIZE_INFORMATION>()
                        as u32,
                    AclSizeInformation,
                ),
                0,
                "GetAclInformation failed"
            );

            let mut users_aces = 0;
            for i in 0..size_info.AceCount {
                let mut pace: *mut core::ffi::c_void = std::ptr::null_mut();
                if GetAce(dacl, i, &mut pace) == 0 {
                    continue;
                }
                let header = pace as *const u8;
                // ACCESS_ALLOWED_ACE_TYPE == 0; the SID starts at offset 8
                // (ACE_HEADER 4 bytes + ACCESS_MASK 4 bytes).
                if *header == 0 {
                    let sid_ptr = header.add(8);
                    if std::slice::from_raw_parts(sid_ptr, users_sid.len()) == users_sid {
                        users_aces += 1;
                    }
                }
            }
            assert_eq!(
                users_aces, 1,
                "expected exactly one Users ACE after two grants, got {users_aces}"
            );

            LocalFree(sd);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

