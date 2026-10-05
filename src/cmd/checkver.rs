//! Check for outdated package manifests.

use clap::Parser;
use libscoop::Session;
use libscoop::package::checkver::{self, CheckverEvent, CheckverReport, ReportSeverity};
use std::cmp::Ordering;
use std::io::IsTerminal;
use std::path::PathBuf;

use crate::Result;
use crate::cmd::shared_args::{scan_dir, validate_dir, write_dir};
use crate::cui::RowUI;
use crate::output::{self, ResultMark, style};

/// The single spinner row's id in the shared [`RowUI`].
const PROGRESS_ROW: &str = "checkver";

/// Check manifest for a newer version
#[derive(Debug, Parser)]
pub struct Args {
    /// Bucket directory to scan for manifests
    ///
    /// Defaults to the current directory, but required with `--update`/
    /// `--force-update` (which rewrite manifests) or hok refuses to run.
    #[arg(short = 'd', long)]
    dir: Option<PathBuf>,

    /// Specific app(s) to check (supports wildcards, default: all)
    #[arg(default_value = "*")]
    app: Vec<String>,

    /// Update manifest with new version and trigger autoupdate
    #[arg(short = 'u', long, action = clap::ArgAction::SetTrue)]
    update: bool,

    /// Force update even when version is unchanged (useful for hash updates)
    #[arg(short = 'f', long, action = clap::ArgAction::SetTrue)]
    force_update: bool,

    /// Only show outdated or failed manifests (skip up-to-date ones)
    #[arg(short = 's', long = "skip-updated", action = clap::ArgAction::SetTrue)]
    skip_updated: bool,

    /// Update manifest to specific version (skip version detection)
    #[arg(short = 'V', long)]
    version: Option<String>,

    /// Request timeout in seconds
    #[arg(short = 't', long, default_value = "30")]
    timeout: u64,

    /// Concurrent manifest checks (0 = no limit)
    #[arg(short = 'j', long, default_value = "8")]
    jobs: usize,

    /// Throw on the first error instead of reporting it and continuing
    #[arg(long = "throw", action = clap::ArgAction::SetTrue)]
    throw_error: bool,
}

pub async fn execute(args: Args, session: &Session) -> Result<()> {
    // `--update` and `--force-update` both rewrite manifests (`--force-update`
    // implies `--update`, like upstream's `if ($ForceUpdate) { $Update = $true }`
    // in `bin/checkver.ps1:404`), so either one demands an explicit target.
    // A plain check only reads, and keeps the current-directory default.
    let dir = if args.update || args.force_update {
        write_dir(args.dir, "checkver")?
    } else {
        scan_dir(args.dir)
    };
    validate_dir(&dir)?;

    // Named packages are reported in manifest order, so the output reads back as
    // the list that was asked for; a whole-bucket sweep (`*`, the default)
    // reports each manifest the moment it lands, which is the fastest feedback
    // and the only order that matters when nobody reads the lines one by one.
    let ordered = args.app.iter().any(|pattern| pattern != "*");

    let options = checkver::CheckverOptions {
        dir,
        app: args.app,
        update: args.update,
        force_update: args.force_update,
        skip_updated: args.skip_updated,
        version: args.version,
        timeout: args.timeout,
        jobs: args.jobs,
        ordered,
        throw_error: args.throw_error,
    };

    // Stream results: each manifest is reported as it is checked, matching
    // Scoop's event-driven output in checkver.ps1.
    let mut sink = ResultSink::new();
    let summary = checkver::execute(options, session, |event| match event {
        CheckverEvent::Progress { done, total } => {
            sink.spin(hok_i18n::t!(
                "cmd.checkver_checking",
                done = done,
                total = total
            ));
        }
        CheckverEvent::Report(report) => render(&sink, report),
    })
    .await?;
    sink.finish();

    // Printed even when nothing needed reporting: "no outdated manifest" and
    // "the scan never ran" must not look the same.
    if summary.checked == 0 {
        output::info(hok_i18n::t!("cmd.checkver_no_manifests"));
    } else {
        output::info(hok_i18n::t!(
            "cmd.checkver_summary",
            total = summary.checked,
            outdated = summary.outdated,
            failed = summary.failed,
            current = summary.up_to_date
        ));
    }

    Ok(())
}

/// Render one manifest's report as a result line.
///
/// Everything goes through the sink, failures included: they used to be `ERROR`
/// lines on stderr, which split the report in two the moment it was piped.
fn render(sink: &ResultSink, r: &CheckverReport) {
    if let Some(message) = &r.message {
        let mark = match r.severity {
            ReportSeverity::Warn => ResultMark::Warn,
            _ => ResultMark::Failed,
        };
        sink.line(output::result_message_line(mark, &r.stem, message));
        return;
    }

    let Some(ver) = &r.new_version else {
        return;
    };

    if ver == &r.current {
        sink.line(output::result_line(
            ResultMark::Ok,
            &r.stem,
            style::success(format!("({ver})")),
        ));
    } else {
        // An upstream version *below* the manifest's is not an update: the arrow
        // follows the version, not the wish.
        let mark = if libscoop::compare_versions(ver, &r.current) == Ordering::Less {
            ResultMark::Downgrade
        } else {
            ResultMark::Updated
        };
        sink.line(output::result_line(
            mark,
            &r.stem,
            version_delta(&r.current, ver),
        ));
    }

    if r.updated {
        sink.line(output::done_line(hok_i18n::t!(
            "cmd.checkver_updated_to",
            ver = ver
        )));
    }
}

/// `(old -> new)`: the old version dim, the arrow accent, the new one green.
fn version_delta(old: &str, new: &str) -> String {
    format!(
        "({} {} {})",
        style::dim(old),
        style::accent("->"),
        style::success(new)
    )
}

/// Where checkver's result lines go.
///
/// On a terminal the lines are drawn *above* a single live spinner row, so the
/// progress line never collides with them and the number of live rows stays 1
/// however many manifests the bucket holds. Off a terminal `indicatif` drops
/// every write, so the lines go straight to stdout — a piped run must still get
/// the whole report.
struct ResultSink {
    rows: Option<RowUI>,
}

impl ResultSink {
    fn new() -> ResultSink {
        ResultSink {
            rows: std::io::stdout().is_terminal().then(RowUI::on_stdout),
        }
    }

    /// Replace the progress row's text.
    fn spin(&mut self, msg: impl AsRef<str>) {
        if let Some(rows) = &mut self.rows {
            rows.set_spin(PROGRESS_ROW, msg.as_ref());
        }
    }

    /// Emit one finished result line.
    fn line(&self, line: String) {
        match &self.rows {
            Some(rows) => rows.println(line),
            None => println!("{line}"),
        }
    }

    /// Drop the progress row.
    fn finish(&mut self) {
        if let Some(rows) = &mut self.rows {
            rows.finish_all();
        }
    }
}
