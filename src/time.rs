//! Project-wide locale + timezone helpers. Every fetcher / renderer that emits a
//! human-readable timestamp goes through here so the same `[general]` defaults
//! (and per-widget overrides) drive every clock-shaped output instead of each
//! site reaching for `Local::now()` / `Utc::now()` independently.
//!
//! - `now_in(tz)` resolves an IANA name (`"Asia/Tokyo"`) to a fixed-offset
//!   `DateTime`. Falls back to `Local` for unknown names — never panics.
//! - `today_in(tz)` is the date-only counterpart for "today" math.
//! - `parse_locale(name)` resolves a `chrono::Locale` from `"en_US"` /
//!   `"ja_JP"`. Falls back to `POSIX` (chrono's hard-coded English).
//! - `format_local(dt, fmt, locale)` is `chrono::format_localized` with a
//!   directive-error fallback so a typo'd `%Q` doesn't crash the splash.
//! - `format_relative_compact(seconds, locale)` is the `3h` / `2d` /
//!   `1w` rounding used by `list_timeline` and friends. Locale only flips
//!   the suffix glyphs; the math is locale-independent.
//!
//! Behaviour with `tz = None` and `locale = None` matches the pre-migration
//! `Local::now()` + `format(...)` output bit-for-bit (POSIX is what chrono's
//! unlocalised `format` already emits), so call sites can migrate without
//! having to compare snapshots.

use chrono::{DateTime, FixedOffset, Local, Locale, NaiveDate, Utc};

pub const DEFAULT_LOCALE: Locale = Locale::POSIX;

pub fn now_in(timezone: Option<&str>) -> DateTime<FixedOffset> {
    match parse_tz(timezone) {
        Some(tz) => Utc::now().with_timezone(&tz).fixed_offset(),
        None => Local::now().fixed_offset(),
    }
}

pub fn today_in(timezone: Option<&str>) -> NaiveDate {
    now_in(timezone).date_naive()
}

pub fn parse_tz(name: Option<&str>) -> Option<chrono_tz::Tz> {
    name.and_then(|s| s.parse().ok())
}

pub fn parse_locale(name: Option<&str>) -> Locale {
    name.and_then(|s| Locale::try_from(s).ok())
        .unwrap_or(DEFAULT_LOCALE)
}

pub fn format_local<Tz>(dt: &DateTime<Tz>, fmt: &str, locale: Option<&str>) -> String
where
    Tz: chrono::TimeZone,
    Tz::Offset: std::fmt::Display,
{
    let loc = parse_locale(locale);
    format_with(dt, fmt, loc).unwrap_or_else(|| dt.format("%H:%M").to_string())
}

fn format_with<Tz>(dt: &DateTime<Tz>, fmt: &str, locale: Locale) -> Option<String>
where
    Tz: chrono::TimeZone,
    Tz::Offset: std::fmt::Display,
{
    use std::fmt::Write;
    let mut buf = String::new();
    write!(&mut buf, "{}", dt.format_localized(fmt, locale)).ok()?;
    Some(buf)
}

/// `3h` / `2d` / `1w` style label for `seconds_ago`. Future deltas grow an `in `
/// prefix; sub-minute deltas collapse to `now`. Returns `None` when the delta
/// exceeds ~4 weeks so callers can fall back to an absolute date.
pub fn format_relative_compact(seconds_ago: i64, _locale: Option<&str>) -> Option<String> {
    if seconds_ago.abs() < 45 {
        return Some("now".into());
    }
    let abs = seconds_ago.abs();
    let body = compact_body(abs)?;
    Some(if seconds_ago >= 0 {
        body
    } else {
        format!("in {body}")
    })
}

fn compact_body(abs: i64) -> Option<String> {
    const MIN: i64 = 60;
    const HOUR: i64 = 60 * 60;
    const DAY: i64 = 60 * 60 * 24;
    const WEEK: i64 = DAY * 7;
    if abs < HOUR {
        Some(format!("{}m", (abs + MIN / 2) / MIN))
    } else if abs < DAY {
        Some(format!("{}h", abs / HOUR))
    } else if abs < WEEK {
        Some(format!("{}d", abs / DAY))
    } else if abs < WEEK * 4 {
        Some(format!("{}w", abs / WEEK))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn now_in_invalid_tz_falls_back_to_local() {
        let _ = now_in(Some("Not/AZone"));
    }

    #[test]
    fn now_in_resolves_iana_name() {
        let dt = now_in(Some("Asia/Tokyo"));
        assert_eq!(dt.offset().local_minus_utc(), 9 * 3600);
    }

    #[test]
    fn parse_locale_defaults_to_posix() {
        assert_eq!(parse_locale(None), Locale::POSIX);
        assert_eq!(parse_locale(Some("nonsense_XX")), Locale::POSIX);
    }

    #[test]
    fn parse_locale_recognises_known_name() {
        assert_eq!(parse_locale(Some("ja_JP")), Locale::ja_JP);
    }

    #[test]
    fn format_local_invalid_directive_falls_back_to_safe_default() {
        let dt = Utc.with_ymd_and_hms(2026, 4, 22, 12, 34, 0).unwrap();
        assert_eq!(format_local(&dt, "%Q", None), "12:34");
    }

    #[test]
    fn format_local_default_locale_matches_posix_format() {
        let dt = Utc.with_ymd_and_hms(2026, 4, 22, 0, 0, 0).unwrap();
        assert_eq!(format_local(&dt, "%b %d", None), "Apr 22");
    }

    #[test]
    fn format_local_japanese_weekday_uses_locale_table() {
        let dt = Utc.with_ymd_and_hms(2026, 4, 22, 0, 0, 0).unwrap();
        assert_eq!(format_local(&dt, "%A", Some("ja_JP")), "水曜日");
    }

    #[test]
    fn format_relative_compact_under_45_seconds_is_now() {
        assert_eq!(format_relative_compact(30, None).as_deref(), Some("now"));
    }

    #[test]
    fn format_relative_compact_minutes_round_half_up() {
        assert_eq!(format_relative_compact(45, None).as_deref(), Some("1m"));
        assert_eq!(format_relative_compact(120, None).as_deref(), Some("2m"));
    }

    #[test]
    fn format_relative_compact_hours() {
        assert_eq!(
            format_relative_compact(3 * 3600, None).as_deref(),
            Some("3h")
        );
    }

    #[test]
    fn format_relative_compact_future_grows_in_prefix() {
        assert_eq!(
            format_relative_compact(-3600, None).as_deref(),
            Some("in 1h")
        );
    }

    #[test]
    fn format_relative_compact_returns_none_past_4_weeks() {
        assert!(format_relative_compact(60 * 86_400, None).is_none());
    }

    #[test]
    fn today_in_matches_now_in_date() {
        let now = now_in(Some("UTC"));
        assert_eq!(today_in(Some("UTC")), now.date_naive());
    }
}
