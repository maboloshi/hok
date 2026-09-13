//! Inspect Scoop shims.
//!
//! Lists the shims installed under `<root>/shims/`, resolves the concrete shim
//! files (`.cmd` / `.ps1` / `.exe` / no extension) for a given command, and
//! answers "where does this command actually live?" the way upstream's
//! `Get-CommandPath` does — the business logic behind the `hok shim` and
//! `hok which` commands.

use std::path::{Path, PathBuf};

use crate::{Session, error::Fallible};

/// List the names of all shims (each shim is reported once, without
/// extension). Both `.shim` metadata files and `.ps1` shims are collected
/// (matching Scoop's `shim list`, which globs `*.shim`, `*.ps1`); a name
/// with both variants is listed once. The list is sorted for stable output.
pub fn list_shims(session: &Session) -> Fallible<Vec<String>> {
    let shims_dir = session.shims_dir();
    if !shims_dir.exists() {
        return Ok(Vec::new());
    }

    let mut names = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(&shims_dir)?.flatten() {
        let name = entry.file_name();
        if let Some(name) = name.to_str() {
            // Collect both `.shim` (metadata; Exe/custom shims) and `.ps1`
            // shims; `.cmd` wrappers are skipped as they pair with those.
            for suffix in [".shim", ".ps1"] {
                if let Some(stem) = name.strip_suffix(suffix) {
                    names.insert(stem.to_string());
                    break;
                }
            }
        }
    }
    Ok(names.into_iter().collect())
}

/// Resolve the concrete shim files for a command name.
///
/// Returns `(extension, path)` pairs for every shim variant that exists
/// (extension is `""` for an extension-less shim). The order is stable:
/// no extension, then `.cmd`, `.ps1`, `.exe`.
pub fn shim_paths(session: &Session, name: &str) -> Fallible<Vec<(String, PathBuf)>> {
    let shims_dir = session.shims_dir();
    let mut paths = Vec::new();
    for ext in ["", ".cmd", ".ps1", ".exe"] {
        let path = shims_dir.join(format!("{name}{ext}"));
        if path.exists() {
            paths.push((ext.to_string(), path));
        }
    }
    Ok(paths)
}

/// The `shims` folder of one scope (`<root>/shims` or `<global>/shims`).
fn shims_dir_in(session: &Session, global: bool) -> PathBuf {
    let root = if global {
        session.config().global_path().to_path_buf()
    } else {
        session.config().root_path().to_path_buf()
    };
    root.join("shims")
}

/// Whether `path` is a Scoop alias script (`<shims>\scoop-<name>.ps1`,
/// upstream `lib/commands.ps1:57-74`).
fn is_alias_script(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let lower = file_name.to_ascii_lowercase();
    lower.starts_with("scoop-") && lower.ends_with(".ps1")
}

/// Whether `path` lies inside directory `dir` (case-insensitive, component
/// aware — a sibling `shims2` must not match `shims`).
fn under_dir(path: &Path, dir: &Path) -> bool {
    let path = comparable(path);
    let mut dir = comparable(dir);
    if !dir.ends_with('\\') {
        dir.push('\\');
    }
    path.starts_with(&dir)
}

/// A path rendered as a lower-cased backslash string for comparison.
fn comparable(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

/// The real target a shim points at — upstream `Get-ShimTarget`
/// (`lib/core.ps1:854-866`).
///
/// - a `.shim` metadata file: the `path = "..."` value on its first line;
/// - anything else (`.cmd`, `.ps1`, extension-less): the first `@rem` / `#`
///   comment line, falling back to the last quoted string.
///
/// The target is returned only when it exists — upstream applies
/// `Convert-Path -ErrorAction SilentlyContinue`, so a shim whose target is
/// gone resolves to nothing (and `which` then reports "not found").
pub fn shim_target(shim_path: &Path) -> Option<PathBuf> {
    let content = std::fs::read_to_string(shim_path).ok()?;

    let target = if shim_path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("shim"))
    {
        content
            .lines()
            .next()?
            .replace("path = ", "")
            .replace('"', "")
    } else {
        match first_comment(&content) {
            Some(line) if !line.is_empty() => line,
            _ => last_quoted(&content).unwrap_or_default(),
        }
    };

    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    let target = PathBuf::from(target);
    target.exists().then_some(target)
}

/// The text after a leading `@rem` / `#` on the first matching line
/// (upstream's `^(?:@rem|#)\s*(.*)$`).
fn first_comment(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let trimmed = line.trim_start();
        trimmed
            .strip_prefix("@rem")
            .or_else(|| trimmed.strip_prefix('#'))
            .map(|rest| rest.trim().to_owned())
    })
}

/// The last quoted string in `content`, excluding quotes containing `@` or `&`
/// (upstream's fallback pattern `['"]([^@&]*?)['"]`, last match wins).
fn last_quoted(content: &str) -> Option<String> {
    let chars: Vec<char> = content.chars().collect();
    let mut last = None;
    let mut index = 0;
    while index < chars.len() {
        let quote = chars[index];
        if quote == '"' || quote == '\'' {
            let mut end = index + 1;
            while end < chars.len() && chars[end] != quote {
                end += 1;
            }
            if end < chars.len() {
                let inner: String = chars[index + 1..end].iter().collect();
                if !inner.is_empty() && !inner.contains(['@', '&']) {
                    last = Some(inner);
                }
                index = end + 1;
                continue;
            }
        }
        index += 1;
    }
    last
}

/// Resolve a command to the path that would actually run — upstream
/// `Get-CommandPath` (`lib/core.ps1:498-532`).
///
/// Returns `None` when the command is not on `PATH` at all, or when it is a
/// shim whose target cannot be resolved — both are upstream's "not found".
///
/// # Deliberate deviations
///
/// Upstream only treats the *user* shims folder as alias scripts
/// (`core.ps1:518`); a **global** `scoop-*.ps1` falls into the shim branch,
/// where its `# Summary: ...` banner is read as the target. Measured on
/// Windows PowerShell 5.1 and pwsh 7: `Get-CommandPath scoop-hello` (user
/// scope) returns the script path, while `Get-CommandPath scoop-ghello`
/// (global scope) returns `$null` — official `scoop which` reports "not
/// found" for a working global alias. hok resolves alias scripts in both
/// scopes.
pub fn command_path(session: &Session, command: &str) -> Option<PathBuf> {
    let resolved = crate::shim::resolve_in_path(command)?;
    classify_command_path(session, resolved)
}

/// [`command_path`]'s classification half: `resolved` is the command file the
/// `PATH` lookup found, and this maps it to the path that would actually run.
fn classify_command_path(session: &Session, resolved: PathBuf) -> Option<PathBuf> {
    let user_shims = shims_dir_in(session, false);
    let global_shims = shims_dir_in(session, true);
    let in_shims = under_dir(&resolved, &user_shims) || under_dir(&resolved, &global_shims);

    // 1. Scoop alias scripts resolve to the script itself.
    if in_shims && is_alias_script(&resolved) {
        return Some(resolved);
    }

    // 2. Shims resolve to their recorded target.
    if in_shims {
        // Upstream rewrites a `.exe` shim path to its `.shim` metadata file
        // before reading the target; the other shim forms are read directly.
        let meta = match resolved
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        {
            true => resolved.with_extension("shim"),
            false => resolved,
        };
        return shim_target(&meta);
    }

    // 3. Anything else on `PATH` is the program itself.
    Some(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils;

    fn setup(name: &str) -> (Session, PathBuf) {
        let root = test_utils::tmpdir(name);
        let session = test_utils::test_session(&root);
        let shims = session.shims_dir();
        std::fs::create_dir_all(&shims).unwrap();
        (session, shims)
    }

    fn write(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn shim_target_reads_each_shim_form() {
        let (session, shims) = setup("shim-target-forms");
        let target = session.app_dir("app").join("current/tool.exe");
        write(&target, "stub");

        write(
            &shims.join("a.shim"),
            &format!("path = \"{}\"\r\n", target.display()),
        );
        write(
            &shims.join("b.cmd"),
            &format!("@rem {}\r\n@\"\" %*\r\n", target.display()),
        );
        write(
            &shims.join("c.ps1"),
            &format!("# {}\n$path = 1\n", target.display()),
        );

        assert_eq!(shim_target(&shims.join("a.shim")), Some(target.clone()));
        assert_eq!(shim_target(&shims.join("b.cmd")), Some(target.clone()));
        assert_eq!(shim_target(&shims.join("c.ps1")), Some(target));
    }

    #[test]
    fn shim_target_is_none_when_the_target_is_gone() {
        let (session, shims) = setup("shim-target-missing");
        let ghost = session.app_dir("app").join("current/ghost.exe");
        write(
            &shims.join("ghost.shim"),
            &format!("path = \"{}\"\r\n", ghost.display()),
        );
        assert_eq!(shim_target(&shims.join("ghost.shim")), None);
    }

    #[test]
    fn a_shim_resolves_to_its_target() {
        let (session, shims) = setup("command-path-shim");
        let target = session.app_dir("demo").join("current/demo.exe");
        write(&target, "stub");
        write(
            &shims.join("demo.shim"),
            &format!("path = \"{}\"\r\n", target.display()),
        );

        assert_eq!(
            classify_command_path(&session, shims.join("demo.exe")),
            Some(target.clone())
        );
        // A `.cmd` shim is read directly (no `.exe` -> `.shim` rewrite).
        write(
            &shims.join("demo2.cmd"),
            &format!("@rem {}\r\n", target.display()),
        );
        assert_eq!(
            classify_command_path(&session, shims.join("demo2.cmd")),
            Some(target.clone())
        );
    }

    #[test]
    fn an_alias_script_resolves_to_itself() {
        let (session, shims) = setup("command-path-alias");
        let script = shims.join("scoop-hello.ps1");
        write(&script, "# Summary: says hello\r\nWrite-Output 'hello'\r\n");
        assert_eq!(
            classify_command_path(&session, script.clone()),
            Some(script)
        );
    }

    #[test]
    fn a_non_shim_path_is_returned_as_is() {
        let (session, _shims) = setup("command-path-plain");
        let plain = session.config().root_path().join("plain/plain.exe");
        write(&plain, "stub");
        assert_eq!(classify_command_path(&session, plain.clone()), Some(plain));
    }

    #[test]
    fn a_broken_shim_resolves_to_nothing() {
        let (session, shims) = setup("command-path-broken");
        let ghost = session.app_dir("demo").join("current/ghost.exe");
        write(
            &shims.join("ghost.shim"),
            &format!("path = \"{}\"\r\n", ghost.display()),
        );
        assert_eq!(
            classify_command_path(&session, shims.join("ghost.exe")),
            None
        );
    }

    #[test]
    fn alias_scripts_are_recognized_only_for_scoop_ps1() {
        assert!(is_alias_script(Path::new(
            "C:\\scoop\\shims\\scoop-hello.ps1"
        )));
        assert!(is_alias_script(Path::new(
            "C:\\scoop\\shims\\SCOOP-Hello.PS1"
        )));
        assert!(!is_alias_script(Path::new("C:\\scoop\\shims\\hello.ps1")));
        assert!(!is_alias_script(Path::new(
            "C:\\scoop\\shims\\scoop-hello.cmd"
        )));
    }

    #[test]
    fn under_dir_is_component_aware() {
        assert!(under_dir(
            Path::new("C:\\scoop\\shims\\demo.exe"),
            Path::new("C:\\scoop\\SHIMS")
        ));
        assert!(!under_dir(
            Path::new("C:\\scoop\\shims2\\demo.exe"),
            Path::new("C:\\scoop\\shims")
        ));
    }

    #[test]
    fn last_quoted_skips_at_and_ampersand() {
        assert_eq!(
            last_quoted("@echo off\r\n\"first\"\r\n\"second\"\r\n"),
            Some("second".to_owned())
        );
        // Upstream's fallback pattern is `['"]([^@&]*?)['"]`, so a quoted run
        // containing `@` or `&` is skipped and the next one wins.
        assert_eq!(last_quoted("\"a@b\" \"keep\""), Some("keep".to_owned()));
        assert_eq!(last_quoted("\"a&b\""), None);
    }
}
