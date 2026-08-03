//! CLI output formatting utilities.
//!
//! Pure presentation helpers — byte-size rendering and visual-width-aware
//! padding for terminal tables. No business logic and no I/O.

// ─── Human-readable size ────────────────────────────────────────────────────

/// Convert bytes to KB/MB/GB representation.
pub fn humansize(length: u64, with_unit: bool) -> String {
    let gb: f64 = 2.0_f64.powf(30_f64);
    let mb: f64 = 2.0_f64.powf(20_f64);
    let kb: f64 = 2.0_f64.powf(10_f64);

    let flength = length as f64;

    if flength > gb {
        let j = (flength / gb).round();

        if with_unit {
            format!("{} GB", j)
        } else {
            j.to_string()
        }
    } else if flength > mb {
        let j = (flength / mb).round();

        if with_unit {
            format!("{} MB", j)
        } else {
            j.to_string()
        }
    } else if flength > kb {
        let j = (flength / kb).round();

        if with_unit {
            format!("{} KB", j)
        } else {
            j.to_string()
        }
    } else if with_unit {
        format!("{} B", flength)
    } else {
        flength.to_string()
    }
}

// ─── Visual width ───────────────────────────────────────────────────────────

/// Approximate visual width of a string in a terminal.
/// CJK characters count as 2, everything else as 1.
pub fn visual_width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            if c >= '\u{1100}' && (c <= '\u{115F}' || c == '\u{2329}' || c == '\u{232A}'
                || ('\u{2E80}'..='\u{9FFF}').contains(&c)
                || ('\u{A000}'..='\u{A4CF}').contains(&c)
                || ('\u{AC00}'..='\u{D7AF}').contains(&c)
                || ('\u{F900}'..='\u{FAFF}').contains(&c)
                || ('\u{FE30}'..='\u{FE6F}').contains(&c)
                || ('\u{FF01}'..='\u{FF60}').contains(&c)
                || ('\u{FFE0}'..='\u{FFE6}').contains(&c))
            {
                2
            } else {
                1
            }
        })
        .sum()
}

/// Pad a string to a target visual width by appending spaces.
pub fn pad_visual(s: &str, width: usize) -> String {
    let vw = visual_width(s);
    if vw >= width {
        s.to_owned()
    } else {
        format!("{}{}", s, " ".repeat(width - vw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── humansize ────────────────────────────────────────────────────────────

    #[test]
    fn humansize_bytes_no_unit() {
        assert_eq!(humansize(100, false), "100");
    }

    #[test]
    fn humansize_bytes_with_unit() {
        assert_eq!(humansize(100, true), "100 B");
    }

    #[test]
    fn humansize_kb_with_unit() {
        assert_eq!(humansize(2 * 1024, true), "2 KB");
    }

    #[test]
    fn humansize_mb_with_unit() {
        assert_eq!(humansize(3 * 1024 * 1024, true), "3 MB");
    }

    #[test]
    fn humansize_gb_with_unit() {
        assert_eq!(humansize(4 * 1024 * 1024 * 1024, true), "4 GB");
    }

    // ── visual_width / pad_visual ────────────────────────────────────────────

    #[test]
    fn visual_width_ascii_is_char_count() {
        assert_eq!(visual_width("curl"), 4);
    }

    #[test]
    fn visual_width_cjk_counts_double() {
        assert_eq!(visual_width("中文"), 4);
        assert_eq!(visual_width("a中b"), 4);
    }

    #[test]
    fn pad_visual_fills_to_width() {
        assert_eq!(pad_visual("ab", 4), "ab  ");
    }

    #[test]
    fn pad_visual_no_pad_when_wider() {
        assert_eq!(pad_visual("abcd", 4), "abcd");
    }

    #[test]
    fn pad_visual_cjk_uses_visual_width() {
        assert_eq!(pad_visual("中", 4), "中  ");
    }
}
