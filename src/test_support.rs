//! Helpers shared by this crate's unit tests.
//!
//! Test-only: `mod test_support` is gated on `cfg(test)` in `lib.rs`, so the
//! integration tests under `tests/` (which link a non-`cfg(test)` build) cannot
//! see it and keep their own harness in `tests/common/mod.rs`.

/// `line` without its ANSI SGR sequences, so rendered output can be asserted
/// as plain text and widths can be measured.
///
/// This was copied into every `cmd::*` test module that renders a table.
pub fn plain(line: &str) -> String {
    let mut out = String::new();
    let mut escaping = false;
    for ch in line.chars() {
        match ch {
            '\x1b' => escaping = true,
            'm' if escaping => escaping = false,
            _ if !escaping => out.push(ch),
            _ => {}
        }
    }
    out
}

/// [`plain`] applied to every line.
pub fn plain_lines(lines: &[String]) -> Vec<String> {
    lines.iter().map(|line| plain(line)).collect()
}
