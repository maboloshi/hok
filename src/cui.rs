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

use crossterm::style::Stylize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
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
            bar.finish_with_message(format!("{} {}", "✓".green(), msg.as_ref()));
        }
    }

    /// Close `id` with a failure marker and message.
    pub fn fail(&mut self, id: &str, err: &str) {
        if let Some(bar) = self.bars.remove(id) {
            let label = self.labels.remove(id).unwrap_or_else(|| id.to_owned());
            bar.set_style(ProgressStyle::with_template("{msg}").unwrap());
            bar.finish_with_message(format!("{} {} ({})", "✗".red(), label, err));
        }
    }

    /// Finish and clear any rows still running.
    pub fn finish_all(&mut self) {
        for (_, bar) in self.bars.drain() {
            bar.finish_and_clear();
        }
        self.labels.clear();
    }
}

/// Single aggregate download line driven by an indicatif [`MultiProgress`].
///
/// Used by the download stage to show one self-updating line like:
///
///   downloading 2/3   3.60 GB ████████░░░░░░░░ 61%  ▸ git 70% · ffmpeg 45%
///
/// The line is registered as a row in a `MultiProgress` and recomposed on every
/// progress event, so the layout is fully controlled while indicatif owns the
/// row redraw. One-off status lines emitted while the bar is active (cache
/// hits, notes) go through [`note`](DownloadBar::note), which draws them
/// *above* the bar via `MultiProgress::println` instead of colliding with it.
///
/// Call [`begin`], then [`update`] on each progress event, then [`finish`] to
/// move off the line.
pub struct DownloadBar {
    /// The owning multi-progress; the bar is its only row and notes stack above.
    mp: MultiProgress,
    /// The single aggregate row, present only while a download is in progress.
    bar: Option<ProgressBar>,
    /// ident -> total bytes to download.
    pkgs: Vec<(String, u64)>,
    /// ident -> bytes downloaded so far.
    done: HashMap<String, u64>,
}

impl DownloadBar {
    pub fn new() -> DownloadBar {
        DownloadBar {
            mp: MultiProgress::new(),
            bar: None,
            pkgs: Vec::new(),
            done: HashMap::new(),
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

    /// Start drawing the download row for `packages` (`(ident, total_bytes)`).
    ///
    /// A no-op (no row is created) when `packages` is empty or stdout is not a
    /// terminal — indicatif renders nothing off a TTY anyway, so skipping avoids
    /// an inert row.
    pub fn begin(&mut self, packages: &[(String, u64)]) {
        self.pkgs.clear();
        self.done.clear();
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
    }

    /// Recompute and push the composed line into the indicatif row.
    fn render(&self) {
        let Some(bar) = &self.bar else { return };
        let grand: u64 = self.pkgs.iter().map(|(_, t)| t).sum();
        let done_sum: u64 = self.done.values().sum();
        let total = self.pkgs.len();

        // Running (not finished) packages, sorted by percent descending.
        let mut running: Vec<(&str, u64, u64)> = self
            .pkgs
            .iter()
            .filter_map(|(id, t)| {
                let d = self.done.get(id.as_str()).copied().unwrap_or(0);
                if d >= *t {
                    None
                } else {
                    Some((id.as_str(), d, *t))
                }
            })
            .collect();
        running.sort_by(|a, b| {
            let pa = a.1 as f64 / a.2.max(1) as f64;
            let pb = b.1 as f64 / b.2.max(1) as f64;
            pb.partial_cmp(&pa).unwrap_or(std::cmp::Ordering::Equal)
        });
        let running_count = running.len();

        // Overall percent.
        let pct = if grand > 0 {
            (done_sum * 100) / grand
        } else {
            0
        };

        // In-progress package list: `git 78% · node 44%`.
        let list = running
            .iter()
            .map(|(id, d, t)| {
                let p = (*d * 100) / (*t).max(1);
                // 只取 '/' 之后的部分作为包名，若无 '/' 则保留原字符串
                let pkg_name = id.split_once('/').map(|(_, name)| name).unwrap_or(id);
                format!("{} {p}%", pkg_name.cyan())
            })
            .collect::<Vec<_>>()
            .join(" \u{00b7} ");

        let label = rust_i18n::t!("cmd.downloading").to_string();

        // Bar: filled cyan blocks + dark-grey remaining.
        const BAR_W: u64 = 16;
        let filled = ((pct as u64) * BAR_W) / 100;
        let filled = filled.min(BAR_W);
        let bar_txt = format!(
            "{}{}",
            "█".repeat(filled as usize).cyan(),
            "░".repeat((BAR_W - filled) as usize).dark_grey(),
        );

        let total_bytes = crate::format::humansize(grand, true);
        let tail = if list.is_empty() {
            String::new()
        } else {
            format!(" \u{25b8} {list}")
        };
        // Compact single line: label count/total · total bytes · bar · pct · list,
        // separated by single spaces (no width padding) so it stays tight.
        let line = format!(
            "  {label} {running_count}/{total} {total_bytes} {bar_txt} {pct}%{tail}",
            label = label,
            running_count = running_count,
            total = total,
            total_bytes = total_bytes,
            bar_txt = bar_txt,
            pct = pct,
            tail = tail,
        );
        bar.set_message(line);
    }
}
