//! Import installed apps from a Scoopfile — a local path, an http(s) URL, or a
//! UNC path (upstream `libexec/scoop-import.ps1:18-22`).
//!
//! Upstream's Scoopfile and hok's legacy format are both accepted
//! ([`libscoop::package::import`]); the compatible subset is applied and every
//! entry that cannot be replayed is reported with its reason. Skipping is not
//! an error: like upstream, the command keeps going and exits 0 (see
//! `scoop-download.ps1:110-143` for the same batch policy).
//!
//! # Deliberate differences from upstream
//!
//! - `apps[].Info` containing `Install failed` is **skipped**, where upstream
//!   replays the install: the file records that it did not complete.
//! - hok's `bucket add` is git-remote only, so a bucket exported with a local
//!   path (`lib/buckets.ps1:112-115`) is skipped instead of handed to git.
//! - The install runs with `AssumeYes` + `IgnoreFailure` (one non-interactive
//!   restore), where upstream installs app by app without `-y`.
//! - Upstream's `config` keys are mapped onto hok's names and reported when hok
//!   has no equivalent, instead of being written blindly.
//! - A Scoopfile's `hok.config` extension (hok's own keys, written by
//!   `hok export --hok`) is restored verbatim; emitting *upstream's* key names
//!   (`scoop export -c`) is still open and belongs to the config work (S4).

use clap::Parser;
use libscoop::package::import::{self, ConfigKey, ImportFile};
use libscoop::{QueryOption, Session, SyncOption, bucket, config, fs, package};

use crate::{Result, output};

/// Import installed packages from a file
#[derive(Debug, Parser)]
#[clap(arg_required_else_help = true)]
pub struct Args {
    /// Scoopfile to import from: a local path, an http(s) URL, or a UNC path
    file: String,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    let content = read_scoopfile(session, &args.file).await.map_err(|e| {
        anyhow::anyhow!(hok_i18n::t!("cmd.import_read_err", file = args.file, e = e))
    })?;
    let file = import::parse_import_json(&content)
        .map_err(|e| anyhow::anyhow!(hok_i18n::t!("cmd.import_parse_err", e = e)))?;

    // Upstream's order: config, then buckets, then apps.
    let mut report = Report::default();
    apply_config(session, &file, &mut report);
    let bucket_names = add_buckets(session, &file, &mut report).await;

    if file.apps.is_empty() {
        output::warn(hok_i18n::t!("cmd.import_no_pkgs"));
    } else {
        install_apps(session, &file, &bucket_names, &mut report).await;
    }

    output::info(hok_i18n::t!("cmd.import_complete"));
    report.summary();
    Ok(())
}

/// Read the Scoopfile from any of upstream's three inputs. A UNC path is read
/// like a file; only a URL is fetched (upstream `url_manifest` for UNC too,
/// which resolves to a plain read).
async fn read_scoopfile(session: &Session, file: &str) -> Result<String> {
    if ["http://", "https://", "ftp://", "ftps://"]
        .iter()
        .any(|scheme| file.starts_with(scheme))
    {
        return Ok(libscoop::network::download_page(session, file, 120, None).await?);
    }
    Ok(fs::read_to_string(file)?)
}

/// What the import did, and what it did not.
///
/// Skips are printed where they happen (`skip`); install failures are reported
/// by the sync layer as they occur, so they are only counted here for the
/// closing summary — which never changes the exit code.
#[derive(Default)]
struct Report {
    /// Entries submitted to `sync`.
    submitted: usize,
    /// Entries confirmed installed afterwards.
    imported: usize,
    /// Entries deliberately not attempted, each with its printed reason.
    skipped: usize,
}

impl Report {
    fn skip(&mut self, message: impl AsRef<str>) {
        output::warn(message);
        self.skipped += 1;
    }

    fn summary(&self) {
        let failed = self.submitted.saturating_sub(self.imported);
        if failed == 0 && self.skipped == 0 {
            return;
        }
        output::info(hok_i18n::t!(
            "cmd.import_summary",
            imported = self.imported,
            failed = failed,
            skipped = self.skipped
        ));
    }
}

/// Apply both config sections: hok's own keys verbatim first, then upstream's,
/// whose keys are mapped onto hok's (or reported when hok has no equivalent).
fn apply_config(session: &Session, file: &ImportFile, report: &mut Report) {
    for (key, value) in &file.hok_config {
        // Aliases have no scalar form: they live behind the alias API rather
        // than `config set`, so an hok export's alias map is restored entry by
        // entry instead of being reported as unrepresentable.
        if key == "alias" {
            restore_aliases(session, value, report);
            continue;
        }
        let Some(value) = import::config_value_to_string(value) else {
            report.skip(hok_i18n::t!("cmd.import_skip_config_value", key = key));
            continue;
        };
        match config::set(session, key, &value) {
            Ok(()) => output::info(hok_i18n::t!(
                "cmd.import_config_set",
                key = key,
                value = value
            )),
            Err(e) => report.skip(hok_i18n::t!("cmd.import_skip_config_err", key = key, e = e)),
        }
    }

    for (key, value) in &file.config {
        let hok_key = match import::map_config_key(key) {
            ConfigKey::Supported(hok_key) => hok_key,
            ConfigKey::MachineSpecific => {
                report.skip(hok_i18n::t!("cmd.import_skip_config_machine", key = key));
                continue;
            }
            ConfigKey::Unsupported => {
                report.skip(hok_i18n::t!(
                    "cmd.import_skip_config_unsupported",
                    key = key
                ));
                continue;
            }
        };
        let Some(value) = import::config_value_to_string(value) else {
            report.skip(hok_i18n::t!("cmd.import_skip_config_value", key = key));
            continue;
        };
        match config::set(session, hok_key, &value) {
            Ok(()) => output::info(hok_i18n::t!(
                "cmd.import_config_set_mapped",
                key = key,
                hok_key = hok_key,
                value = value
            )),
            Err(e) => report.skip(hok_i18n::t!("cmd.import_skip_config_err", key = key, e = e)),
        }
    }
}

/// Restore an exported alias map (`{ name: command }`) through the alias API.
fn restore_aliases(session: &Session, value: &serde_json::Value, report: &mut Report) {
    let Some(aliases) = value.as_object() else {
        report.skip(hok_i18n::t!("cmd.import_skip_config_value", key = "alias"));
        return;
    };
    for (name, command) in aliases {
        let Some(command) = command.as_str() else {
            report.skip(hok_i18n::t!("cmd.import_skip_alias", name = name));
            continue;
        };
        match config::alias_add(session, name, command) {
            Ok(()) => output::info(hok_i18n::t!(
                "cmd.import_alias_set",
                name = name,
                command = command
            )),
            Err(e) => report.skip(hok_i18n::t!(
                "cmd.import_skip_alias_err",
                name = name,
                e = e
            )),
        }
    }
}

/// Add the Scoopfile's buckets and return every name an app may reference
/// (the ones already present included).
async fn add_buckets(session: &Session, file: &ImportFile, report: &mut Report) -> Vec<String> {
    let mut names: Vec<String> = match bucket::list(session) {
        Ok(buckets) => buckets.iter().map(|b| b.name().to_owned()).collect(),
        Err(e) => {
            report.skip(hok_i18n::t!("cmd.import_skip_bucket_list", e = e));
            Vec::new()
        }
    };

    for entry in &file.buckets {
        if names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&entry.name))
        {
            report.skip(hok_i18n::t!(
                "cmd.import_skip_bucket_exists",
                name = entry.name
            ));
            continue;
        }
        // A source that is not a git remote (upstream writes the local path of
        // a non-git bucket) cannot be cloned by hok. An empty source is left to
        // `bucket add`, which resolves known bucket names from the built-in
        // list — the same fallback upstream has.
        if !entry.source.is_empty() && !import::is_git_remote(&entry.source) {
            report.skip(hok_i18n::t!(
                "cmd.import_skip_bucket_local",
                name = entry.name,
                source = entry.source
            ));
            continue;
        }
        match bucket::add(session, &entry.name, &entry.source).await {
            Ok(()) => {
                output::info(hok_i18n::t!("cmd.import_bucket_added", name = entry.name));
                names.push(entry.name.clone());
            }
            Err(e) => report.skip(hok_i18n::t!(
                "cmd.import_skip_bucket_err",
                name = entry.name,
                e = e
            )),
        }
    }
    names
}

/// One entry that will be submitted to `sync`.
struct PendingApp {
    /// The install query: `bucket/name`, `name@version`, or a manifest URL/path.
    query: String,
    /// The install name, used to confirm the install and to hold afterwards.
    name: String,
    /// Re-hold the app once it is installed.
    hold: bool,
}

/// The session-wide settings one group of entries is installed under.
///
/// Upstream takes `--global` and `--arch` per app; hok holds both on the
/// session, so entries that need different settings cannot share one call.
#[derive(PartialEq, Eq)]
struct GroupKey {
    global: bool,
    architecture: Option<String>,
}

/// Install the Scoopfile's apps, grouped by the session-wide knobs each entry
/// needs: `--global` and `--arch` are per-app upstream but per-session in hok,
/// so a group is installed together under one setting.
async fn install_apps(
    session: &Session,
    file: &ImportFile,
    bucket_names: &[String],
    report: &mut Report,
) {
    let mut groups: Vec<(GroupKey, Vec<PendingApp>)> = Vec::new();

    for app in &file.apps {
        // Upstream replays a failed install as a normal one; hok skips it: the
        // Scoopfile records that the install did not complete, so reinstalling
        // is a guess at best and would report success for a broken app.
        if app.is_failed() {
            report.skip(hok_i18n::t!("cmd.import_skip_app_failed", name = app.name));
            continue;
        }
        if let Some(source) = app.source.as_deref()
            && import::is_unavailable_bucket(source, bucket_names)
        {
            report.skip(hok_i18n::t!(
                "cmd.import_skip_app_bucket",
                name = app.name,
                source = source
            ));
            continue;
        }
        let Some(query) = import::install_query(app, bucket_names) else {
            report.skip(hok_i18n::t!("cmd.import_skip_app_version", name = app.name));
            continue;
        };
        let key = GroupKey {
            global: app.is_global(),
            architecture: app.architecture().map(str::to_owned),
        };
        let entry = PendingApp {
            query,
            name: app.name.clone(),
            hold: app.is_held(),
        };
        match groups.iter_mut().find(|(group, _)| *group == key) {
            Some((_, apps)) => apps.push(entry),
            None => groups.push((key, vec![entry])),
        }
    }

    // Only the entries that survived the filters are announced: the header
    // must not promise packages that were reported as skipped above.
    let total: usize = groups.iter().map(|(_, apps)| apps.len()).sum();
    output::info(hok_i18n::t!("cmd.import_found", count = total));
    report.submitted += total;

    // Groups without an architecture override run first, while the session
    // still carries the architecture the config resolves to: an override is
    // process-wide and cannot be undone through the public API.
    groups.sort_by_key(|(key, _)| key.architecture.is_some());

    let original_global = session.is_global();
    for (key, apps) in groups {
        session.set_global(key.global);
        if let Some(arch) = key.architecture.as_deref()
            && let Err(e) = session.set_default_architecture(arch)
        {
            report.skip(hok_i18n::t!("cmd.import_skip_app_arch", arch = arch, e = e));
            continue;
        }

        let queries: Vec<&str> = apps.iter().map(|app| app.query.as_str()).collect();
        // Non-interactive restore: yes to every prompt, and a failing app must
        // not abort the rest (upstream installs app by app and continues).
        let options = vec![SyncOption::AssumeYes, SyncOption::IgnoreFailure];
        if let Err(e) = package::sync::sync(session, queries, options).await {
            report.skip(hok_i18n::t!("cmd.import_skip_app_err", e = e));
            continue;
        }

        // With `IgnoreFailure` a failed package still leaves the batch
        // successful, so the count comes from what is actually installed — not
        // from what was submitted, which would report broken apps as imported.
        let names: Vec<&str> = apps.iter().map(|app| app.name.as_str()).collect();
        let installed: std::collections::HashSet<String> =
            match package::query::query(session, names, vec![QueryOption::Explicit], true) {
                Ok(packages) => packages
                    .iter()
                    .map(|package| package.name().to_lowercase())
                    .collect(),
                Err(e) => {
                    report.skip(hok_i18n::t!("cmd.import_skip_app_check", e = e));
                    continue;
                }
            };

        for app in &apps {
            if !installed.contains(&app.name.to_lowercase()) {
                continue;
            }
            report.imported += 1;
            if app.hold
                && let Err(e) = package::hold::hold(session, &app.name, true)
            {
                report.skip(hok_i18n::t!("cmd.import_skip_hold", name = app.name, e = e));
            }
        }
    }
    session.set_global(original_global);
}
