//! PE executable introspection — subsystem detection.
//!
//! hok picks the shim variant for an `.exe` target based on its PE
//! subsystem: GUI targets get the GUI-subsystem shim (no console window on
//! double-click), console targets get the console shim (the invoking shell
//! waits, so interactive children keep working).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// `IMAGE_SUBSYSTEM_WINDOWS_GUI` — target is a GUI application.
const SUBSYSTEM_WINDOWS_GUI: u16 = 2;

/// Read the PE `Subsystem` field of an executable.
///
/// Returns `None` when the file is missing, unreadable, or not a PE image
/// (bad MZ/PE signatures, truncated header). The field distinguishes GUI
/// (`2`) from console (`3`) targets.
pub fn subsystem(path: &Path) -> Option<u16> {
    let mut file = File::open(path).ok()?;

    // DOS header: "MZ" magic, `e_lfanew` at 0x3C.
    let mut dos = [0u8; 0x40];
    file.read_exact(&mut dos).ok()?;
    if dos[0] != b'M' || dos[1] != b'Z' {
        return None;
    }
    let pe_off = u32::from_le_bytes([dos[0x3C], dos[0x3D], dos[0x3E], dos[0x3F]]) as u64;

    // PE signature (4) + COFF header (20) + enough of the optional header
    // to reach `Subsystem` (offset 0x44 within both PE32 and PE32+ optional
    // headers) = 0x5C from the PE start.
    let mut buf = [0u8; 0x60];
    file.seek(SeekFrom::Start(pe_off)).ok()?;
    file.read_exact(&mut buf).ok()?;
    if &buf[..4] != b"PE\0\0" {
        return None;
    }
    Some(u16::from_le_bytes([buf[0x5C], buf[0x5D]]))
}

/// Whether the executable at `path` is a GUI-subsystem image.
///
/// Files that cannot be read (missing, non-PE, truncated, …) are treated as
/// **not** GUI — callers then fall back to the console shim variant, which
/// keeps interactive children working at the cost of a console flash when
/// double-clicked.
pub fn is_gui(path: &Path) -> bool {
    subsystem(path) == Some(SUBSYSTEM_WINDOWS_GUI)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subsystem_gui() {
        let dir = crate::test_utils::tmpdir("pe_subsystem_gui");
        let exe = dir.join("app.exe");
        crate::test_utils::write_fake_pe(&exe, 2);
        assert_eq!(subsystem(&exe), Some(2));
        assert!(is_gui(&exe));
    }

    #[test]
    fn test_subsystem_console() {
        let dir = crate::test_utils::tmpdir("pe_subsystem_console");
        let exe = dir.join("app.exe");
        crate::test_utils::write_fake_pe(&exe, 3);
        assert_eq!(subsystem(&exe), Some(3));
        assert!(!is_gui(&exe));
    }

    #[test]
    fn test_not_a_pe() {
        let dir = crate::test_utils::tmpdir("pe_not_pe");
        let f = dir.join("app.exe");
        std::fs::write(&f, b"not an executable at all").unwrap();
        assert_eq!(subsystem(&f), None);
        assert!(!is_gui(&f));
    }

    #[test]
    fn test_missing_file() {
        let dir = crate::test_utils::tmpdir("pe_missing");
        assert_eq!(subsystem(&dir.join("nope.exe")), None);
        assert!(!is_gui(&dir.join("nope.exe")));
    }

    #[test]
    fn test_truncated_header() {
        let dir = crate::test_utils::tmpdir("pe_truncated");
        let f = dir.join("app.exe");
        // MZ magic but fewer than 0x40 bytes → cannot read e_lfanew
        std::fs::write(&f, b"MZ").unwrap();
        assert_eq!(subsystem(&f), None);
    }

    #[test]
    fn test_pe_off_out_of_bounds() {
        let dir = crate::test_utils::tmpdir("pe_off_oob");
        let f = dir.join("app.exe");
        let mut data = vec![0u8; 0x100];
        data[0] = b'M';
        data[1] = b'Z';
        // e_lfanew points past EOF → seek/read fails → None
        data[0x3C..0x40].copy_from_slice(&0xFFFFu32.to_le_bytes());
        std::fs::write(&f, &data).unwrap();
        assert_eq!(subsystem(&f), None);
        assert!(!is_gui(&f));
    }
}
