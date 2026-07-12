//! Unified CLI output with switchable style.
//!
//! Supported styles:
//! - `scoop`: Step-by-step "message... done." (Scoop original style, default)
//! - `pacman`: Section headers with `::`, checkmark/icon prefixes
//!
//! Switch via: `hok config set output-style pacman`

use crossterm::style::Stylize;
use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};

const STYLE_SCOOP: u8 = 0;
const STYLE_PACMAN: u8 = 1;

static OUTPUT_STYLE: AtomicU8 = AtomicU8::new(STYLE_SCOOP);

/// Set output style by name ("scoop" or "pacman").
pub fn set_style(name: &str) {
    let val = match name {
        "pacman" => STYLE_PACMAN,
        _ => STYLE_SCOOP,
    };
    OUTPUT_STYLE.store(val, Ordering::Relaxed);
}

fn is_scoop() -> bool {
    OUTPUT_STYLE.load(Ordering::Relaxed) == STYLE_SCOOP
}

/// Section heading.
pub fn header(msg: impl AsRef<str>) {
    let m = msg.as_ref();
    if is_scoop() { println!("\n{m}") }
    else { println!("\n{} {}", "::".dark_cyan().bold(), m.dark_cyan().bold()) }
}

/// Informational message.
pub fn info(msg: impl AsRef<str>) {
    let m = msg.as_ref();
    if is_scoop() { println!("{m}") }
    else { println!("  {} {}", "✓".dark_green().bold(), m.dark_green().bold()) }
}

/// Warning message.
pub fn warn(msg: impl AsRef<str>) {
    let m = msg.as_ref();
    if is_scoop() { eprintln!("WARN {m}") }
    else { println!("  {} {}", "⚠".dark_yellow().bold(), m.dark_yellow().bold()) }
}

/// Error message to stderr.
pub fn err(msg: impl AsRef<str>) {
    let m = msg.as_ref();
    if is_scoop() { eprintln!("ERROR {m}") }
    else { eprintln!("  {} {}", "✗".dark_red().bold(), m) }
}

/// Status/progress line (printed as-is, no trailing newline in scoop mode).
pub fn status(msg: impl AsRef<str>) {
    let m = msg.as_ref();
    if is_scoop() { print!("  {m}... "); let _ = std::io::stdout().flush(); }
    else { println!("  {}", m.dark_grey().bold()) }
}

/// Label-value pair.
pub fn field(label: impl AsRef<str>, value: impl AsRef<str>) {
    if is_scoop() { println!("  {}: {}", label.as_ref(), value.as_ref()) }
    else { println!("  {} {}", label.as_ref().dark_cyan().bold(), value.as_ref()) }
}

/// Operation completed (paired with status in scoop mode: "... done.").
pub fn done(msg: impl AsRef<str>) {
    if is_scoop() { println!("done.") }
    else { let m = msg.as_ref(); println!("  {} {}", "✓".dark_green().bold(), m.dark_green().bold()) }
}

/// Print "ok".
pub fn ok() {
    if is_scoop() { println!("ok") }
    else { println!("  {}", "ok".dark_green().bold()) }
}

/// Named entity + message.
pub fn named(name: impl AsRef<str>, msg: impl AsRef<str>) {
    if is_scoop() { println!("  {} {}", name.as_ref(), msg.as_ref()) }
    else { println!("  {} {}", name.as_ref().dark_blue().bold(), msg.as_ref()) }
}

/// Change indicator (label -> value).
pub fn change(label: impl AsRef<str>, _op: impl AsRef<str>, value: impl AsRef<str>) {
    if is_scoop() { println!("  {} -> {}", label.as_ref(), value.as_ref()) }
    else { println!("  {} {}", label.as_ref().dark_blue().bold(), value.as_ref()) }
}

/// Detailed debug info (only shown with --detail flag in pacman mode;
/// always shown as plain text in scoop mode for step visibility).
pub fn detail(msg: impl AsRef<str>) {
    let m = msg.as_ref();
    if is_scoop() { println!("  {m}") }
    else if crate::is_detail() { println!("  {} {}", "·".bold(), m) }
}
