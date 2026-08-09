//! Time utilities — Scoop-compatible `last_update` timestamp codec.
//!
//! Scoop stores bucket-update timestamps as .NET round-trip strings
//! (`[System.DateTime]::Now.ToString('o')`), e.g.
//! `2026-07-19T10:48:34.0100861+08:00` — local wall time with offset and
//! 7 fractional digits. The writer (`bucket::update`) and the reader
//! (`Config::update_cooldown_remaining`) must agree on this exact format;
//! keeping the format/parse pair in one module guarantees they cannot
//! drift apart.
//!
//! # Design
//!
//! - [`format_last_update()`] — encode a [`time::OffsetDateTime`] the way
//!   Scoop writes `LAST_UPDATE` (local wall time + offset via the system
//!   timezone, matching .NET and the official PowerShell implementation).
//! - [`parse_last_update()`] — decode Scoop's value back into a UTC
//!   [`time::OffsetDateTime`], accepting offset round-trip strings, `Z`
//!   suffixes, and bare no-offset values (treated as UTC).

use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};

/// Scoop `LAST_UPDATE` output shape: `YYYY-MM-DDTHH:MM:SS.fffffff±HH:MM`.
const FMT_LAST_UPDATE: &[time::format_description::FormatItem<'_>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:7][offset_hour sign:mandatory]:[offset_minute]");

/// Bare no-offset input shape (treated as UTC): fractional seconds optional.
/// Two formats because a time optional group starting with a literal (`.`)
/// does not fall back when the group is absent (yields `InvalidLiteral`).
const FMT_NAIVE_UTC_NO_FRAC: &[time::format_description::FormatItem<'_>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]");
const FMT_NAIVE_UTC_FRAC: &[time::format_description::FormatItem<'_>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:1+]");

/// Format `now` as Scoop's `last_update` config value.
///
/// Matches PowerShell's `[System.DateTime]::Now.ToString('o')`:
/// `YYYY-MM-DDTHH:MM:SS.fffffff±HH:MM` (local time + offset, 7 fractional
/// digits — 100 ns ticks, truncated).
pub fn format_last_update(now: OffsetDateTime) -> String {
    let local = UtcOffset::local_offset_at(now).unwrap_or(UtcOffset::UTC);
    now.to_offset(local)
        .format(FMT_LAST_UPDATE)
        .expect("static format description cannot fail")
}

/// Parse Scoop's `last_update` config value back into a UTC
/// [`OffsetDateTime`].
///
/// Accepts the round-trip format written by [`format_last_update`] (local
/// time + offset), `Z`-suffixed UTC timestamps, and bare no-offset values
/// (treated as UTC) produced by other tools. Returns `None` when the string
/// cannot be parsed.
pub fn parse_last_update(s: &str) -> Option<OffsetDateTime> {
    if let Ok(dt) = OffsetDateTime::parse(s, &Rfc3339) {
        return Some(dt.to_offset(UtcOffset::UTC));
    }
    if let Ok(dt) = PrimitiveDateTime::parse(s, FMT_NAIVE_UTC_NO_FRAC) {
        return Some(dt.assume_utc());
    }
    PrimitiveDateTime::parse(s, FMT_NAIVE_UTC_FRAC)
        .ok()
        .map(|dt| dt.assume_utc())
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::{Date, Month, Time};

    /// The UTC instant used across parse tests: 2026-07-19T02:48:34Z.
    fn utc_instant() -> OffsetDateTime {
        OffsetDateTime::parse("2026-07-19T02:48:34Z", &Rfc3339).unwrap()
    }

    /// 1970-01-01T00:00:00.{ns}Z (nanosecond not aligned to 100 ns).
    fn instant_with_ns(ns: u32) -> OffsetDateTime {
        PrimitiveDateTime::new(
            Date::from_calendar_date(1970, Month::January, 1).unwrap(),
            Time::from_hms_nano(0, 0, 0, ns).unwrap(),
        )
        .assume_utc()
    }

    // ── format_last_update ───────────────────────────────────────────────────

    #[test]
    fn format_produces_seven_digit_fraction_and_offset() {
        // 123_456_700 ns = 1_234_567 100-ns ticks → ".1234567"
        let s = format_last_update(instant_with_ns(123_456_700));
        let frac_and_offset = s.split('.').nth(1).unwrap();
        assert_eq!(&frac_and_offset[..7], "1234567");
        // 7 fractional digits + ±HH:MM offset
        assert_eq!(frac_and_offset.len(), 13);
    }

    #[test]
    fn format_truncates_to_100ns_ticks() {
        // 123_456_789 ns must truncate to ".1234567" (not round to ".1234568"),
        // matching the previous jiff implementation (`subsec_nanosecond() / 100`).
        let s = format_last_update(instant_with_ns(123_456_789));
        let frac_and_offset = s.split('.').nth(1).unwrap();
        assert_eq!(&frac_and_offset[..7], "1234567");
    }

    // ── parse_last_update ────────────────────────────────────────────────────

    #[test]
    fn parse_accepts_zoned_roundtrip_value() {
        // Local wall time (+08:00) must convert back to the UTC instant.
        let s = "2026-07-19T10:48:34.1234567+08:00";
        let ts = parse_last_update(s).expect("zoned value should parse");
        assert_eq!(ts.unix_timestamp(), utc_instant().unix_timestamp());
        // 1234567 ticks → 123456700 ns (7-digit granularity)
        assert_eq!(ts.nanosecond(), 123_456_700);
    }

    #[test]
    fn parse_accepts_plain_utc_timestamp_fallback() {
        let ts =
            parse_last_update("2026-07-19T02:48:34Z").expect("plain UTC timestamp should parse");
        assert_eq!(ts.unix_timestamp(), utc_instant().unix_timestamp());
    }

    #[test]
    fn parse_accepts_bare_no_offset_value() {
        // No offset and no `Z`: treated as UTC (other tools' output).
        let ts = parse_last_update("2026-07-19T02:48:34").expect("bare value should parse");
        assert_eq!(ts.unix_timestamp(), utc_instant().unix_timestamp());
        // Bare value with fractional seconds too.
        let ts = parse_last_update("2026-07-19T02:48:34.1234567").expect("bare frac should parse");
        assert_eq!(ts.unix_timestamp(), utc_instant().unix_timestamp());
        assert_eq!(ts.nanosecond(), 123_456_700);
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_last_update("not-a-time").is_none());
    }

    // ── format ⇄ parse ───────────────────────────────────────────────────────

    #[test]
    fn format_parse_roundtrip_preserves_tick_precision() {
        let now = instant_with_ns(123_456_700);
        let s = format_last_update(now);
        let back = parse_last_update(&s).expect("roundtrip parse should succeed");
        // 7 fractional digits = 100 ns ticks; compare at tick granularity
        assert_eq!(back.unix_timestamp(), now.unix_timestamp());
        assert_eq!(back.nanosecond() / 100, now.nanosecond() / 100);
    }
}
