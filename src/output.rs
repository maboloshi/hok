//! Unified CLI output with consistent coloring.
//! Pacman-inspired hierarchy: :: headers, indented items, minimal icons.

use crossterm::style::Stylize;

/// Section heading with `::` prefix (pacman style).
pub fn header(msg: impl AsRef<str>) {
    println!("\n{} {}", "::".dark_cyan().bold(), msg.as_ref().dark_cyan().bold());
}

/// Success message.
pub fn info(msg: impl AsRef<str>) {
    println!("  {} {}", "✓".dark_green().bold(), msg.as_ref().dark_green().bold());
}

/// Warning message.
pub fn warn(msg: impl AsRef<str>) {
    println!("  {} {}", "⚠".dark_yellow().bold(), msg.as_ref().dark_yellow().bold());
}

/// Error message to stderr.
pub fn err(msg: impl AsRef<str>) {
    eprintln!("  {} {}", "✗".dark_red().bold(), msg.as_ref());
}

/// Indented status line (grey).
pub fn status(msg: impl AsRef<str>) {
    println!("  {}", msg.as_ref().dark_grey().bold());
}

/// Label-value pair.
pub fn field(label: impl AsRef<str>, value: impl AsRef<str>) {
    println!("  {} {}", label.as_ref().dark_cyan().bold(), value.as_ref());
}

/// Operation completed.
pub fn done(msg: impl AsRef<str>) {
    println!("  {} {}", "✓".dark_green().bold(), msg.as_ref().dark_green().bold());
}

/// Print "ok".
pub fn ok() {
    println!("  {}", "ok".dark_green().bold());
}

/// Named entity + message (name in blue).
pub fn named(name: impl AsRef<str>, msg: impl AsRef<str>) {
    println!("  {} {}", name.as_ref().dark_blue().bold(), msg.as_ref());
}

/// Label + value (label in blue).
pub fn change(label: impl AsRef<str>, _op: impl AsRef<str>, value: impl AsRef<str>) {
    println!("  {} {}", label.as_ref().dark_blue().bold(), value.as_ref());
}

/// Detailed debug info (only shown with --detail flag).
pub fn detail(msg: impl AsRef<str>) {
    if crate::is_detail() {
        println!("  {} {}", "·".bold(), msg.as_ref());
    }
}
