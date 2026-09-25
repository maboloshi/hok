//! Clean up apps by removing old versions.

use std::io::IsTerminal;
use std::sync::Mutex;

use clap::{ArgAction, Parser};
use libscoop::package::cleanup::{CleanupOptions, CleanupProgress, CleanupResult};
use libscoop::{Session, cache, package};

use crate::cmd::shared_args::ensure_global;
use crate::cmd::version_note;
use crate::cui::RowUI;
use crate::format::{humansize, pad_visual, visual_width};
use crate::output::style;
use crate::{Result, output};

/// Cleanup apps by removing old versions
#[derive(Debug, Parser)]
#[clap()]
pub struct Args {
    /// Given named app(s) to be cleaned up
    #[arg(action = ArgAction::Append)]
    app: Vec<String>,
    /// Cleanup all apps (alternative to '*')
    #[arg(short = 'a', long, action = ArgAction::SetTrue)]
    all: bool,
    /// Cleanup a globally installed app
    #[arg(short = 'g', long, action = ArgAction::SetTrue)]
    global: bool,
    /// Remove outdated download cache simultaneously
    #[arg(short = 'k', long, action = ArgAction::SetTrue, conflicts_with = "list")]
    cache: bool,
    /// List the installed versions instead of cleaning them
    #[arg(short = 'l', long, action = ArgAction::SetTrue)]
    list: bool,
}

/// Per-version progress rendering.
///
/// On a terminal every old version gets its own spinner row, which finishes in
/// place as `✓ app: 1.0.0` or `✗ app: 1.0.0 (err)`: the sweep reports each
/// version as it goes instead of dumping the whole batch at the end, and a
/// failure stays on the row of the version that failed. In a pipe `indicatif`
/// draws nothing, so the same lines are rendered from the settled results
/// (which also keeps a redirected run in the requested order).
enum Progress {
    Rich(RowUI),
    Pipe,
}

impl Progress {
    fn new(terminal: bool) -> Progress {
        if terminal {
            Progress::Rich(RowUI::new())
        } else {
            Progress::Pipe
        }
    }

    /// Render one notification from the sweep's workers (a terminal only).
    fn render(&mut self, progress: &CleanupProgress) {
        let Progress::Rich(ui) = self else {
            return;
        };
        match progress {
            CleanupProgress::Started { name, version } => {
                let label = row_label(name, version);
                let verb = hok_i18n::t!("cmd.cleanup_removing", name = name).to_string();
                ui.set_spin(&label, format!("{verb} {version} ..."));
            }
            CleanupProgress::Finished {
                name,
                version,
                error,
            } => {
                let label = row_label(name, version);
                match error {
                    None => ui.done(&label),
                    Some(error) => ui.fail(&label, error),
                }
            }
        }
    }

    /// Settle any row the sweep left open (when it aborted on an error).
    fn finish(&mut self) {
        if let Progress::Rich(ui) = self {
            ui.finish_all();
        }
    }
}

/// The label of one version's row: also the text of its settled line, so a
/// terminal row and a piped line read identically (`app: 1.0.0`).
fn row_label(name: &str, version: &str) -> String {
    hok_i18n::t!("cmd.cleanup_row", name = name, ver = version).to_string()
}

/// Render one settled result as plain lines: the piped counterpart of the
/// terminal rows.
fn render_settled(result: &CleanupResult) {
    for version in &result.removed {
        output::done(row_label(&result.name, version));
    }
    for failure in &result.failures {
        eprintln!(
            "  {} {} ({})",
            style::danger_bold("✗"),
            style::emphasis(row_label(&result.name, &failure.version)),
            style::danger(&failure.error)
        );
    }
}

/// One rendered row of the `--list` table.
struct VersionRow {
    /// The app name on its first row; empty on the continuation rows.
    name: String,
    /// The version directory name.
    version: String,
    /// The note column: empty on the current version, the old-version note on
    /// the retained ones.
    note: String,
    /// Retained (non-current) versions render dimmed, the continuation-row
    /// shape `hok list --versions` uses.
    dimmed: bool,
}

/// The installed-version rows of one app.
///
/// The current version heads the block and carries the app name (so the named
/// row is the one a cleanup would keep); every other version follows as a
/// dimmed continuation row, exactly like `hok list --versions`. An app with no
/// version directory at all still gets one `-` row, so it is visible in the
/// listing instead of silently vanishing.
fn version_rows(session: &Session, name: &str) -> Result<Vec<VersionRow>> {
    let mut versions = package::list::list_installed_versions(session, name)?;
    if versions.is_empty() {
        return Ok(vec![VersionRow {
            name: name.to_owned(),
            version: "-".to_owned(),
            note: hok_i18n::t!("cmd.list_no_versions").to_string(),
            dimmed: true,
        }]);
    }

    // Whether the shared resolver could name a current version at all.
    let has_current = versions.iter().any(|installed| installed.is_current);

    // Stable: the current version moves to the front, the rest keep the
    // resolver's newest-first order.
    versions.sort_by_key(|installed| !installed.is_current);

    Ok(versions
        .into_iter()
        .enumerate()
        .map(|(index, installed)| VersionRow {
            name: if index == 0 {
                name.to_owned()
            } else {
                String::new()
            },
            version: installed.version,
            note: version_note(index, installed.is_current, has_current).text(),
            dimmed: !installed.is_current,
        })
        .collect())
}

/// Append the version rows of every named app to `rows`, in order.
///
/// `list_installed_versions` resolves the app directory through the session's
/// *current* scope, so a caller merging the local and global batches must
/// consume one batch per scope instead of collecting both name lists first.
fn collect_version_rows(
    session: &Session,
    names: &[String],
    rows: &mut Vec<VersionRow>,
) -> Result<()> {
    for name in names {
        rows.extend(version_rows(session, name)?);
    }
    Ok(())
}

/// Lay out the `--list` table: emphasized header, dim dashed rule and one line
/// per version — `hok list --versions`'s shape without the per-app
/// Source/Updated columns. The note column is last and is never padded, so no
/// rendered line carries trailing spaces.
fn layout_versions(rows: &[VersionRow], headers: [&str; 3]) -> Vec<String> {
    let [hdr_name, hdr_version, hdr_note] = headers;

    let w_name = visual_width(hdr_name).max(
        rows.iter()
            .map(|row| visual_width(&row.name))
            .max()
            .unwrap_or(0),
    );
    let w_version = visual_width(hdr_version).max(
        rows.iter()
            .map(|row| visual_width(&row.version))
            .max()
            .unwrap_or(0),
    );

    let mut lines = vec![
        format!(
            "  {}  {}  {}",
            style::accent_bold(pad_visual(hdr_name, w_name)),
            style::accent_bold(pad_visual(hdr_version, w_version)),
            style::accent_bold(hdr_note),
        ),
        format!(
            "  {}  {}  {}",
            style::dim("-".repeat(w_name)),
            style::dim("-".repeat(w_version)),
            style::dim("-".repeat(visual_width(hdr_note))),
        ),
    ];

    for row in rows {
        let name = pad_visual(&row.name, w_name);
        // A row without a note ends at the version column: padding the cell
        // would only add trailing spaces.
        let version = if row.note.is_empty() {
            row.version.clone()
        } else {
            pad_visual(&row.version, w_version)
        };
        let (name, version, note) = if row.dimmed {
            (style::dim(name), style::dim(version), style::dim(&row.note))
        } else {
            (style::emphasis(name), version, row.note.clone())
        };

        let mut line = format!("  {}  {}", name, version);
        if !note.is_empty() {
            line.push_str(&format!("  {}", note));
        }
        lines.push(line);
    }

    lines
}

/// Print the version table with the localized headers.
fn render_versions(rows: &[VersionRow]) {
    let name = hok_i18n::t!("cmd.list_header_name");
    let version = hok_i18n::t!("cmd.list_header_version");
    let note = hok_i18n::t!("cmd.list_header_info");

    for line in layout_versions(rows, [name.as_str(), version.as_str(), note.as_str()]) {
        println!("{line}");
    }
}

/// Whether `--list` covers every app in scope.
///
/// Unlike the destructive path — where a bare `cleanup` is a usage error — a
/// bare `cleanup -l` is a listing of everything installed, and `-a`/`*` select
/// the same set.
fn lists_every_app(args: &Args) -> bool {
    args.all || args.app.is_empty() || args.app.iter().any(|app| app == "*")
}

/// `--list`: render the versions the selected apps have on disk, removing
/// nothing.
///
/// Read-only, so it skips the destructive path's elevation gate for
/// `--global`, and an empty selection means *every* app in scope: a bare
/// `cleanup -l` is a listing, not the destructive default upstream refuses.
fn list_versions(args: &Args, session: &Session) -> Result<()> {
    let all = lists_every_app(args);
    let mut rows = Vec::new();

    if all {
        // Local first, then global — the two batches are rendered one scope at
        // a time (see `collect_version_rows`).
        session.set_global(false);
        let names = package::cleanup::installed_app_names(session);
        collect_version_rows(session, &names, &mut rows)?;
        if args.global {
            session.set_global(true);
            let names = package::cleanup::installed_app_names(session);
            collect_version_rows(session, &names, &mut rows)?;
        }
    } else {
        // Named apps go through upstream's `Confirm-InstallationStatus`, so a
        // typo or a cross-scope app is reported instead of rendering an empty
        // table.
        let names = package::cleanup::confirm_installation_status(session, &args.app, args.global);
        session.set_global(args.global);
        collect_version_rows(session, &names, &mut rows)?;
    }

    // Nothing installed at all: say so once. For named apps an empty table
    // already follows their per-app error, so no second line is added.
    if rows.is_empty() {
        if all {
            output::info(hok_i18n::t!("cmd.list_no_apps"));
        }
        return Ok(());
    }

    output::header(hok_i18n::t!("cmd.header_installed_apps"));
    render_versions(&rows);

    Ok(())
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    if args.list {
        return list_versions(&args, session);
    }

    // --all or '*' overrides any specific app names
    let all = args.all || args.app.iter().any(|a| a == "*");

    // Upstream refuses a bare `cleanup` rather than sweeping every app
    // (`scoop-cleanup.ps1:24`).
    if args.app.is_empty() && !args.all {
        return Err(anyhow::anyhow!(hok_i18n::t!("cmd.cleanup_app_missing")));
    }

    // Upstream requires elevation for any `--global` cleanup, including the
    // sweep-all form (`scoop-cleanup.ps1:26-28`).
    ensure_global(session, args.global, "cleanup")?;

    // Cleanup is a maintenance operation; individual failures should not abort it.
    let options = CleanupOptions {
        ignore_failures: true,
        cache: args.cache,
    };

    // The sweep removes version directories in parallel and reports each one as
    // it goes; its rows are settled even when the sweep itself fails.
    let terminal = std::io::stdout().is_terminal();
    let progress = Mutex::new(Progress::new(terminal));
    let observer = |event: CleanupProgress| progress.lock().unwrap().render(&event);

    let mut results = Vec::new();
    // The cache entries `-k` dropped, across both scopes: upstream prints
    // nothing about them, which made a `-k` sweep that only removed cache look
    // like it had done nothing at all.
    let mut cache_removed = Vec::new();
    // `Some(0)` means every named app was reported missing/broken: the global
    // "nothing to clean" line would then contradict the per-app errors.
    let mut named_targets = None;

    let sweep = (|| -> Result<()> {
        if all {
            // Local first, then global — upstream appends the global apps to the
            // local list (`scoop-cleanup.ps1:67-70`). The app list is always
            // explicit, so a named cleanup that resolved to nothing cannot fall
            // back to a full sweep.
            session.set_global(false);
            let names = package::cleanup::installed_app_names(session);
            let outcome = package::cleanup::cleanup(session, &names, &options, &observer)?;
            results.extend(outcome.apps);
            cache_removed.extend(outcome.cache_removed);
            if args.global {
                session.set_global(true);
                let names = package::cleanup::installed_app_names(session);
                let outcome = package::cleanup::cleanup(session, &names, &options, &observer)?;
                results.extend(outcome.apps);
                cache_removed.extend(outcome.cache_removed);
            }
        } else {
            // Named apps go through upstream's `Confirm-InstallationStatus`
            // (`lib/core.ps1:1084-1121`): unusable names are reported and
            // dropped, the rest are cleaned in the requested scope.
            let names =
                package::cleanup::confirm_installation_status(session, &args.app, args.global);
            named_targets = Some(names.len());
            session.set_global(args.global);
            let outcome = package::cleanup::cleanup(session, &names, &options, &observer)?;
            results.extend(outcome.apps);
            cache_removed.extend(outcome.cache_removed);
        }
        Ok(())
    })();

    progress.lock().unwrap().finish();
    sweep?;

    // The settled lines. A terminal already rendered them as rows; a pipe has
    // no rows, so every line is rendered here, in the requested order.
    if !terminal {
        for result in &results {
            render_settled(result);
        }
    }

    // Versions that survived are the one thing worth repeating at the end: the
    // rows scroll away, this line survives a scrollback or a CI log. Upstream
    // aborts the whole sweep on the first failure; hok finishes it and reports
    // here, so the exit code stays 0.
    let failed: Vec<&str> = results
        .iter()
        .filter(|result| !result.failures.is_empty())
        .map(|result| result.name.as_str())
        .collect();
    let reported = results.iter().any(|result| !result.removed.is_empty());

    if !failed.is_empty() {
        output::warn(hok_i18n::t!(
            "cmd.cleanup_failed_summary",
            count = failed.len(),
            apps = failed.join(", ")
        ));
    } else if all {
        // Upstream closes the sweep-all form with this line
        // (`scoop-cleanup.ps1:83-85`); the named form has no summary.
        output::done(hok_i18n::t!("cmd.cleanup_all_done"));
    } else if !reported && named_targets != Some(0) {
        output::info(hok_i18n::t!("cmd.cleanup_no_old"));
    }

    if args.cache {
        // One line for what `-k` freed. Upstream is silent here, so the only
        // visible effect of a sweep that removed no version dirs was the
        // absence of output — indistinguishable from a command that did
        // nothing. Printed even when nothing went, so silence always means "no
        // `-k`".
        let total_size: u64 = cache_removed.iter().map(cache::CacheFile::size).sum();
        output::info(hok_i18n::t!(
            "cmd.cleanup_cache_removed",
            count = cache_removed.len(),
            size = humansize(total_size, true)
        ));

        // Upstream clears leftover partial downloads once, after the sweep
        // (`scoop-cleanup.ps1:79-81`).
        cache::remove_incomplete(session)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, version: &str, note: &str, dimmed: bool) -> VersionRow {
        VersionRow {
            name: name.to_owned(),
            version: version.to_owned(),
            note: note.to_owned(),
            dimmed,
        }
    }

    /// `line` without its ANSI SGR sequences, so widths can be measured.
    fn plain(line: &str) -> String {
        let mut out = String::new();
        let mut escaping = false;
        for ch in line.chars() {
            match ch {
                '\x1b' => escaping = true,
                'm' if escaping => escaping = false,
                _ if !escaping => out.push(ch),
                _ => {}
            }
        }
        out
    }

    /// `-l`'s selection rule: nothing named (or `-a`/`*`) means every app,
    /// while the destructive path keeps refusing a bare `cleanup`.
    #[test]
    fn list_selection_defaults_to_every_app() {
        let parse = |argv: &[&str]| Args::try_parse_from(argv.iter().copied()).expect("parse");

        assert!(lists_every_app(&parse(&["cleanup", "-l"])));
        assert!(lists_every_app(&parse(&["cleanup", "-l", "-a"])));
        assert!(lists_every_app(&parse(&["cleanup", "-l", "*"])));
        assert!(!lists_every_app(&parse(&["cleanup", "-l", "demo"])));
        assert!(!lists_every_app(&parse(&[
            "cleanup", "-l", "demo", "other"
        ])));

        // `-l` and `-k` cannot be combined, and the destructive form keeps its
        // usage error at the command layer.
        assert!(Args::try_parse_from(["cleanup", "-l", "-k"]).is_err());
        assert!(Args::try_parse_from(["cleanup", "-l"]).is_ok());
    }

    /// The version column starts one gutter after the widest name (CJK names
    /// included), the note column follows the version, and a row without a
    /// note ends at the version column instead of trailing spaces.
    #[test]
    fn layout_versions_aligns_and_trims_empty_notes() {
        let rows = vec![
            row("7zip", "2.0.0", "", false),
            row("", "1.9.0", "旧版本", true),
            row("directoryopus-cn", "13.25.3", "旧版本", true),
        ];
        let lines = layout_versions(&rows, ["名称", "版本", "状态"])
            .iter()
            .map(|line| plain(line))
            .collect::<Vec<_>>();

        let version_col = 2 + visual_width("directoryopus-cn") + 2;
        let note_col = version_col + visual_width("13.25.3") + 2;
        for (line, version) in lines[2..].iter().zip(["2.0.0", "1.9.0", "13.25.3"]) {
            let at = line.find(version).expect("the version is on the line");
            assert_eq!(visual_width(&line[..at]), version_col, "on `{line}`");
        }
        for line in &lines[3..] {
            let at = line.find("旧版本").expect("the note is on the line");
            assert_eq!(visual_width(&line[..at]), note_col, "on `{line}`");
        }

        assert!(
            lines[2].ends_with("2.0.0"),
            "a note-less row ends at the version column: {}",
            lines[2]
        );
        assert!(
            lines.iter().all(|line| !line.ends_with(' ')),
            "trailing spaces: {lines:#?}"
        );
    }
}
