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
//! # Function overview (Scoop / Pacman)
//!
//! | Function    | Scoop mode                 | Pacman mode                      |
//! |-------------|----------------------------|----------------------------------|
//! | `header`    | `\nTitle`                  | `\n:: Title` (cyan :: bold)      |
//! | `info`      | `  msg` (green)            | `  ✓ msg` (✓ bold, msg green)   |
//! | `warn`      | `WARN msg` (yellow stderr) | `  ⚠ msg` (⚠ bold, msg yellow) |
//! | `err`       | `ERROR msg` (red stderr)   | `  ✗ msg` (✗ bold, msg red)    |
//! | `status`    | `  msg...` (inline flush)  | `  msg` (grey)                   |
//! | `done`      | `  msg` (green)            | `  ✓ msg` (✓ bold, msg green)   |
//! | `ok`        | `ok` (green)               | `  ok` (green)                   |
//! | `detail`    | `  msg` (always visible)   | `  · msg` (· bold, only --detail)|
//! | `field`     | `  label: value`           | `  label` (cyan bold) `value`    |
//! | `named`     | `  name msg`               | `  name` (blue bold) `msg`       |
//! | `change`    | `  label -> value`         | `  label` (blue bold) `value`    |
//! | `progress`  | `  action target...` (flush)| same (Scoop-only style)         |
//! | `prompt`    | `\nmsg ` (no newline)      | same                             |

use crossterm::style::Stylize;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

const STYLE_SCOOP: u8 = 0;
const STYLE_PACMAN: u8 = 1;

static OUTPUT_STYLE: AtomicU8 = AtomicU8::new(STYLE_SCOOP);
static COLOR_ENABLED: AtomicBool = AtomicBool::new(true);

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

/// Whether color output is currently enabled.
#[allow(dead_code)]
pub fn is_color_enabled() -> bool {
    COLOR_ENABLED.load(Ordering::Relaxed)
}

fn is_scoop() -> bool {
    OUTPUT_STYLE.load(Ordering::Relaxed) == STYLE_SCOOP
}
/// Check if currently in scoop output style.
#[allow(dead_code)]
pub fn is_scoop_style() -> bool {
    is_scoop()
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

/// Status/progress line (printed as-is, no trailing newline in scoop mode).
pub fn status(msg: impl AsRef<str>) {
    let m = msg.as_ref();
    if is_scoop() {
        println!("  {m}...");
        let _ = std::io::stdout().flush();
    } else {
        println!("  {}", m.dark_grey());
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

/// Operation completed (paired with status in scoop mode: "... done.").
pub fn done(msg: impl AsRef<str>) {
    let m = msg.as_ref();
    if is_scoop() {
        println!("  {}", m.green());
    } else {
        println!("  {} {}", "✓".dark_green().bold(), m.dark_green());
    }
}

/// Print "ok".
pub fn ok() {
    if is_scoop() {
        println!("{}", rust_i18n::t!("output.ok").green());
    } else {
        println!("  {}", rust_i18n::t!("output.ok").dark_green());
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
        std::io::stdin().read_line(&mut input).unwrap();
        let c = input.trim_end();
        if c.chars().count() == 1 {
            let ch: char = c.chars().next().unwrap();
            if ['y', 'Y', 'n', 'N'].contains(&ch) {
                return ch == 'y' || ch == 'Y';
            }
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
