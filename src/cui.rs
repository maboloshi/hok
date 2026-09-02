//! Console UI components for progress display.
//!
//! Provides [`RowUI`] — a shared row-based status UI powered by a single
//! `indicatif::MultiProgress`. Both download progress bars and spinner rows
//! are rows in the same `MultiProgress`, so they stack vertically aligned
//! instead of overwriting each other.
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
use std::io::Write;
use std::time::Duration;

/// Shared row-based status UI.
///
/// Rows are keyed by an identifier (`id`). A row is created lazily by
/// [`start`](RowUI::start) (a spinner), updated in place, and closed by
/// [`done`](RowUI::done) / [`fail`](RowUI::fail).
///
/// For a *single* cross-package download progress line, see [`DownloadBar`], a
/// hand-drawn `\r`-redrawn bar (no MultiProgress) used by the download stage.
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

/// Hand-drawn single-line aggregate download progress (no MultiProgress).
///
/// Used by the download stage to show one self-updating line like:
///
///   downloading 2/3   3.60 GB ████████░░░░░░░░ 61%  ▸ git 70% · ffmpeg 45%
///
/// Redrawn in place with `\r\x1b[2K` on every progress event, so the layout
/// is fully controlled (unlike indicatif's fixed field-template bars) and a
/// changing label/package list never shifts the bar column. Call [`begin`],
/// then [`update`] on each progress event, then [`finish`] to move off the line.
pub struct DownloadBar {
    /// ident -> total bytes to download.
    pkgs: Vec<(String, u64)>,
    /// ident -> bytes downloaded so far.
    done: HashMap<String, u64>,
    /// Whether a line is currently being drawn.
    active: bool,
}

impl DownloadBar {
    pub fn new() -> DownloadBar {
        DownloadBar {
            pkgs: Vec::new(),
            done: HashMap::new(),
            active: false,
        }
    }

    /// Start drawing a download line for `packages` (`(ident, total_bytes)`).
    pub fn begin(&mut self, packages: &[(String, u64)]) {
        if packages.is_empty() {
            return;
        }
        self.pkgs = packages.to_vec();
        self.done.clear();
        self.active = true;
        self.render();
    }

    /// Report that `ident` has `done` bytes downloaded (package cumulative).
    pub fn update(&mut self, ident: &str, done: u64) {
        if !self.active {
            return;
        }
        self.done.insert(ident.to_owned(), done);
        self.render();
    }

    /// Finish the line (move off it) and reset. No-op if not active.
    pub fn finish(&mut self) {
        if self.active {
            // Redraw a cleared line then move to the next row so subsequent
            // output does not overwrite a stale progress line.
            self.active = false;
            println!();
        }
    }

    /// Recompute and draw the current line in place.
    fn render(&self) {
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
                format!("{} {p}%", id.cyan())
            })
            .collect::<Vec<_>>()
            .join(" \u{00b7} ");

        let label = rust_i18n::t!("cmd.downloading").to_string();

        // Bar: filled cyan blocks + dark-grey remaining.
        const BAR_W: u64 = 16;
        let filled = ((pct as u64) * BAR_W) / 100;
        let filled = filled.min(BAR_W);
        let bar = format!(
            "{}{}",
            "█".repeat(filled as usize).cyan(),
            "░".repeat((BAR_W - filled) as usize).dark_grey()
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
            "  {label} {running_count}/{total} {total_bytes} {bar} {pct}%{tail}",
            label = label,
            running_count = running_count,
            total = total,
            total_bytes = total_bytes,
            bar = bar,
            pct = pct,
            tail = tail,
        );
        print!("\r\x1b[2K{}", line);
        let _ = std::io::stdout().flush();
    }
}
