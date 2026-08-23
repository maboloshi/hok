//! Unified CLI output with switchable style.
//!
//! Supported styles:
//! - `scoop`: Step-by-step "message... done." (Scoop original style, default)
//! - `pacman`: Section headers with `::`, checkmark/icon prefixes
//!
//! Switch via: `hok config set output-style pacman`
//!
//! Both styles support colored output. Color can be disabled via
//! `--no-color` flag, `NO_COLOR` environment variable, or config.
//!
//! # TTY-aware single-step progress
//!
//! The `status` → `done`/`ok` single-step pairs are TTY-aware. On a TTY the
//! `status` call starts a spinner that is frozen by the paired `done`/`ok`;
//! when stdout is a pipe/redirect (CI) the same calls emit plain line-based
//! text with no ANSI, so progress is never lost. Steps finishing faster than
//! [`MIN_SPIN`] never draw a spinner frame (no noise).
//!
//! Report/data-line output (search/list/status/checkver …) goes through
//! [`status_line`], which always renders plain text and never animates.
//!
//! # Function overview (Scoop / Pacman)
//!
//! | Function      | Scoop mode                 | Pacman mode                      |
//! |---------------|----------------------------|----------------------------------|
//! | `header`      | `\nTitle`                  | `\n:: Title` (cyan :: bold)      |
//! | `info`        | `  msg` (green)            | `  ✓ msg` (✓ bold, msg green)   |
//! | `warn`        | `WARN msg` (yellow stderr) | `  ⚠ msg` (⚠ bold, msg yellow) |
//! | `err`         | `ERROR msg` (red stderr)   | `  ✗ msg` (✗ bold, msg red)    |
//! | `status`      | `  msg` (TTY: spinner)     | `  msg` (grey)                   |
//! | `status_line` | `  msg...`                 | `  msg` (grey)                   |
//! | `done`        | `  msg` (green)            | `  ✓ msg` (✓ bold, msg green)   |
//! | `ok`          | `ok` (green)               | `  ok` (green)                   |
//! | `detail`      | `  msg` (always visible)   | `  · msg` (· bold, only --detail)|
//! | `field`       | `  label: value`           | `  label` (cyan bold) `value`    |
//! | `named`       | `  name msg`               | `  name` (blue bold) `msg`       |
//! | `change`      | `  label -> value`         | `  label` (blue bold) `value`    |
//! | `progress`    | `  action target...` (flush)| same (Scoop-only style)         |
//! | `prompt`      | `\nmsg ` (no newline)      | same                             |

use crossterm::style::Stylize;
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const STYLE_SCOOP: u8 = 0;
const STYLE_PACMAN: u8 = 1;

static OUTPUT_STYLE: AtomicU8 = AtomicU8::new(STYLE_SCOOP);
static COLOR_ENABLED: AtomicBool = AtomicBool::new(true);

/// Spinner frames for single-step progress on a TTY.
const SPINNER_CHARS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const SPINNER_TICK: Duration = Duration::from_millis(80);
/// Steps that finish faster than this never draw a spinner frame (no noise).
const MIN_SPIN: Duration = Duration::from_millis(300);

/// A single in-progress `status` step (only alive while a TTY spinner runs).
struct Step {
    msg: Mutex<String>,
    started: Instant,
    stop: AtomicBool,
}

/// The currently active TTY step, if any. `None` in non-TTY mode and between
/// steps.
static STEP: Mutex<Option<Arc<Step>>> = Mutex::new(None);

/// Output sink. Defaults to stdout; tests swap in a capture buffer.
static OUT: Mutex<Option<Box<dyn Write + Send>>> = Mutex::new(None);

/// Run `f` against the shared output writer (stdout by default).
fn with_out<R>(f: impl FnOnce(&mut dyn Write) -> R) -> R {
    let mut guard = OUT.lock().unwrap();
    if guard.is_none() {
        *guard = Some(Box::new(std::io::stdout()));
    }
    f(guard.as_mut().unwrap().as_mut())
}

/// Whether stdout is a TTY. All TTY/spinner behaviour keys off this.
fn is_tty() -> bool {
    std::io::stdout().is_terminal()
}

/// Set output style by name ("scoop" or "pacman").
pub fn set_style(name: &str) {
    let val = match name {
        "pacman" => STYLE_PACMAN,
        _ => STYLE_SCOOP,
    };
    OUTPUT_STYLE.store(val, Ordering::Relaxed);
}

/// Enable or disable colored output.
pub fn set_color_enabled(enabled: bool) {
    COLOR_ENABLED.store(enabled, Ordering::Relaxed);
    // crossterm respects NO_COLOR env var by default; force the override
    crossterm::style::force_color_output(enabled);
}

fn is_scoop() -> bool {
    OUTPUT_STYLE.load(Ordering::Relaxed) == STYLE_SCOOP
}

/// Section heading.
pub fn header(msg: impl AsRef<str>) {
    let m = msg.as_ref();
    if is_scoop() {
        println!("\n{}", m);
    } else {
        println!("\n{} {}", "::".dark_cyan().bold(), m.dark_cyan());
    }
}

/// Informational message.
pub fn info(msg: impl AsRef<str>) {
    let m = msg.as_ref();
    if is_scoop() {
        println!("{}", m.green());
    } else {
        println!("  {} {}", "✓".dark_green().bold(), m.dark_green());
    }
}

/// Warning message.
pub fn warn(msg: impl AsRef<str>) {
    let m = msg.as_ref();
    if is_scoop() {
        eprintln!("{} {m}", rust_i18n::t!("output.warn").yellow().bold());
    } else {
        println!("  {} {}", "⚠".dark_yellow().bold(), m.dark_yellow());
    }
}

/// Error message to stderr.
pub fn err(msg: impl AsRef<str>) {
    let m = msg.as_ref();
    if is_scoop() {
        eprintln!("{} {m}", rust_i18n::t!("output.error").red().bold());
    } else {
        eprintln!("  {} {}", "✗".dark_red().bold(), m.dark_red());
    }
}

// ── TTY-aware single-step progress ────────────────────────────────────────

/// Redraw the static message line (without a spinner frame) for `step`.
fn draw_step(step: &Step) {
    let msg = step.msg.lock().unwrap().clone();
    with_out(|w| {
        let _ = write!(w, "\r  {msg}");
        let _ = w.flush();
    });
}

/// Start (or, for repeated progress updates, refresh) the current step.
///
/// Called by `status` in scoop+TTY mode. A repeated call while a step is
/// already active is treated as an in-place progress update (e.g. per-file
/// hash checks), refreshing the spinner's message rather than starting a new
/// line.
fn step_begin(msg: &str) {
    let mut guard = STEP.lock().unwrap();
    if let Some(existing) = guard.as_ref() {
        *existing.msg.lock().unwrap() = msg.to_string();
        draw_step(existing);
        return;
    }
    let step = Arc::new(Step {
        msg: Mutex::new(msg.to_string()),
        started: Instant::now(),
        stop: AtomicBool::new(false),
    });
    draw_step(&step);
    spawn_spinner(step.clone());
    *guard = Some(step);
}

/// Background animation for a step's spinner. Draws a frame only after the
/// step has outlived [`MIN_SPIN`], so fast steps stay a static text line.
fn spawn_spinner(step: Arc<Step>) {
    std::thread::spawn(move || {
        loop {
            {
                let msg = step.msg.lock().unwrap();
                if step.stop.load(Ordering::Relaxed) {
                    break;
                }
                let elapsed = step.started.elapsed();
                if elapsed >= MIN_SPIN {
                    let idx = (elapsed.as_millis() / SPINNER_TICK.as_millis()) as usize
                        % SPINNER_CHARS.len();
                    with_out(|w| {
                        let _ = write!(w, "\r  {} {msg}", SPINNER_CHARS[idx]);
                        let _ = w.flush();
                    });
                }
            }
            std::thread::sleep(SPINNER_TICK);
        }
    });
}

/// How an active step is frozen by its paired completion call.
enum StepResult<'a> {
    Done(&'a str),
    Ok,
}

/// Freeze an active spinner (if any) with the given completion marker.
///
/// When no step is active — e.g. report-only `done`/`ok` — renders the
/// plain line instead.
fn step_finish(result: StepResult) {
    let step = STEP.lock().unwrap().take();
    match step {
        Some(step) => {
            step.stop.store(true, Ordering::Relaxed);
            // Wait out any in-flight spinner write so the clear + completion
            // land cleanly, then stop the frame thread (it sees `stop`).
            let _msg = step.msg.lock().unwrap();
            with_out(|w| {
                let _ = write!(w, "\r\x1b[2K");
                match result {
                    StepResult::Done(m) => {
                        let _ = writeln!(w, "  {}", m.green());
                    }
                    StepResult::Ok => {
                        let _ = writeln!(w, "{}", rust_i18n::t!("output.ok").green());
                    }
                }
                let _ = w.flush();
            });
        }
        None => {
            with_out(|w| match result {
                StepResult::Done(m) => {
                    let _ = writeln!(w, "  {}", m.green());
                }
                StepResult::Ok => {
                    let _ = writeln!(w, "{}", rust_i18n::t!("output.ok").green());
                }
            });
        }
    }
}

/// Status/progress line for a single step.
///
/// On a TTY it starts a spinner frozen by the paired `done`/`ok`. On a
/// pipe/redirect it prints a plain line (no ANSI, no animation).
pub fn status(msg: impl AsRef<str>) {
    let m = msg.as_ref();
    if is_scoop() {
        if is_tty() {
            step_begin(m);
        } else {
            with_out(|w| {
                let _ = writeln!(w, "  {m}");
            });
        }
    } else {
        with_out(|w| {
            let _ = writeln!(w, "  {}", m.dark_grey());
        });
    }
}

/// Plain status/report line for report/data output (search/list/status/
/// checkver data lines). Renders exactly like the historical `status`
/// output and never animates, so report output stays byte-stable.
pub fn status_line(msg: impl AsRef<str>) {
    let m = msg.as_ref();
    if is_scoop() {
        with_out(|w| {
            let _ = writeln!(w, "  {m}...");
        });
    } else {
        with_out(|w| {
            let _ = writeln!(w, "  {}", m.dark_grey());
        });
    }
}

/// Operation completed (paired with `status`).
pub fn done(msg: impl AsRef<str>) {
    let m = msg.as_ref();
    if is_scoop() {
        if is_tty() {
            step_finish(StepResult::Done(m));
        } else {
            // Non-TTY: plain text, no ANSI (keeps piped/CI output clean).
            with_out(|w| {
                let _ = writeln!(w, "  {m}");
            });
        }
    } else {
        with_out(|w| {
            let _ = writeln!(w, "  {} {}", "✓".dark_green().bold(), m.dark_green());
        });
    }
}

/// Print "ok".
pub fn ok() {
    if is_scoop() {
        if is_tty() {
            step_finish(StepResult::Ok);
        } else {
            // Non-TTY: plain text, no ANSI.
            with_out(|w| {
                let _ = writeln!(w, "{}", rust_i18n::t!("output.ok"));
            });
        }
    } else {
        with_out(|w| {
            let _ = writeln!(w, "  {}", rust_i18n::t!("output.ok").dark_green());
        });
    }
}

/// Label-value pair.
pub fn field(label: impl AsRef<str>, value: impl AsRef<str>) {
    if is_scoop() {
        println!("  {}: {}", label.as_ref().cyan(), value.as_ref());
    } else {
        println!("  {} {}", label.as_ref().dark_cyan().bold(), value.as_ref());
    }
}

/// Named entity + message.
pub fn named(name: impl AsRef<str>, msg: impl AsRef<str>) {
    if is_scoop() {
        println!("  {} {}", name.as_ref(), msg.as_ref());
    } else {
        println!("  {} {}", name.as_ref().dark_blue().bold(), msg.as_ref());
    }
}

/// Change indicator (label -> value).
pub fn change(label: impl AsRef<str>, _op: impl AsRef<str>, value: impl AsRef<str>) {
    if is_scoop() {
        println!("  {} -> {}", label.as_ref(), value.as_ref());
    } else {
        println!("  {} {}", label.as_ref().dark_blue().bold(), value.as_ref());
    }
}

/// Detailed debug info (only shown with --detail flag in pacman mode;
/// always shown as plain text in scoop mode for step visibility).
pub fn detail(msg: impl AsRef<str>) {
    let m = msg.as_ref();
    if is_scoop() {
        println!("  {m}");
    } else if crate::is_detail() {
        println!("  {} {}", rust_i18n::t!("output.detail_prefix").bold(), m);
    }
}

/// Prompt user to continue or not.
pub fn prompt_yes_no() -> bool {
    loop {
        print!("\n{} ", rust_i18n::t!("output.confirm_continue"));
        std::io::stdout().flush().unwrap();
        let mut input = String::new();
        // EOF (piped/no input) or a read error must not spin forever — a
        // non-interactive confirmation is conservatively "no".
        match std::io::stdin().read_line(&mut input) {
            Ok(0) => return false,
            Ok(_) => {}
            Err(_) => return false,
        }
        let c = input.trim_end();
        if c.chars().count() == 1 {
            let ch: char = c.chars().next().unwrap();
            if ['y', 'Y', 'n', 'N'].contains(&ch) {
                return ch == 'y' || ch == 'Y';
            }
        }
        if c.is_empty() {
            // An empty line is a "no".
            return false;
        }
    }
}

/// Display an interactive prompt message (no trailing newline).
pub fn prompt(msg: impl AsRef<str>) {
    print!("\n{} ", msg.as_ref());
    let _ = std::io::stdout().flush();
}

/// Inline progress with flush (paired with ok/err/done).
/// Example: progress("Holding", "7zip") → "  Holding 7zip... "
pub fn progress(action: impl AsRef<str>, target: impl AsRef<str>) {
    print!("  {} {}... ", action.as_ref(), target.as_ref());
    let _ = std::io::stdout().flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Serialises the output tests that swap the global sink, so concurrent
    /// runs can't clobber each other's capture buffer.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);
    impl Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Run `f` with the output sink redirected to an in-memory buffer and
    /// colour disabled (tests exercise the deterministic non-TTY text path:
    /// cargo test's stdout is a pipe, so `is_tty()` is false).
    fn capture<R>(f: impl FnOnce() -> R) -> (R, String) {
        let _g = TEST_LOCK.lock().unwrap();
        set_color_enabled(false);
        set_style("scoop");
        let shared = Arc::new(Mutex::new(Vec::new()));
        {
            let mut out = OUT.lock().unwrap();
            *out = Some(Box::new(CaptureWriter(shared.clone())));
        }
        let result = f();
        let text = String::from_utf8(shared.lock().unwrap().clone()).unwrap();
        {
            let mut out = OUT.lock().unwrap();
            *out = None;
        }
        (result, text)
    }

    #[test]
    fn status_ok_non_tty() {
        let (_, text) = capture(|| {
            status("Checking hash...");
            ok();
        });
        assert_eq!(text, "  Checking hash...\nok\n");
    }

    #[test]
    fn status_done_non_tty() {
        let (_, text) = capture(|| {
            status("Resolving packages...");
            done("Resolving packages completed.");
        });
        assert_eq!(
            text,
            "  Resolving packages...\n  Resolving packages completed.\n"
        );
    }

    #[test]
    fn status_progress_update_non_tty() {
        let (_, text) = capture(|| {
            status("Checking hash...");
            status("Checking hash...7zip");
            status("Checking hash...git");
            ok();
        });
        assert_eq!(
            text,
            "  Checking hash...\n  Checking hash...7zip\n  Checking hash...git\nok\n"
        );
    }

    #[test]
    fn status_line_matches_historical() {
        let (_, text) = capture(|| {
            status_line("main/git 2.40.0");
            status_line("    autoupdate available");
        });
        assert_eq!(
            text,
            "  main/git 2.40.0...\n      autoupdate available...\n"
        );
    }

    #[test]
    fn done_without_status_is_plain() {
        let (_, text) = capture(|| {
            done("Everything is ok!");
        });
        assert_eq!(text, "  Everything is ok!\n");
    }

    #[test]
    fn ok_after_progress_is_plain() {
        let (_, text) = capture(|| {
            ok();
        });
        assert_eq!(text, "ok\n");
    }
}
