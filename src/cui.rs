//! Console UI components for progress display.
//!
//! Provides two independent UI systems for different phases of operations:
//!
//! - [`MultiProgressUI`] — Download progress bars powered by `indicatif`.
//!   Supports multiple concurrent downloads, each tracked by an identifier.
//!   Progress is aggregated per-identifier when the same package downloads
//!   from multiple URLs.
//!
//! - [`BucketUpdateUI`] — Lightweight terminal output for bucket update
//!   status. Uses raw crossterm commands (`Clear`, `MoveToPreviousLine`)
//!   to render a sorted, auto-refreshing list of bucket names with
//!   colour-coded status (green = success, red = failure).
//!
//! # Design
//!
//! - **Separation of concerns**: Download progress uses `indicatif` (rich,
//!   multi-bar); bucket updates use raw terminal commands (simple, fast).
//!   They never share state — each is constructed and used independently.
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
//! - `BucketUpdateUI::draw()` moves the cursor upward after rendering, so
//!   callers must ensure the terminal supports cursor movement.
//! - The bucket list is sorted alphabetically for stable display order.

use crossterm::{
    cursor,
    style::{Print, Stylize},
    terminal::{Clear, ClearType},
    ExecutableCommand,
};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::{collections::HashMap, io::stdout};

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
    pub fn update(&mut self, ident: String, url: String, _: String, dltotal: u64, dlnow: u64) {
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

/// Simple UI for bucket update progress
#[allow(dead_code)]
pub struct BucketUpdateUI {
    pub data: HashMap<String, BucketState>,
    pub cursor: usize,
}

/// Bucket update state
#[allow(dead_code)]
pub enum BucketState {
    Started,
    Failed(String),
    Successed,
}

#[allow(dead_code)]
impl BucketUpdateUI {
    pub fn new() -> BucketUpdateUI {
        BucketUpdateUI {
            data: HashMap::new(),
            cursor: 0,
        }
    }

    /// Add a bucket progress to the UI.
    pub fn add(&mut self, name: &str) {
        self.set_state(name, BucketState::Started);
    }

    /// Set the bucket progress to failed.
    pub fn fail(&mut self, name: &str, msg: &str) {
        self.data
            .insert(name.to_owned(), BucketState::Failed(msg.to_owned()));
        self.draw();
    }

    /// Set the bucket progress to successed.
    pub fn succeed(&mut self, name: &str) {
        self.set_state(name, BucketState::Successed);
    }

    /// Insert state and redraw (shared by `add` / `succeed`).
    fn set_state(&mut self, name: &str, state: BucketState) {
        self.data.insert(name.to_owned(), state);
        self.draw();
    }

    /// Draw the progress to the stdout.
    pub fn draw(&mut self) {
        let mut stdout = stdout();
        let mut sorted = self.data.iter().collect::<Vec<_>>();
        sorted.sort_by_key(|&(k, _)| k.clone());

        for (name, state) in sorted.iter() {
            let _ = match state {
                BucketState::Started => stdout
                    .execute(Clear(ClearType::CurrentLine))
                    .unwrap()
                    .execute(Print(format!("{}\n", name)))
                    .unwrap(),
                BucketState::Successed => stdout
                    .execute(Clear(ClearType::CurrentLine))
                    .unwrap()
                    .execute(Print(format!("{} {}\n", name, "Ok".green())))
                    .unwrap(),
                BucketState::Failed(_err) => stdout
                    .execute(Clear(ClearType::CurrentLine))
                    .unwrap()
                    .execute(Print(format!("{} {}\n", name, "Err".red())))
                    .unwrap(),
            };
        }

        // move cursor back to the first line
        stdout
            .execute(cursor::MoveToPreviousLine(sorted.len() as u16))
            .unwrap();
    }
}
