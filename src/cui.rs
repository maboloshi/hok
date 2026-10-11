//! Console UI components for progress display.
//!
//! Provides [`RowUI`] — a shared row-based status UI powered by a single
//! `indicatif::MultiProgress`. Both download progress bars and spinner rows
//! are rows in the same `MultiProgress`, so they stack vertically aligned
//! instead of overwriting each other.
//!
//! The download stage's aggregate bar is [`DownloadBar`] — also an
//! `indicatif::MultiProgress`, so one-off status lines emitted while it is
//! active (e.g. cache hits) are drawn *above* the bar and never collide with it.
//!
//! # Design
//!
//! - **No event loop coupling**: These are pure UI components. The event
//!   loop (`eventloop.rs`) calls into them; they do not know about events.
//! - **Best-effort rendering**: All `stdout` writes use `let _ = ...`;
//!   rendering failures are silently ignored since they are non-fatal.
//! - **Thread-safe**: `RowUI` can be driven from any thread; `indicatif`
//!   handles internal synchronisation.

use crate::output::style;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use libscoop::package::download::{PackageVerifyProgressContext, VerifyState};
use std::collections::HashMap;
use std::io::IsTerminal;
use std::time::Duration;

/// Shared row-based status UI.
///
/// Rows are keyed by an identifier (`id`). A row is created lazily by
/// [`start`](RowUI::start) (a spinner), updated in place, and closed by
/// [`done`](RowUI::done) / [`fail`](RowUI::fail).
///
/// For the download stage's *single* cross-package progress line, see
/// [`DownloadBar`], a separate indicatif `MultiProgress`.
pub struct RowUI {
    mp: MultiProgress,
    bars: HashMap<String, ProgressBar>,
    labels: HashMap<String, String>,
}

impl RowUI {
    pub fn new() -> RowUI {
        RowUI {
            mp: MultiProgress::new(),
            bars: HashMap::new(),
            labels: HashMap::new(),
        }
    }

    /// Rows whose *finished lines are the command's output*, so every row and
    /// every [`println`](RowUI::println) call lands on **stdout**.
    ///
    /// [`new`](RowUI::new) inherits `MultiProgress`'s stderr default, which is
    /// right for a progress indicator that accompanies a stdout transcript
    /// (`bucket update`) but wrong for a report command: the results would go to
    /// stderr and a redirected run would lose them. `indicatif` still renders
    /// nothing off a TTY, so a pipe is unaffected either way.
    pub fn on_stdout() -> RowUI {
        RowUI {
            mp: MultiProgress::with_draw_target(ProgressDrawTarget::stdout()),
            bars: HashMap::new(),
            labels: HashMap::new(),
        }
    }

    /// Start a spinner row for `id` (no-op if already running).
    pub fn start(&mut self, id: &str) {
        if self.bars.contains_key(id) {
            return;
        }
        let bar = self.mp.add(ProgressBar::new_spinner());
        bar.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {msg}")
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        bar.set_message(id.to_owned());
        bar.enable_steady_tick(Duration::from_millis(80));
        self.labels.insert(id.to_owned(), id.to_owned());
        self.bars.insert(id.to_owned(), bar);
    }

    /// Update (or lazily create) the spinner row `id` with a new message.
    pub fn set_spin(&mut self, id: &str, msg: impl AsRef<str>) {
        if let Some(bar) = self.bars.get(id) {
            bar.set_message(msg.as_ref().to_owned());
        } else {
            self.start(id);
            if let Some(bar) = self.bars.get(id) {
                bar.set_message(msg.as_ref().to_owned());
            }
        }
    }

    /// Close `id` with a success marker.
    pub fn done(&mut self, id: &str) {
        let label = self
            .labels
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.to_owned());
        self.done_with(id, &label);
    }

    /// Close `id` with a custom success message.
    pub fn done_with(&mut self, id: &str, msg: impl AsRef<str>) {
        if let Some(bar) = self.bars.remove(id) {
            self.labels.remove(id);
            bar.set_style(ProgressStyle::with_template("{msg}").unwrap());
            bar.finish_with_message(format!(
                "{} {}",
                style::success("✓"),
                style::success(msg.as_ref())
            ));
        }
    }

    /// Close `id` with a failure marker and message.
    pub fn fail(&mut self, id: &str, err: &str) {
        if let Some(bar) = self.bars.remove(id) {
            let label = self.labels.remove(id).unwrap_or_else(|| id.to_owned());
            bar.set_style(ProgressStyle::with_template("{msg}").unwrap());
            bar.finish_with_message(format!(
                "{} {} ({})",
                style::danger_bold("✗"),
                style::emphasis(label),
                style::danger(err)
            ));
        }
    }

    /// Finish and clear any rows still running.
    pub fn finish_all(&mut self) {
        for (_, bar) in self.bars.drain() {
            bar.finish_and_clear();
        }
        self.labels.clear();
    }

    /// Print a standalone note above the rows, without disturbing them.
    ///
    /// Used for a bucket that was never attempted (no spinner row was opened
    /// for it): `indicatif` places the line above every live row.
    pub fn note(&self, msg: impl AsRef<str>) {
        let _ = self.mp.println(format!("  {}", style::dim(msg.as_ref())));
    }

    /// Print an already-rendered line above the rows, verbatim.
    ///
    /// The raw counterpart of [`note`](RowUI::note): no indent and no dim, so a
    /// finished result line keeps the colours its own renderer chose
    /// (`output::result_line` and friends).
    ///
    /// ⚠️ `indicatif` renders *nothing* when the stream is not a terminal, so
    /// this swallows the line off a TTY — a caller that must keep its output in
    /// a pipe has to fall back to `println!` (that is what `BucketRows::Plain`
    /// exists for).
    pub fn println(&self, line: impl AsRef<str>) {
        let _ = self.mp.println(line.as_ref());
    }
}

/// Bucket-update progress rows: live spinners on a terminal, plain lines in a
/// pipe.
///
/// `indicatif` renders *nothing* when the stream is not a terminal
/// ([`ProgressDrawTarget`] drops every draw), so a failed bucket pull would
/// disappear from piped / CI output — the one line that must not be lost. The
/// plain mode therefore prints exactly the rows that carry information
/// (skipped, failed) and stays silent for the running / successful ones, which
/// on a terminal are the transient spinners.
///
/// [`ProgressDrawTarget`]: indicatif::ProgressDrawTarget
pub enum BucketRows {
    /// Live spinner rows, one per bucket (a terminal).
    Rich(RowUI),
    /// Plain lines, failures and skips only (a pipe).
    Plain,
}

impl BucketRows {
    /// Pick the renderer for the current output stream.
    pub fn new(terminal: bool) -> BucketRows {
        if terminal {
            BucketRows::Rich(RowUI::new())
        } else {
            BucketRows::Plain
        }
    }

    /// Start a spinner row for `id` (terminal only).
    pub fn start(&mut self, id: &str) {
        if let BucketRows::Rich(ui) = self {
            ui.start(id);
        }
    }

    /// Finish `id` successfully (terminal only).
    pub fn done(&mut self, id: &str) {
        if let BucketRows::Rich(ui) = self {
            ui.done(id);
        }
    }

    /// Report a failed bucket update.
    ///
    /// `err` is the raw backend error (the terminal row renders it in
    /// parentheses); `message` is the localized one-line description the plain
    /// mode prints.
    pub fn fail(&mut self, id: &str, err: &str, message: &str) {
        match self {
            BucketRows::Rich(ui) => ui.fail(id, err),
            BucketRows::Plain => {
                println!("  {} {}", style::danger_bold("✗"), style::danger(message))
            }
        }
    }

    /// Report a bucket that was not updated at all, in either mode.
    pub fn skip(&self, message: &str) {
        match self {
            BucketRows::Rich(ui) => ui.note(message),
            BucketRows::Plain => println!("  {}", style::dim(message)),
        }
    }

    /// Finish and clear any rows still running (terminal only).
    pub fn finish_all(&mut self) {
        if let BucketRows::Rich(ui) = self {
            ui.finish_all();
        }
    }
}

/// Single aggregate download line driven by an indicatif [`MultiProgress`].
///
/// Used by the download stage to show one self-updating line like:
///
///   downloading 1/3 · verifying 1/5  3.60 GB ████████░░░░░░░░ 61%  ▸ git ✓ · ffmpeg 45%
///
/// Verification runs *inside* the download phase (each file is hashed as it
/// lands), so the line carries its state instead of letting status lines fight
/// for the terminal. The `· verifying n/m` segment only appears when the batch
/// hashes files inline, and a package leaves the line only after it passes or
/// fails — with verification off it leaves when it finishes downloading, which
/// is the original behaviour.
///
/// The line is registered as a row in a `MultiProgress` and recomposed on every
/// progress event, so the layout is fully controlled while indicatif owns the
/// row redraw. One-off status lines emitted while the bar is active (cache
/// hits, notes) go through [`note`](DownloadBar::note), which draws them
/// *above* the bar via `MultiProgress::println` instead of colliding with it.
///
/// Call [`begin`], then [`update`] / [`update_verify`] on the progress events,
/// then [`finish`] to move off the line.
pub struct DownloadBar {
    /// The owning multi-progress; the bar is its only row and notes stack above.
    mp: MultiProgress,
    /// The single aggregate row, present only while a download is in progress.
    bar: Option<ProgressBar>,
    /// ident -> total bytes to download.
    pkgs: Vec<(String, u64)>,
    /// ident -> bytes downloaded so far.
    done: HashMap<String, u64>,
    /// Files this batch will hash inline (0 = the `· verifying` segment is off).
    verify_total: u32,
    /// Files hashed so far across the batch.
    verify_done: u32,
    /// ident -> verification state, from its first verify event on.
    verify: HashMap<String, VerifyState>,
}

impl DownloadBar {
    pub fn new() -> DownloadBar {
        DownloadBar {
            mp: MultiProgress::new(),
            bar: None,
            pkgs: Vec::new(),
            done: HashMap::new(),
            verify_total: 0,
            verify_done: 0,
            verify: HashMap::new(),
        }
    }

    /// Whether progress bars will be shown at all (i.e. stdout is a terminal).
    ///
    /// Off a TTY indicatif renders nothing, so the event loop can fall back to
    /// the handler's plain status lines instead of swallowing them.
    pub fn visible() -> bool {
        std::io::stdout().is_terminal()
    }

    /// Whether an aggregate row is currently being drawn.
    pub fn is_active(&self) -> bool {
        self.bar.is_some()
    }

    /// Start drawing the download row for `packages` (`(ident, total_bytes)`),
    /// showing `verify_files` as the `· verifying 0/n` denominator.
    ///
    /// A no-op (no row is created) when `packages` is empty or stdout is not a
    /// terminal — indicatif renders nothing off a TTY anyway, so skipping avoids
    /// an inert row.
    pub fn begin(&mut self, packages: &[(String, u64)], verify_files: u32) {
        self.pkgs.clear();
        self.done.clear();
        self.verify.clear();
        self.verify_total = verify_files;
        self.verify_done = 0;
        if packages.is_empty() || !std::io::stdout().is_terminal() {
            self.bar = None;
            return;
        }
        self.pkgs = packages.to_vec();
        if self.bar.is_none() {
            let bar = self.mp.add(ProgressBar::new_spinner());
            // Show only our composed message; no spinner, no indicatif bar art.
            bar.set_style(ProgressStyle::with_template("{msg}").unwrap());
            // Drive periodic redraws (the row has no position to change, so a
            // steady tick is what makes indicatif repaint on each update).
            bar.enable_steady_tick(Duration::from_millis(100));
            self.bar = Some(bar);
        }
        self.render();
    }

    /// Report that `ident` has `done` bytes downloaded (package cumulative).
    pub fn update(&mut self, ident: &str, done: u64) {
        if !self.is_active() {
            return;
        }
        self.done.insert(ident.to_owned(), done);
        self.render();
    }

    /// Fold one inline-verification event into the line: the batch counter and
    /// the package's state (checking / passed / failed).
    pub fn update_verify(&mut self, ctx: &PackageVerifyProgressContext) {
        if !self.is_active() {
            return;
        }
        // The backend computes both, so its numbers win: they stay consistent
        // with what the hashing tasks actually counted.
        self.verify_done = ctx.done;
        self.verify_total = ctx.total;
        self.verify.insert(ctx.ident.clone(), ctx.state);
        self.render();
    }

    /// Draw a one-off status line *above* the active bar.
    ///
    /// Uses `MultiProgress::println`, which indicatif places above all its rows,
    /// so concurrent status output (e.g. a cache hit for another package) never
    /// shares the bar's terminal row.
    pub fn note(&self, msg: impl AsRef<str>) {
        if self.is_active() {
            let _ = self.mp.println(msg.as_ref());
        }
    }

    /// Finish the row (remove it) and reset. No-op if not active.
    pub fn finish(&mut self) {
        if let Some(bar) = self.bar.take() {
            bar.finish_and_clear();
        }
        self.pkgs.clear();
        self.done.clear();
        self.verify.clear();
        self.verify_done = 0;
        self.verify_total = 0;
    }

    /// Recompute and push the composed line into the indicatif row.
    fn render(&self) {
        if let Some(bar) = &self.bar {
            bar.set_message(self.line());
        }
    }

    /// The composed line, independent of the terminal so it can be tested:
    /// `  downloading 1/2 · verifying 3/4 15 MB ████░░ 45% ▸ git ✓ · node 45%`.
    fn line(&self) -> String {
        let grand: u64 = self.pkgs.iter().map(|(_, t)| t).sum();
        let done_sum: u64 = self.done.values().sum();
        let total = self.pkgs.len();

        // The `n/m` pair counts packages still downloading; the tail lists
        // every package that has not finished hashing. With verification off
        // the tail keeps the original behaviour: a package leaves the line as
        // soon as its bytes are in.
        let mut downloading = 0usize;
        let mut visible: Vec<(&str, u64, u64)> = Vec::new();
        for (id, t) in &self.pkgs {
            let d = self.done.get(id.as_str()).copied().unwrap_or(0);
            if d < *t {
                downloading += 1;
            } else if self.verify_total == 0 {
                continue;
            }
            visible.push((id.as_str(), d, *t));
        }
        visible.sort_by(|a, b| {
            let pa = a.1 as f64 / a.2.max(1) as f64;
            let pb = b.1 as f64 / b.2.max(1) as f64;
            pb.partial_cmp(&pa).unwrap_or(std::cmp::Ordering::Equal)
        });

        // Overall percent (byte-based: it belongs to the download phase).
        let pct = (done_sum * 100).checked_div(grand).unwrap_or(0);

        // Package states: percentage while the bytes are still arriving, then
        // the verification result — `git ✓ · node ✗ · ffmpeg 45%`.
        let list = visible
            .iter()
            .map(|(id, d, t)| {
                // 只取 '/' 之后的部分作为包名，若无 '/' 则保留原字符串
                let pkg_name = id.split_once('/').map(|(_, name)| name).unwrap_or(id);
                let name = style::accent(pkg_name);
                match self.verify.get(*id) {
                    // A failure wins over the percentage: a dead download never
                    // reaches its byte total, so the byte-based branch below
                    // would freeze it on a stale percentage instead of ✗.
                    Some(VerifyState::Failed) => format!("{} {}", name, style::danger("✗")),
                    _ if d < t => format!("{} {p}%", name, p = (*d * 100) / (*t).max(1)),
                    Some(VerifyState::Passed) => format!("{} {}", name, style::success("✓")),
                    // Bytes are in; the hash result has not arrived yet.
                    _ => format!("{} {}", name, style::dim(hok_i18n::t!("detail.verifying"))),
                }
            })
            .collect::<Vec<_>>()
            .join(" \u{00b7} ");

        let label = hok_i18n::t!("cmd.downloading").to_string();
        // `· verifying 1/5` — only while this batch hashes files inline.
        let verify_segment = if self.verify_total > 0 {
            format!(
                " \u{00b7} {}",
                hok_i18n::t!(
                    "cmd.verifying_count",
                    done = self.verify_done,
                    total = self.verify_total
                )
            )
        } else {
            String::new()
        };

        // Bar: accent-filled blocks + dim remaining.
        const BAR_W: u64 = 16;
        let filled = (pct * BAR_W) / 100;
        let filled = filled.min(BAR_W);
        let bar_txt = format!(
            "{}{}",
            style::accent("█".repeat(filled as usize)),
            style::dim("░".repeat((BAR_W - filled) as usize)),
        );

        let total_bytes = crate::format::humansize(grand, true);
        let tail = if list.is_empty() {
            String::new()
        } else {
            format!(" \u{25b8} {list}")
        };
        // Compact single line: label count/total · verifying segment · total
        // bytes · bar · pct · list, separated by single spaces (no width
        // padding) so it stays tight.
        format!(
            "  {label} {downloading}/{total}{verify_segment} {total_bytes} {bar_txt} {pct}%{tail}",
            label = label,
            downloading = downloading,
            total = total,
            verify_segment = verify_segment,
            total_bytes = total_bytes,
            bar_txt = bar_txt,
            pct = pct,
            tail = tail,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bar in a known state: `git` fully downloaded and verified, `7zip`
    /// still downloading, with `verify_total` files in the batch.
    fn prepared_bar() -> DownloadBar {
        let mut bar = DownloadBar::new();
        bar.pkgs = vec![("main/git".to_owned(), 100), ("main/7zip".to_owned(), 100)];
        bar.done.insert("main/git".to_owned(), 100);
        bar.done.insert("main/7zip".to_owned(), 45);
        bar.verify_total = 4;
        bar.verify_done = 3;
        bar.verify
            .insert("main/git".to_owned(), VerifyState::Passed);
        bar
    }

    /// The one-line shape: downloading count, verifying counter, the finished
    /// package's mark and the downloading package's percentage. The words are
    /// localized, so only the structural markers are asserted.
    #[test]
    fn line_folds_verification_into_the_download_line() {
        let line = prepared_bar().line();
        assert!(line.contains("1/2"), "downloading count: {line}");
        assert!(line.contains("3/4"), "verifying counter: {line}");
        assert!(line.contains('✓'), "passed package marked: {line}");
        assert!(line.contains("45%"), "downloading package keeps %: {line}");
    }

    /// A package whose bytes are in but whose result has not arrived yet is
    /// shown as being verified, not silently dropped.
    #[test]
    fn line_keeps_a_package_until_it_is_verified() {
        let mut bar = prepared_bar();
        bar.verify
            .insert("main/git".to_owned(), VerifyState::Checking);
        let line = bar.line();
        assert!(line.contains("git"), "still listed: {line}");
        assert!(!line.contains('✓'), "no result yet: {line}");

        bar.verify
            .insert("main/git".to_owned(), VerifyState::Failed);
        let line = bar.line();
        assert!(line.contains('✗'), "failed package marked: {line}");
    }

    /// A package that died mid-download keeps a percentage below its total —
    /// the mark has to win, or the line would show a stale percentage instead
    /// of ✗.
    #[test]
    fn line_marks_a_download_that_died_below_its_total() {
        let mut bar = prepared_bar();
        bar.verify
            .insert("main/7zip".to_owned(), VerifyState::Failed);
        let line = bar.line();
        assert!(line.contains('✗'), "failed download marked: {line}");
        assert!(!line.contains("45%"), "no stale percentage: {line}");
    }

    /// Without inline verification the line keeps its original shape: no
    /// `· verifying` segment, and a finished package leaves the line.
    #[test]
    fn line_without_verification_matches_the_original_shape() {
        let mut bar = prepared_bar();
        bar.verify_total = 0;
        bar.verify_done = 0;
        bar.verify.clear();
        let line = bar.line();
        assert!(!line.contains("/4"), "no verifying segment: {line}");
        // `git` finished downloading and has no state to display.
        assert!(!line.contains("git"), "finished package left: {line}");
        assert!(line.contains("7zip"), "downloading package stays: {line}");
    }
}
