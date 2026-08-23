//! Console UI components for progress display.
//!
//! Provides a download-progress UI system for different phases of operations:
//!
//! - [`MultiProgressUI`] — Download progress bars powered by `indicatif`.
//!   Supports multiple concurrent downloads, each tracked by an identifier.
//!   Progress is aggregated per-identifier when the same package downloads
//!   from multiple URLs.
//!
//! # Design
//!
//! - **No event loop coupling**: These are pure UI components. The event
//!   loop (`eventloop.rs`) calls into them; they do not know about events.
//! - **Best-effort rendering**: All `stdout` writes use `let _ = ...` /
//!   `.unwrap()`; rendering failures are silently ignored since they are
//!   non-fatal to the operation.
//!
//! # Important Notes
//!
//! - `MultiProgressUI::update()` can be called from any thread; `indicatif`
//!   handles internal synchronisation.

use crossterm::style::Stylize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashMap;

static BAR_FMT: &str = " {wide_msg} {total_bytes:>12} [{bar:>20}] {percent:>3}%";

/// Multiple progress bars with own context.
pub struct MultiProgressUI {
    mp: MultiProgress,
    ctx: HashMap<String, HashMap<String, (u64, u64)>>,
    bars: HashMap<String, ProgressBar>,
}

impl MultiProgressUI {
    pub fn new() -> MultiProgressUI {
        MultiProgressUI {
            mp: MultiProgress::new(),
            ctx: HashMap::new(),
            bars: HashMap::new(),
        }
    }

    /// Update progress bar with the given context.
    pub fn update(&mut self, ident: String, url: String, dltotal: u64, dlnow: u64) {
        if dltotal == 0 {
            return;
        }

        let mut total = 0;
        let mut now = 0;

        self.ctx
            .entry(ident.clone())
            .and_modify(|inner| {
                inner.insert(url.clone(), (dltotal, dlnow));
            })
            .or_insert({
                let mut ctx = HashMap::new();
                ctx.insert(url.clone(), (dltotal, dlnow));
                ctx
            })
            .iter()
            .for_each(|(_, (t, n))| {
                total += t;
                now += n;
            });

        self.bars
            .entry(ident.clone())
            .and_modify(|bar| {
                bar.set_length(total);
                bar.set_position(now);

                if total == now {
                    bar.finish();
                }
            })
            .or_insert_with(|| {
                let bar = self.mp.add(ProgressBar::new(total));
                bar.set_message(ident.clone());
                bar.set_position(0);
                bar.set_style(
                    ProgressStyle::default_bar()
                        .template(BAR_FMT)
                        .unwrap()
                        .progress_chars("#> "),
                );
                bar
            });
    }
}

/// Spinner rows for a concurrent bucket sync.
///
/// Buckets pull concurrently (`bucket::update` spawns one task per bucket), so
/// each bucket gets its own spinner row that finishes in place. A plain inline
/// status/done pair would interleave the "Updating..."/"updated." lines when
/// buckets complete out of order; per-row spinners keep each bucket result
/// on its own line regardless of completion order.
pub struct BucketUpdateUI {
    mp: MultiProgress,
    bars: HashMap<String, ProgressBar>,
}

impl BucketUpdateUI {
    pub fn new() -> BucketUpdateUI {
        BucketUpdateUI {
            mp: MultiProgress::new(),
            bars: HashMap::new(),
        }
    }

    /// Start a spinner row for the named bucket.
    pub fn start(&mut self, name: &str) {
        let bar = self.mp.add(ProgressBar::new_spinner());
        bar.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {msg}")
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        bar.set_message(name.to_owned());
        bar.enable_steady_tick(std::time::Duration::from_millis(80));
        self.bars.insert(name.to_owned(), bar);
    }

    /// Finish the named bucket spinner with a success marker.
    pub fn succeed(&mut self, name: &str) {
        if let Some(bar) = self.bars.remove(name) {
            bar.set_style(ProgressStyle::with_template("{msg}").unwrap());
            bar.finish_with_message(format!("{} {}", "✓".green(), name));
        }
    }

    /// Finish the named bucket spinner with a failure marker and message.
    pub fn fail(&mut self, name: &str, msg: &str) {
        if let Some(bar) = self.bars.remove(name) {
            bar.set_style(ProgressStyle::with_template("{msg}").unwrap());
            bar.finish_with_message(format!("{} {} ({})", "✗".red(), name, msg));
        }
    }

    /// Finish and clear any spinner rows still running.
    pub fn finish_all(&mut self) {
        for (_, bar) in self.bars.drain() {
            bar.finish_and_clear();
        }
    }
}
