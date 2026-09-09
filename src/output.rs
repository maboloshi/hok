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
static SHOW_STEPS: AtomicBool = AtomicBool::new(false);

/// Enable or disable colored output.
pub fn set_color_enabled(enabled: bool) {
    COLOR_ENABLED.store(enabled, Ordering::Relaxed);
    // crossterm respects NO_COLOR env var by default; force the override
    crossterm::style::force_color_output(enabled);
}

/// Whether verbose step output is enabled (the global `-v` flag and above).
pub fn show_steps() -> bool {
    SHOW_STEPS.load(Ordering::Relaxed)
}

/// Turn verbose step output on/off (mirrors [`set_color_enabled`]; set once at
/// startup from the global verbosity level).
pub fn set_show_steps(enabled: bool) {
    SHOW_STEPS.store(enabled, Ordering::Relaxed);
}

/// Render a verbose "verb + object" lifecycle step line (the `-v` expansion).
///
/// `sentence` is the fully localized phrase (the `object` already substituted,
/// e.g. `Running pre_install script` or `解压 7zip-24.09.7z`). The parts before
/// and after the `object` are grey (notes colour) so the line reads as
/// subordinate detail under the package commit header; the `object` itself is
/// emitted in the default foreground so the concrete target stands out. When
/// `object` is absent (phrase-only lines) the whole sentence is grey.
///
/// These lines only appear when verbose step output is enabled; at the default
/// verbosity nothing is printed here.
pub fn step(sentence: impl AsRef<str>, object: Option<impl AsRef<str>>) {
    if !show_steps() {
        return;
    }
    let sentence = sentence.as_ref();
    match object {
        Some(o) => {
            let o = o.as_ref();
            if let Some(i) = sentence.find(o)
                && !o.is_empty()
            {
                println!(
                    "  {}{}{}",
                    sentence[..i].dark_grey(),
                    o,
                    sentence[i + o.len()..].dark_grey()
                );
            } else {
                println!("  {}", sentence.dark_grey());
            }
        }
        None => println!("  {}", sentence.dark_grey()),
    }
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

/// Which PowerShell stream a script output line belongs to (drives its colour).
#[derive(Clone, Copy)]
pub enum ScriptStream {
    /// Success / information stream — shown in the default foreground.
    Info,
    /// Warning stream — shown in yellow.
    Warning,
    /// Error stream / stderr — shown in red.
    Error,
}

/// Colour a single script output line by the stream it came from, for the
/// `  {line}` rendering the frontend uses for package scripts.
///
/// A line that already carries script-embedded ANSI (contains `ESC`) is passed
/// through untouched so we never double-wrap and corrupt the script's own
/// colours; only lines without their own colour get the conventional colour
/// for their stream (error red / warning yellow / stdout stays default).
pub fn script_line(stream: ScriptStream, text: impl AsRef<str>) -> String {
    let text = text.as_ref();
    if text.contains('\x1b') {
        text.to_string()
    } else {
        match stream {
            ScriptStream::Info => text.to_string(),
            ScriptStream::Warning => text.yellow().to_string(),
            ScriptStream::Error => text.red().to_string(),
        }
    }
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

/// Read a 0-based selection index in `0..count` after the localized selection
/// prompt. Returns `None` on EOF / empty / non-numeric / out-of-range input so
/// the caller decides how to report an invalid choice (shared by the
/// "choose among multiple matches" flows of `cat` / `home` / `depends`).
pub fn prompt_select(count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    print!("\n{} ", rust_i18n::t!("output.select_prompt"));
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return None;
    }
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let index = trimmed.parse::<usize>().ok()?;
    (index < count).then_some(index)
}
/// Inline progress with flush (paired with ok/err/done).
/// Example: progress("Holding", "7zip") → "  Holding 7zip... "
pub fn progress(action: impl AsRef<str>, target: impl AsRef<str>) {
    print!("  {} {}... ", action.as_ref(), target.as_ref());
    let _ = std::io::stdout().flush();
}
