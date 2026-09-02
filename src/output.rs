//! Unified CLI output — a single renderer with one constant style.
//!
//! Pretty stdout output is deliberately minimal and constant across all
//! commands. Detailed, granular debugging is routed to the tracing log
//! channel instead (driven by -v / -vv / -vvv), so stdout never grows a
//! second, noisier path. There is no scoop/pacman dual style.
//!
//! # Color semantics
//!
//! | Semantic            | Style                        |
//! |---------------------|------------------------------|
//! | section header      | :: cyan bold                 |
//! | package name        | white bold                   |
//! | version             | green                        |
//! | arrow               | blue                         |
//! | success check       | green                        |
//! | failure cross / err | red (bold)                   |
//! | warning             | yellow                       |
//! | counter (k/N)       | dark grey                    |
//! | notes / status      | grey                         |
//!
//! Color can be disabled via --no-color, NO_COLOR, or config; crossterm
//! also auto-disables it on non-TTY output.
//!
//! # Primitives
//!
//! - header: section heading.
//! - commit_header: per-package commit header (k/N) pkg [: old -> new].
//! - action: per-package counting line (k/N) verb pkg.
//! - closure_done / closure_fail: per-package success/failure closure.
//! - done / ok: success markers.
//! - info / warn / err: side notes and prefixes.
//! - field / named / change: structured label/value lines.

use crossterm::style::Stylize;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

static COLOR_ENABLED: AtomicBool = AtomicBool::new(true);

/// Enable or disable colored output.
pub fn set_color_enabled(enabled: bool) {
    COLOR_ENABLED.store(enabled, Ordering::Relaxed);
    // crossterm respects NO_COLOR env var by default; force the override
    crossterm::style::force_color_output(enabled);
}

/// Section heading: `:: resolving...` (cyan bold).
pub fn header(msg: impl AsRef<str>) {
    println!(
        "\n{} {}",
        "::".dark_cyan().bold(),
        msg.as_ref().dark_cyan().bold()
    );
}

/// Informational side note (grey).
pub fn info(msg: impl AsRef<str>) {
    println!("{}", msg.as_ref().grey());
}

/// Warning message (yellow, to stderr).
pub fn warn(msg: impl AsRef<str>) {
    eprintln!("{} {m}", "WARN".yellow().bold(), m = msg.as_ref());
}

/// Error message (red bold, to stderr).
pub fn err(msg: impl AsRef<str>) {
    eprintln!("{} {m}", "ERROR".red().bold(), m = msg.as_ref());
}

/// Status/note line (grey), with a trailing ellipsis for live progress feel.
pub fn status(msg: impl AsRef<str>) {
    println!("{}", status_line(msg));
    let _ = std::io::stdout().flush();
}

/// The grey "  {msg}..." status line as a styled string, without printing.
///
/// Lets a component (e.g. the indicatif download bar `note`) render a status
/// line through its own sink while staying visually identical to [`status`].
pub fn status_line(msg: impl AsRef<str>) -> String {
    format!("  {}...", msg.as_ref().grey())
}
/// Label-value pair (label cyan bold).
pub fn field(label: impl AsRef<str>, value: impl AsRef<str>) {
    println!("  {} {}", label.as_ref().dark_cyan().bold(), value.as_ref());
}

/// Operation completed with a success marker: check msg (green).
pub fn done(msg: impl AsRef<str>) {
    println!("  {} {}", "✓".green(), msg.as_ref().green());
}

/// Per-package commit header: `(k/N) name` with an optional `: old -> new`
/// version change.
///
/// The counting counter (dark grey) anchors the package in the commit
/// sequence; the name is white bold; versions green; the arrow blue. When
/// `old` and `new` are both empty (e.g. uninstall, which has no version
/// change) it renders just `(k/N) name`.
pub fn commit_header(
    index: usize,
    total: usize,
    pkg: impl AsRef<str>,
    old: impl AsRef<str>,
    new: impl AsRef<str>,
) {
    let counter = format!("({index}/{total})").dark_grey();
    let name = pkg.as_ref().white().bold();
    if new.as_ref().is_empty() {
        println!("{counter} {name}");
    } else if old.as_ref().is_empty() {
        // Fresh install: `(k/N) name: new`.
        println!("{counter} {name}: {}", new.as_ref().green());
    } else {
        // Upgrade: `(k/N) name: old -> new`.
        println!(
            "{counter} {name}: {} {} {}",
            old.as_ref().green(),
            "->".blue(),
            new.as_ref().green()
        );
    }
}

/// Per-package success closure: `  ✓ pkg` (green check + white bold name).
///
/// The bare closure of the commit phase — the counter lives on the header
/// rendered by [`commit_header`], so this line stays `✓ pkg` for grep-able
/// pipe output.
pub fn closure_done(pkg: impl AsRef<str>) {
    println!("{} {}", "✓".green(), pkg.as_ref().white().bold());
}

/// Per-package failure closure: `  ✗ pkg (err)`.
///
/// Like [`closure_done`] but with a red bold cross and the error message
/// in red — the failure counterpart of the per-package commit closure.
pub fn closure_fail(pkg: impl AsRef<str>, err: impl AsRef<str>) {
    println!(
        "{} {} ({})",
        "✗".red().bold(),
        pkg.as_ref().white().bold(),
        err.as_ref().red()
    );
}

/// Print "ok".
pub fn ok() {
    println!("  {}", rust_i18n::t!("output.ok").green());
}

/// Named entity + message (name white bold).
pub fn named(name: impl AsRef<str>, msg: impl AsRef<str>) {
    println!("  {} {}", name.as_ref().white().bold(), msg.as_ref());
}

/// Change indicator `label -> value` (label blue bold, arrow blue, value green).
pub fn change(label: impl AsRef<str>, _op: impl AsRef<str>, value: impl AsRef<str>) {
    println!(
        "  {} {} {}",
        label.as_ref().dark_blue().bold(),
        "->".blue(),
        value.as_ref().green()
    );
}

/// Per-package counting action line: `(k/N) verb pkg`.
///
/// The counter is dark grey; the package name is white bold so each line
/// answers "which package + what action + result" in one readable row.
#[allow(dead_code)] // reserved for the per-package counting rows (phases ②③)
pub fn action(k: usize, n: usize, verb: impl AsRef<str>, pkg: impl AsRef<str>) {
    println!(
        "{}{} {}",
        format!("({k}/{n})").dark_grey(),
        verb.as_ref(),
        pkg.as_ref().white().bold()
    );
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
