//! Unified CLI output — a single renderer with one constant style.
//!
//! Pretty stdout output is deliberately minimal and constant across all
//! commands. Detailed, granular debugging is routed to the tracing log
//! channel instead (driven by -v / -vv / -vvv), so stdout never grows a
//! second, noisier path. There is no scoop/pacman dual style.
//!
//! # Color palette
//!
//! Every colour in the CLI resolves through one role in [`style`]; no command
//! picks its own ANSI colour. The roles use only the normal-intensity ANSI set
//! (36/32/33/31/90) plus bold-on-default-foreground for emphasis, so the same
//! output stays legible on both dark and light terminal themes.
//!
//! | Role     | ANSI                     | Used for                             |
//! |----------|--------------------------|--------------------------------------|
//! | accent   | dark cyan (36)           | `::` headers, labels, arrows, bar    |
//! | emphasis | bold, default foreground | package / entity names               |
//! | success  | dark green (32)          | `ok`, `✓`, versions                  |
//! | warning  | dark yellow (33)         | `WARN` prefix, warnings              |
//! | danger   | dark red (31)            | `ERROR` prefix, `✗`, failures        |
//! | dim      | dark grey (90)           | counters (k/N), notes, steps, rules  |
//!
//! Why not the obvious names: crossterm's `white`/`grey` are ANSI 97/37 and its
//! `red`/`green`/`yellow`/`cyan`/`blue` are the *bright* 91/92/93/96/94 — all of
//! which wash out on a light terminal theme — while ANSI 34 (`dark_blue`) is
//! too dark on a dark theme. The roles above avoid both failure modes.
//!
//! Color can be disabled via --no-color, NO_COLOR, or config. The roles gate on
//! that flag themselves and return plain text when it is off, so no stray SGR
//! escape (not even bold) reaches piped output.
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

/// Whether colored output is enabled (see [`set_color_enabled`]).
pub fn color_enabled() -> bool {
    COLOR_ENABLED.load(Ordering::Relaxed)
}

/// The single output palette: one role per semantic element.
///
/// Every command colours text through these functions instead of calling
/// crossterm's `Stylize` directly, so the whole CLI shares one scheme and no
/// command can introduce a theme-dependent colour. Each role is a no-op when
/// colour output is disabled, returning the plain text.
pub mod style {
    use crossterm::style::{Attribute, Color, Stylize};

    /// Paint `msg` with an optional foreground colour and/or bold, or return
    /// it unchanged when colour output is off.
    fn paint(msg: &str, color: Option<Color>, bold: bool) -> String {
        if !super::color_enabled() {
            return msg.to_owned();
        }
        let mut styled = msg.stylize();
        if let Some(color) = color {
            styled = styled.with(color);
        }
        if bold {
            styled = styled.attribute(Attribute::Bold);
        }
        styled.to_string()
    }

    /// Accent: `::` section headers, field labels, arrows, progress-bar fill.
    pub fn accent(msg: impl AsRef<str>) -> String {
        paint(msg.as_ref(), Some(Color::DarkCyan), false)
    }

    /// Accent, bold: `::` section headers and field labels.
    pub fn accent_bold(msg: impl AsRef<str>) -> String {
        paint(msg.as_ref(), Some(Color::DarkCyan), true)
    }

    /// Emphasis: package / entity names. Bold on the default foreground, so it
    /// reads as black-on-light and white-on-dark.
    pub fn emphasis(msg: impl AsRef<str>) -> String {
        paint(msg.as_ref(), None, true)
    }

    /// Success: `ok`, `✓`, a new version.
    pub fn success(msg: impl AsRef<str>) -> String {
        paint(msg.as_ref(), Some(Color::DarkGreen), false)
    }

    /// Warning text.
    pub fn warning(msg: impl AsRef<str>) -> String {
        paint(msg.as_ref(), Some(Color::DarkYellow), false)
    }

    /// Warning, bold: the `WARN` prefix.
    pub fn warning_bold(msg: impl AsRef<str>) -> String {
        paint(msg.as_ref(), Some(Color::DarkYellow), true)
    }

    /// Failure text.
    pub fn danger(msg: impl AsRef<str>) -> String {
        paint(msg.as_ref(), Some(Color::DarkRed), false)
    }

    /// Failure, bold: the `ERROR` prefix and the `✗` cross.
    pub fn danger_bold(msg: impl AsRef<str>) -> String {
        paint(msg.as_ref(), Some(Color::DarkRed), true)
    }

    /// Dim: counters (`k/N`), notes, steps, table rules, dimmed rows.
    pub fn dim(msg: impl AsRef<str>) -> String {
        paint(msg.as_ref(), Some(Color::DarkGrey), false)
    }
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
                    style::dim(&sentence[..i]),
                    o,
                    style::dim(&sentence[i + o.len()..])
                );
            } else {
                println!("  {}", style::dim(sentence));
            }
        }
        None => println!("  {}", style::dim(sentence)),
    }
}

/// Section heading: `:: resolving...` (accent bold).
pub fn header(msg: impl AsRef<str>) {
    println!(
        "\n{} {}",
        style::accent_bold("::"),
        style::accent_bold(msg.as_ref())
    );
}

/// Informational side note (dim).
pub fn info(msg: impl AsRef<str>) {
    println!("{}", style::dim(msg.as_ref()));
}

/// Warning message (yellow bold prefix, to stderr).
pub fn warn(msg: impl AsRef<str>) {
    eprintln!("{} {m}", style::warning_bold("WARN"), m = msg.as_ref());
}

/// Error message (red bold prefix, to stderr).
pub fn err(msg: impl AsRef<str>) {
    eprintln!("{} {m}", style::danger_bold("ERROR"), m = msg.as_ref());
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
            ScriptStream::Warning => style::warning(text),
            ScriptStream::Error => style::danger(text),
        }
    }
}

/// Status/note line (dim), with a trailing ellipsis for live progress feel.
pub fn status(msg: impl AsRef<str>) {
    println!("{}", status_line(msg));
    let _ = std::io::stdout().flush();
}

/// The dim "  {msg}..." status line as a styled string, without printing.
///
/// Lets a component (e.g. the indicatif download bar `note`) render a status
/// line through its own sink while staying visually identical to [`status`].
pub fn status_line(msg: impl AsRef<str>) -> String {
    format!("  {}...", style::dim(msg.as_ref()))
}
/// Label-value pair (label accent bold).
pub fn field(label: impl AsRef<str>, value: impl AsRef<str>) {
    println!(
        "  {} {}",
        style::accent_bold(label.as_ref()),
        value.as_ref()
    );
}

/// Operation completed with a success marker: check msg (green).
pub fn done(msg: impl AsRef<str>) {
    println!("  {} {}", style::success("✓"), style::success(msg.as_ref()));
}

/// Per-package commit header: `(k/N) name` with an optional `: old -> new`
/// version change.
///
/// The counting counter (dim) anchors the package in the commit sequence; the
/// name is emphasized (bold default foreground); versions are success green;
/// the arrow is accent. When `old` and `new` are both empty (e.g. uninstall,
/// which has no version change) it renders just `(k/N) name`.
pub fn commit_header(
    index: usize,
    total: usize,
    pkg: impl AsRef<str>,
    old: impl AsRef<str>,
    new: impl AsRef<str>,
) {
    let counter = style::dim(format!("({index}/{total})"));
    let name = style::emphasis(pkg.as_ref());
    if new.as_ref().is_empty() {
        println!("{counter} {name}");
    } else if old.as_ref().is_empty() {
        // Fresh install: `(k/N) name: new`.
        println!("{counter} {name}: {}", style::success(new.as_ref()));
    } else {
        // Upgrade: `(k/N) name: old -> new`.
        println!(
            "{counter} {name}: {} {} {}",
            style::success(old.as_ref()),
            style::accent("->"),
            style::success(new.as_ref())
        );
    }
}

/// Per-package success closure: `  ✓ pkg` (success check + emphasized name).
///
/// The bare closure of the commit phase — the counter lives on the header
/// rendered by [`commit_header`], so this line stays `✓ pkg` for grep-able
/// pipe output.
pub fn closure_done(pkg: impl AsRef<str>) {
    println!("{} {}", style::success("✓"), style::emphasis(pkg.as_ref()));
}

/// Per-package failure closure: `  ✗ pkg (err)`.
///
/// Like [`closure_done`] but with a bold red cross and the error message in
/// red — the failure counterpart of the per-package commit closure.
pub fn closure_fail(pkg: impl AsRef<str>, err: impl AsRef<str>) {
    println!(
        "{} {} ({})",
        style::danger_bold("✗"),
        style::emphasis(pkg.as_ref()),
        style::danger(err.as_ref())
    );
}

/// Print "ok".
pub fn ok() {
    println!("  {}", style::success(hok_i18n::t!("output.ok")));
}

/// Named entity + message (name emphasized).
pub fn named(name: impl AsRef<str>, msg: impl AsRef<str>) {
    println!("  {} {}", style::emphasis(name.as_ref()), msg.as_ref());
}

/// Change indicator `label -> value` (label accent bold, arrow accent, value
/// success green).
pub fn change(label: impl AsRef<str>, _op: impl AsRef<str>, value: impl AsRef<str>) {
    println!(
        "  {} {} {}",
        style::accent_bold(label.as_ref()),
        style::accent("->"),
        style::success(value.as_ref())
    );
}

/// Per-package counting action line: `(k/N) verb pkg`.
///
/// The counter is dim; the package name is emphasized so each line answers
/// "which package + what action + result" in one readable row.
#[allow(dead_code)] // reserved for the per-package counting rows (phases ②③)
pub fn action(k: usize, n: usize, verb: impl AsRef<str>, pkg: impl AsRef<str>) {
    println!(
        "{} {} {}",
        style::dim(format!("({k}/{n})")),
        verb.as_ref(),
        style::emphasis(pkg.as_ref())
    );
}

/// Prompt user to continue or not.
pub fn prompt_yes_no() -> bool {
    loop {
        print!("\n{} ", hok_i18n::t!("output.confirm_continue"));
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
    print!("\n{} ", hok_i18n::t!("output.select_prompt"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The palette reads a process-global flag, so colour tests serialize.
    static LOCK: Mutex<()> = Mutex::new(());

    /// Run `f` with colour forced on, restoring it afterwards.
    fn with_color<T>(f: impl FnOnce() -> T) -> T {
        set_color_enabled(true);
        let out = f();
        set_color_enabled(true);
        out
    }

    /// Run `f` with colour forced off, restoring it afterwards.
    fn without_color<T>(f: impl FnOnce() -> T) -> T {
        set_color_enabled(false);
        let out = f();
        set_color_enabled(true);
        out
    }

    /// The palette must stay on the normal-intensity ANSI set (36/32/33/31/90)
    /// plus bold-on-default: the bright variants (37/97/91/92/93/96/94) and
    /// ANSI 34 wash out on one terminal theme or the other.
    #[test]
    fn roles_use_theme_relative_colors() {
        let _guard = LOCK.lock().unwrap();
        with_color(|| {
            assert!(style::accent("x").contains("38;5;6m"), "accent = dark cyan");
            assert!(
                style::success("x").contains("38;5;2m"),
                "success = dark green"
            );
            assert!(
                style::warning("x").contains("38;5;3m"),
                "warning = dark yellow"
            );
            assert!(style::danger("x").contains("38;5;1m"), "danger = dark red");
            assert!(style::dim("x").contains("38;5;8m"), "dim = dark grey");

            // Emphasis is bold on the default foreground, never a white/grey fg.
            let emphasis = style::emphasis("x");
            assert!(emphasis.contains("\x1b[1m"), "emphasis = bold");
            assert!(!emphasis.contains("38;5;"), "emphasis keeps default fg");

            // No role may reach for a bright/theme-dependent colour.
            let all = [
                style::accent("x"),
                style::accent_bold("x"),
                style::emphasis("x"),
                style::success("x"),
                style::warning("x"),
                style::warning_bold("x"),
                style::danger("x"),
                style::danger_bold("x"),
                style::dim("x"),
            ];
            for forbidden in [
                "38;5;7m",  // grey (37)
                "38;5;15m", // white (97)
                "38;5;9m",  // bright red (91)
                "38;5;10m", // bright green (92)
                "38;5;11m", // bright yellow (93)
                "38;5;12m", // bright blue (94)
                "38;5;14m", // bright cyan (96)
                "38;5;4m",  // dark blue (34)
            ] {
                assert!(
                    all.iter().all(|s| !s.contains(forbidden)),
                    "palette must not emit {forbidden}"
                );
            }
        });
    }

    /// With colour off every role returns plain text — including bold, which
    /// crossterm's own `Stylize` would leak into piped output.
    #[test]
    fn roles_are_plain_without_color() {
        let _guard = LOCK.lock().unwrap();
        without_color(|| {
            for styled in [
                style::accent("pkg"),
                style::accent_bold("pkg"),
                style::emphasis("pkg"),
                style::success("pkg"),
                style::warning("pkg"),
                style::warning_bold("pkg"),
                style::danger("pkg"),
                style::danger_bold("pkg"),
                style::dim("pkg"),
            ] {
                assert_eq!(styled, "pkg");
            }
        });
    }
}
