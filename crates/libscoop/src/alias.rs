//! Manage Scoop command aliases.
//!
//! Mirrors upstream `lib/commands.ps1`: an alias is a
//! `<shims>\scoop-<name>.ps1` script whose first line carries the optional
//! summary, and the config `alias` map records `name -> "scoop-<name>"` as its
//! index. The scripts (not the config) are the command registry — upstream
//! `command_files` globs `shims\scoop-*.ps1` and `command_path` resolves a
//! subcommand to that file — so [`resolve`] checks the file system, and a
//! config entry whose script is gone is reported as `<BROKEN>` by [`list`] but
//! is not dispatchable.
//!
//! This is the single entry point for alias storage, so the CLI never touches
//! the shims directory or the `alias` config key itself.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::Session;
use crate::error::{Error, Fallible};
use crate::internal::fs::ensure_dir;
use crate::internal::os::ps_command;

/// One configured alias, as rendered by `hok alias list`.
pub struct Alias {
    /// The alias name (the config key).
    pub name: String,
    /// The script body (everything after the `# Summary:` line), or
    /// `<BROKEN>` when the backing script file is missing.
    pub command: String,
    /// The `# Summary:` line, empty when the script carries none.
    pub summary: String,
    /// Whether the backing script file is missing.
    pub broken: bool,
}

/// Script file stem of an alias: `scoop-<name>`.
pub fn script_stem(name: &str) -> String {
    format!("scoop-{name}")
}

/// Path of the script backing `name`: `<shims>\scoop-<name>.ps1`.
pub fn script_path(session: &Session, name: &str) -> PathBuf {
    session
        .shims_dir()
        .join(format!("{}.ps1", script_stem(name)))
}

/// Resolve `token` to its alias script, if that script exists.
///
/// Upstream treats the shims directory as the command registry, so this is a
/// file-system lookup — never a config lookup.
pub fn resolve(session: &Session, token: &str) -> Option<PathBuf> {
    let path = script_path(session, token);
    path.is_file().then_some(path)
}

/// Every configured alias, sorted by name.
///
/// An empty config yields an empty list; the caller decides how to report it.
pub fn list(session: &Session) -> Fallible<Vec<Alias>> {
    let config = session.config();
    let Some(map) = config.aliases() else {
        return Ok(Vec::new());
    };
    let mut aliases: Vec<Alias> = map.keys().map(|name| read(session, name)).collect();
    aliases.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(aliases)
}

/// Read one alias from its script file, falling back to `<BROKEN>`.
fn read(session: &Session, name: &str) -> Alias {
    let path = script_path(session, name);
    let Ok(bytes) = std::fs::read(&path) else {
        return Alias {
            name: name.to_owned(),
            command: BROKEN.to_owned(),
            summary: String::new(),
            broken: true,
        };
    };
    // Lossy decode, like the config reader: upstream's `Get-Content` never
    // fails on stray bytes, it substitutes them.
    let content = String::from_utf8_lossy(&bytes);
    Alias {
        name: name.to_owned(),
        command: command_of(&content),
        summary: summary_of(&content),
        broken: false,
    }
}

/// Marker upstream prints for an alias whose script file is missing.
const BROKEN: &str = "<BROKEN>";

/// The command body: every line after the `# Summary:` line.
///
/// Upstream keeps the lines as a string array; hok joins them with a single
/// space because the value is display-only — execution always runs the script
/// file itself.
fn command_of(content: &str) -> String {
    content
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The `# Summary: <text>` line, empty when absent.
///
/// Upstream matches the pattern anywhere in the file (`lib/help.ps1:5-7`), so
/// this scans every line, not only the first one.
fn summary_of(content: &str) -> String {
    content
        .lines()
        .find_map(|line| line.trim().strip_prefix("# Summary:"))
        .map(str::trim)
        .unwrap_or_default()
        .to_owned()
}

/// The command body of an alias with its line structure preserved, or `None`
/// when its script file is missing.
///
/// [`Alias::command`] joins the lines for display; a caller that recreates the
/// script elsewhere (the `hok` export extension restored by `hok import`) needs
/// the body as written, because joining statements with spaces changes what
/// PowerShell executes.
pub fn body(session: &Session, name: &str) -> Option<String> {
    let content = std::fs::read_to_string(script_path(session, name)).ok()?;
    let body = content
        .lines()
        .skip(1)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned();
    Some(body)
}

/// Add an alias, writing its script and indexing it in the config.
///
/// `description` becomes the script's `# Summary:` line (upstream always emits
/// the line, empty when no description was given). Fails with
/// [`Error::AliasInvalidName`] when `name` cannot name a script,
/// [`Error::AliasAlreadyExists`] when the name is already configured, and
/// [`Error::AliasScriptExists`] when its script file is already taken.
pub fn add(
    session: &Session,
    name: &str,
    command: &str,
    description: Option<&str>,
) -> Fallible<()> {
    validate_name(name)?;

    if session
        .config()
        .aliases()
        .is_some_and(|map| map.contains_key(name))
    {
        return Err(Error::AliasAlreadyExists(name.to_owned()));
    }

    let path = script_path(session, name);
    if path.exists() {
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("{}.ps1", script_stem(name)));
        return Err(Error::AliasScriptExists(file));
    }

    ensure_dir(&session.shims_dir())?;
    // Byte-for-byte what upstream writes: `lib/commands.ps1:65-71` builds the
    // body as a single string joined with "`n" and pipes it into
    // `Out-UTF8File`, whose `WriteAllLines` call appends the platform newline
    // (`lib/core.ps1:1242-1246`) — so the file is UTF-8 without BOM, LF between
    // the summary and the body, and CRLF at the end. Official Scoop reads the
    // same file, so the encoding is part of the interface.
    let script = format!(
        "# Summary: {}\n{command}\r\n",
        description.unwrap_or_default()
    );
    std::fs::write(&path, script)?;

    {
        let mut config = session.config_mut()?;
        config.set_alias(name, Some(&script_stem(name)))?;
    }
    crate::config::write(&session.config())
}

/// Remove an alias: its script file (when present) and its config entry.
///
/// Fails with [`Error::AliasNotFound`] when the name is not configured,
/// matching upstream `rm_alias`'s `abort`.
pub fn remove(session: &Session, name: &str) -> Fallible<()> {
    let configured = session
        .config()
        .aliases()
        .is_some_and(|map| map.contains_key(name));
    if !configured {
        return Err(Error::AliasNotFound(name.to_owned()));
    }

    let path = script_path(session, name);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }

    {
        let mut config = session.config_mut()?;
        config.set_alias(name, None)?;
    }
    crate::config::write(&session.config())
}

/// Run an alias script with `args`, returning its exit code.
///
/// The script inherits hok's stdio so interactive aliases work, and the child
/// is killed when the session's cancellation token fires (Ctrl-C), like the
/// package script runner.
pub async fn run(session: &Session, name: &str, args: &[OsString]) -> Fallible<i32> {
    let Some(path) = resolve(session, name) else {
        return Err(Error::AliasNotFound(name.to_owned()));
    };

    let mut child = ps_command()
        .arg("-File")
        .arg(&path)
        .args(args)
        .spawn()
        .map_err(|e| Error::Custom(format!("failed to run alias '{name}': {e}")))?;

    let token = session.cancellation_token();
    loop {
        if token.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::UserAborted);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                // A signal-terminated child has no exit code; report it as a
                // generic failure rather than inventing a Scoop code.
                return Ok(status.code().unwrap_or(1));
            }
            Ok(None) => {
                tokio::select! {
                    _ = token.cancelled() => {}
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
                }
            }
            Err(e) => {
                let _ = child.kill();
                return Err(Error::Custom(format!(
                    "failed to wait for alias '{name}': {e}"
                )));
            }
        }
    }
}

/// Reject names that cannot become a `<name>.ps1` file in the shims directory.
fn validate_name(name: &str) -> Fallible<()> {
    let invalid = name.is_empty()
        || name == "."
        || name == ".."
        || name.chars().any(|c| {
            c.is_whitespace() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        });
    if invalid {
        return Err(Error::AliasInvalidName(name.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(name: &str) -> (Session, std::path::PathBuf) {
        let root = crate::test_utils::tmpdir(name);
        let session = crate::test_utils::test_session(&root);
        (session, root)
    }

    #[test]
    fn add_writes_the_upstream_script_bytes_and_config_index() {
        let (session, root) = session("alias_add");
        add(
            &session,
            "rmall",
            "scoop uninstall $args[0]",
            Some("Uninstall"),
        )
        .unwrap();

        let bytes = std::fs::read(root.join("shims/scoop-rmall.ps1")).unwrap();
        assert_eq!(
            bytes,
            b"# Summary: Uninstall\nscoop uninstall $args[0]\r\n".to_vec(),
            "upstream writes LF after the summary and CRLF at the end, no BOM"
        );
        assert_eq!(
            session
                .config()
                .aliases()
                .unwrap()
                .get("rmall")
                .map(String::as_str),
            Some("scoop-rmall"),
            "the config holds the script name, not the command body"
        );
    }

    #[test]
    fn add_without_description_keeps_the_summary_line() {
        let (session, root) = session("alias_add_no_desc");
        add(&session, "ls2", "scoop list", None).unwrap();
        let bytes = std::fs::read(root.join("shims/scoop-ls2.ps1")).unwrap();
        assert_eq!(bytes, b"# Summary: \nscoop list\r\n".to_vec());
    }

    #[test]
    fn list_reads_command_and_summary_and_marks_broken() {
        let (session, root) = session("alias_list");
        add(&session, "b", "scoop list", Some("List apps")).unwrap();
        add(&session, "a", "scoop status", None).unwrap();
        std::fs::remove_file(root.join("shims/scoop-b.ps1")).unwrap();

        let aliases = list(&session).unwrap();
        assert_eq!(aliases.len(), 2);
        assert!(!aliases[0].broken, "the intact alias sorts first");
        assert_eq!(aliases[0].name, "a");
        assert_eq!(aliases[0].command, "scoop status");
        assert!(aliases[1].broken, "the alias whose script was deleted");
        assert_eq!(aliases[1].name, "b");
        assert_eq!(aliases[1].command, BROKEN);
        assert_eq!(aliases[1].summary, "", "broken aliases carry no summary");
    }

    #[test]
    fn list_of_an_unconfigured_session_is_empty() {
        let (session, _) = session("alias_list_empty");
        assert!(list(&session).unwrap().is_empty());
    }

    #[test]
    fn add_rejects_duplicates_invalid_names_and_taken_scripts() {
        let (session, root) = session("alias_add_guards");
        add(&session, "rmall", "scoop uninstall", None).unwrap();

        assert!(matches!(
            add(&session, "rmall", "other", None),
            Err(Error::AliasAlreadyExists(_))
        ));
        for bad in ["", "a b", "a/b", "a\\b", "a:b", "a*b", "a?b", ".", ".."] {
            assert!(
                matches!(
                    add(&session, bad, "x", None),
                    Err(Error::AliasInvalidName(_))
                ),
                "{bad:?} must be rejected"
            );
        }

        // A stray script with no config entry blocks the name (upstream's
        // "File ... already exists in shims directory" guard).
        std::fs::write(root.join("shims/scoop-free.ps1"), "x").unwrap();
        assert!(matches!(
            add(&session, "free", "x", None),
            Err(Error::AliasScriptExists(_))
        ));
    }

    #[test]
    fn remove_deletes_script_and_config_and_rejects_unknown_names() {
        let (session, root) = session("alias_remove");
        add(&session, "rmall", "scoop uninstall", None).unwrap();
        assert!(resolve(&session, "rmall").is_some());

        remove(&session, "rmall").unwrap();
        assert!(!root.join("shims/scoop-rmall.ps1").exists());
        assert!(session.config().aliases().unwrap().get("rmall").is_none());
        assert!(resolve(&session, "rmall").is_none());

        assert!(matches!(
            remove(&session, "rmall"),
            Err(Error::AliasNotFound(_))
        ));
    }

    #[test]
    fn remove_tolerates_a_missing_script_but_keeps_the_config_in_sync() {
        let (session, root) = session("alias_remove_broken");
        add(&session, "rmall", "scoop uninstall", None).unwrap();
        std::fs::remove_file(root.join("shims/scoop-rmall.ps1")).unwrap();

        remove(&session, "rmall").unwrap();
        assert!(session.config().aliases().unwrap().get("rmall").is_none());
    }

    #[test]
    fn resolve_only_reports_existing_scripts() {
        let (session, root) = session("alias_resolve");
        std::fs::create_dir_all(root.join("shims")).unwrap();
        assert!(resolve(&session, "ghost").is_none());
        std::fs::write(root.join("shims/scoop-ghost.ps1"), "x").unwrap();
        assert_eq!(
            resolve(&session, "ghost").unwrap(),
            root.join("shims/scoop-ghost.ps1")
        );
    }

    #[test]
    fn command_and_summary_parsing() {
        assert_eq!(command_of("# Summary: s\nscoop list\n"), "scoop list");
        assert_eq!(
            command_of("# Summary: s\nscoop list\nscoop status\n"),
            "scoop list scoop status"
        );
        assert_eq!(command_of("no summary line\nscoop list\n"), "scoop list");
        assert_eq!(summary_of("# Summary: hello\nscoop list\n"), "hello");
        assert_eq!(summary_of("# Summary: \nscoop list\n"), "");
        assert_eq!(summary_of("scoop list\n"), "");
    }
}
