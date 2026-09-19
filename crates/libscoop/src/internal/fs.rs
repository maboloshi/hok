//! File-system utilities.
//!
//! Thin wrappers around common `std::fs` operations used throughout
//! the codebase: directory creation, recursive removal, file copying,
//! and JSON serialisation with pretty-printing.
//!
//! # Design
//!
//! - **Error-returning helpers**: [`ensure_dir()`], [`remove_dir()`],
//!   and [`write_json()`] return `Fallible` for ergonomic use with `?`.
//! - **Scoop-style JSON output**: [`write_json()`] writes with 4-space
//!   indentation, CRLF line endings and a trailing newline — matching the
//!   official Scoop manifest convention (same as
//!   [`crate::package::formatjson::to_scoop_json`]).

use serde::Serialize;
use serde_json::ser::PrettyFormatter;
use std::io;
use std::path::Path;

use crate::Error;
use crate::error::Fallible;

/// Ensure given `path` exist.
#[inline]
pub fn ensure_dir<P: AsRef<Path> + ?Sized>(path: &P) -> io::Result<()> {
    std::fs::create_dir_all(path.as_ref())
}

/// Read the entire contents of a file into a string.
#[inline]
pub fn read_to_string<P: AsRef<Path>>(path: P) -> io::Result<String> {
    std::fs::read_to_string(path.as_ref())
}

/// Write the given contents to a file at given path.
#[inline]
pub fn write<P: AsRef<Path>, C: AsRef<[u8]>>(path: P, contents: C) -> io::Result<()> {
    std::fs::write(path.as_ref(), contents.as_ref())
}

/// Remove given `path` recursively.
#[inline]
pub fn remove_dir<P: AsRef<Path>>(path: P) -> io::Result<()> {
    std::fs::remove_dir_all(path.as_ref())
}

/// Write given serializable data to a JSON file at given path.
///
/// This function will create the file if it does not exist, and truncate it.
///
/// The output uses 4-space indentation, CRLF line endings and a trailing
/// newline, matching the official Scoop manifest convention — the same
/// output as [`crate::package::formatjson::to_scoop_json`]. This keeps the
/// file ending intact (no "no newline at end of file" diffs) when a
/// manifest is rewritten by commands such as `checkver`/`checkhashes`.
pub fn write_json<P, D>(path: P, data: D) -> Fallible<()>
where
    P: AsRef<Path>,
    D: Serialize,
{
    let path = path.as_ref();
    // A path with no parent directory (empty, or a bare root) has nowhere to
    // be written: report it instead of panicking. The config path is empty
    // when `$HOME`/`USERPROFILE` is unset (see
    // [`crate::config::default::home_join`]).
    let parent = path.parent().ok_or_else(|| {
        Error::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "cannot write JSON to '{}': the path has no parent directory",
                path.display()
            ),
        ))
    })?;
    ensure_dir(parent)?;

    // Use 4 spaces for indentation
    let formatter = PrettyFormatter::with_indent(b"    ");
    let mut buf = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    data.serialize(&mut ser)?;

    // CRLF line endings + trailing newline (official Scoop writes manifests
    // via `[IO.File]::WriteAllLines`, which appends a newline after the
    // final line).
    let mut content =
        String::from_utf8(buf).map_err(|e| Error::Custom(format!("utf8 error: {}", e)))?;
    content = content.replace('\n', "\r\n");
    if !content.ends_with("\r\n") {
        content.push_str("\r\n");
    }

    std::fs::write(path, content.as_bytes())?;
    Ok(())
}

/// Remove a symlink at `lnk`.
pub fn remove_symlink<P: AsRef<Path>>(lnk: P) -> io::Result<()> {
    let lnk = lnk.as_ref();
    let metadata = lnk.symlink_metadata()?;
    let mut permissions = metadata.permissions();

    // Remove possible readonly flag on the symlink added by `attrib +R` command
    if permissions.readonly() {
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(lnk, permissions)?;
    }

    if let Ok(target_metadata) = lnk.metadata() {
        if target_metadata.file_type().is_dir() {
            std::fs::remove_dir(lnk)
        } else {
            std::fs::remove_file(lnk)
        }
    } else {
        std::fs::remove_file(lnk).or_else(|_| std::fs::remove_dir(lnk))
    }
}

/// Create a directory symlink at `lnk` pointing to `src`.
pub fn symlink_dir<P: AsRef<Path>, Q: AsRef<Path>>(src: P, lnk: Q) -> io::Result<()> {
    let src = src.as_ref();
    let lnk = lnk.as_ref();
    // A leftover entry has to go before the link can be created. Removal fails
    // for a real non-empty directory, which no link can replace anyway: report
    // that failure rather than letting the link creation below surface the
    // less informative `ERROR_ALREADY_EXISTS` (os error 183).
    if let Err(e) = remove_symlink(lnk)
        && lnk.exists()
    {
        return Err(e);
    }

    // Same degenerate-path guard as [`write_json`]: a link path with no
    // parent directory cannot be created.
    let parent = lnk.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "cannot create a directory symlink at '{}': the path has no parent directory",
                lnk.display()
            ),
        )
    })?;
    ensure_dir(parent)?;
    if std::os::windows::fs::symlink_dir(src, lnk).is_err() {
        junction::create(src, lnk)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hok-fs-writejson-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn write_json_output_ends_with_crlf() {
        let path = tmp_path("ends-with-crlf.json");
        let v = serde_json::json!({"version": "1.0"});
        write_json(&path, &v).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(
            text.ends_with("\r\n"),
            "output should end with CRLF, got tail: {:?}",
            &text[text.len().saturating_sub(12)..]
        );
        // Interior line endings must be CRLF too, matching Scoop convention.
        assert!(text.contains("\r\n"), "output should use CRLF line endings");
        assert!(
            !text.split("\r\n").any(|line| line.contains('\n')),
            "no bare LF line endings allowed"
        );
    }

    #[test]
    fn write_json_preserves_trailing_newline_when_rewriting() {
        let path = tmp_path("rewrite.json");
        // Simulate a real manifest with a trailing newline + blank line.
        std::fs::write(&path, "{\n    \"version\": \"0.9\"\n}\n\n").unwrap();

        let v = serde_json::json!({"version": "1.0"});
        write_json(&path, &v).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert!(
            bytes.ends_with(b"\n"),
            "rewritten file must end with a newline"
        );
        let text = String::from_utf8(bytes).unwrap();
        assert!(
            text.ends_with("\r\n"),
            "rewritten file should end with CRLF"
        );
    }

    /// A path with no parent directory (an empty path, as produced by
    /// `config::default::hok_config_path` without a home directory) must be
    /// rejected with an error instead of panicking.
    #[test]
    fn write_json_rejects_path_without_parent() {
        let v = serde_json::json!({"version": "1.0"});

        let err = write_json(std::path::PathBuf::new(), &v).unwrap_err();
        assert!(matches!(err, Error::Io(_)), "got: {err:?}");
        assert!(
            err.to_string().contains("no parent directory"),
            "unexpected message: {err}"
        );

        // A bare root has no parent either.
        let err = write_json("/", &v).unwrap_err();
        assert!(matches!(err, Error::Io(_)), "got: {err:?}");
    }

    /// [`symlink_dir`] shares the degenerate-path guard: an empty link path is
    /// an error, not a panic.
    #[test]
    fn symlink_dir_rejects_path_without_parent() {
        let err = symlink_dir(".", std::path::PathBuf::new()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    /// A real non-empty directory at the link path cannot be replaced by a
    /// link. The cleanup failure must be reported as-is instead of being
    /// swallowed into the misleading `ERROR_ALREADY_EXISTS` the link creation
    /// would raise (the shape that made a failed persist link read as
    /// "os error 183").
    #[test]
    fn symlink_dir_reports_an_unremovable_existing_directory() {
        let dir = crate::test_utils::tmpdir("fs_symlink_unremovable");
        let src = dir.join("src");
        let lnk = dir.join("lnk");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&lnk).unwrap();
        std::fs::write(lnk.join("keep"), b"x").unwrap();

        let err = symlink_dir(&src, &lnk).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::DirectoryNotEmpty);
        assert!(
            lnk.join("keep").exists(),
            "the directory must be left alone"
        );
    }

    /// A leftover link at the link path is still replaced silently — the
    /// tightening above must not turn an ordinary re-link into an error.
    #[test]
    fn symlink_dir_replaces_an_existing_link() {
        let dir = crate::test_utils::tmpdir("fs_symlink_relink");
        let first = dir.join("first");
        let second = dir.join("second");
        let lnk = dir.join("lnk");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();

        symlink_dir(&first, &lnk).unwrap();
        symlink_dir(&second, &lnk).unwrap();

        assert_eq!(
            std::fs::canonicalize(&lnk).unwrap(),
            std::fs::canonicalize(&second).unwrap()
        );
    }
}
