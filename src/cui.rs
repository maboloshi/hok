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
use std::collections::hash_map::Entry;
use std::time::Duration;

/// Template for download progress bars: cyan fill on a blue track, with
/// partial block characters for a smooth fill.
const BAR_FMT: &str = " {wide_msg} {total_bytes:>12} [{bar:20.cyan/blue}] {percent:>3}%";

/// Shared row-based status UI.
///
/// Rows are keyed by an identifier (`id`). A row is created lazily by
/// [`set_bar`](RowUI::set_bar) (a progress bar) or [`start`](RowUI::start)
/// (a spinner), updated in place, and closed by [`done`](RowUI::done) /
/// [`fail`](RowUI::fail).
pub struct RowUI {
    mp: MultiProgress,
    bars: HashMap<String, ProgressBar>,
    labels: HashMap<String, String>,
    /// Per-id, per-url download totals (aggregated across mirrors).
    ctx: HashMap<String, HashMap<String, (u64, u64)>>,
}

impl RowUI {
    pub fn new() -> RowUI {
        RowUI {
            mp: MultiProgress::new(),
            bars: HashMap::new(),
            labels: HashMap::new(),
            ctx: HashMap::new(),
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
    #[allow(dead_code)] // reserved for per-package spinner rows (phase ②③)
    pub fn set_spin(&mut self, id: &str, msg: &str) {
        if let Some(bar) = self.bars.get(id) {
            bar.set_message(msg.to_owned());
        } else {
            self.start(id);
            if let Some(bar) = self.bars.get(id) {
                bar.set_message(msg.to_owned());
            }
        }
    }

    /// Update a download progress bar for `id`, aggregating concurrent
    /// downloads of the same package from multiple URLs. `url` keys the
    /// per-URL contribution; the bar shows the summed total/now.
    pub fn set_bar(&mut self, id: &str, url: &str, total: u64, now: u64) {
        if total == 0 {
            return;
        }
        let mut agg_total = 0;
        let mut agg_now = 0;
        self.ctx
            .entry(id.to_owned())
            .and_modify(|inner| {
                inner.insert(url.to_owned(), (total, now));
            })
            .or_insert_with(|| {
                let mut inner = HashMap::new();
                inner.insert(url.to_owned(), (total, now));
                inner
            })
            .iter()
            .for_each(|(_, (t, n))| {
                agg_total += t;
                agg_now += n;
            });

        match self.bars.entry(id.to_owned()) {
            Entry::Occupied(mut e) => {
                let bar = e.get_mut();
                bar.set_length(agg_total);
                bar.set_position(agg_now);
                if agg_total == agg_now {
                    bar.finish();
                }
            }
            Entry::Vacant(v) => {
                let label = self
                    .labels
                    .entry(id.to_owned())
                    .or_insert_with(|| id.to_owned())
                    .clone();
                let bar = self.mp.add(ProgressBar::new(agg_total));
                bar.set_message(label);
                bar.set_position(0);
                bar.set_style(
                    ProgressStyle::default_bar()
                        .template(BAR_FMT)
                        .unwrap()
                        .progress_chars("#> "),
                );
                v.insert(bar);
            }
        }
    }

    /// Close `id` with a success marker.
    pub fn done(&mut self, id: &str) {
        if let Some(bar) = self.bars.remove(id) {
            let label = self.labels.remove(id).unwrap_or_else(|| id.to_owned());
            bar.set_style(ProgressStyle::with_template("{msg}").unwrap());
            bar.finish_with_message(format!("{} {}", "✓".green(), label));
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
