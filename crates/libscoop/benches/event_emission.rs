//! Criterion benchmarks for the fine-grained (traced-only) event emission path
//! (phase 3b decision support).
//!
//! Phase 3b proposes *source-side gating* of the fine-grained events that the
//! CLI already routes to `tracing::debug!` (shim / shortcut / env / persist /
//! ps-module sub-steps). Before churning ~30 emission sites, measure the actual
//! per-event cost of constructing + transmitting one such event through the
//! same bounded bus the real `EventBus` uses, so the maintainer can judge
//! whether the "waste" is material.
//!
//! The producer constructs `Event::PackageShimAddProgress` and `try_send`s it
//! into a `tokio::sync::mpsc` channel of capacity 20 — the exact type and cap
//! of `EventBus`'s outbound channel (`EVENT_BUS_CAPACITY`). A drainer thread
//! consumes from the other end, standing in for the CLI event loop.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use tokio::sync::mpsc;

use libscoop::Event;

/// The bus capacity used by the real `EventBus` (`EVENT_BUS_CAPACITY`).
const BUS_CAP: usize = 20;

/// A representative traced-only event: shim creation progress for one file.
fn traced_event(name: &str) -> Event {
    Event::PackageShimAddProgress(name.to_owned())
}

/// Construct the event only (no channel) — the allocation cost gating skips.
fn bench_event_construct(c: &mut Criterion) {
    c.bench_function("event/construct_traced_progress", |b| {
        b.iter(|| black_box(traced_event(black_box("app.exe"))))
    });
}

/// Construct + `try_send` into the bounded bus (cap 20) with an active drainer
/// standing in for the event loop — the full cost source-side gating removes.
fn bench_event_try_send(c: &mut Criterion) {
    c.bench_function("event/traced_progress_try_send", |b| {
        let (tx, mut rx) = mpsc::channel::<Event>(BUS_CAP);
        let drainer = std::thread::spawn(move || {
            // Consume as fast as the event loop does between renders; exit when
            // the producer drops its sender (channel disconnects).
            loop {
                match rx.try_recv() {
                    Ok(_) => continue,
                    Err(mpsc::error::TryRecvError::Empty) => std::thread::yield_now(),
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                }
            }
        });
        b.iter(|| {
            let _ = tx.try_send(traced_event("app.exe"));
        });
        drop(tx);
        let _ = drainer.join();
    });
}

criterion_group!(event_benches, bench_event_construct, bench_event_try_send,);
criterion_main!(event_benches);
