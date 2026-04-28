//! Shared helpers for the `clock_*` family. Every sibling fetcher lives in this module tree and
//! pulls from here for tz resolution, safe strftime, julian-day math, and the small placeholder
//! helper. Keeping these central avoids per-fetcher duplication while letting each sibling keep a
//! narrow public surface.
//!
//! tz / locale resolution is delegated to [`crate::time`] so the same defaults flow through every
//! fetcher in the project, not just clocks.

use chrono::{DateTime, FixedOffset, NaiveDate};

use crate::payload::{Body, Payload, TextBlockData};
use crate::time as t;

pub const DEFAULT_FORMAT: &str = "%H:%M";

pub fn now_in(timezone: Option<&str>) -> DateTime<FixedOffset> {
    t::now_in(timezone)
}

pub fn parse_tz(name: &str) -> Option<chrono_tz::Tz> {
    t::parse_tz(Some(name))
}

pub fn safe_format(dt: &DateTime<FixedOffset>, fmt: &str) -> String {
    safe_format_with_locale(dt, fmt, None)
}

pub fn safe_format_with_locale(
    dt: &DateTime<FixedOffset>,
    fmt: &str,
    locale: Option<&str>,
) -> String {
    let primary = t::format_local(dt, fmt, locale);
    if primary.is_empty() {
        t::format_local(dt, DEFAULT_FORMAT, locale)
    } else {
        primary
    }
}

pub fn julian_day(date: NaiveDate) -> i64 {
    let epoch = NaiveDate::from_ymd_opt(2000, 1, 1).expect("2000-01-01 always valid");
    2_451_545 + (date - epoch).num_days()
}

/// Two-line warning body — mirrors `fetcher::shape_mismatch_placeholder` so users see a
/// consistent "this widget is misconfigured" shape instead of a silent wrong render.
pub fn placeholder(msg: &str) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body: Body::TextBlock(TextBlockData {
            lines: vec![
                format!("⚠ {msg}"),
                "check [widget.options] in config".into(),
            ],
        }),
    }
}

pub fn parse_options<T: serde::de::DeserializeOwned + Default>(
    raw: Option<&toml::Value>,
) -> Result<T, String> {
    match raw {
        None => Ok(T::default()),
        Some(value) => value
            .clone()
            .try_into::<T>()
            .map_err(|e| format!("invalid options: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn safe_format_falls_back_on_invalid_directive() {
        let dt = Utc
            .with_ymd_and_hms(2026, 4, 22, 12, 34, 0)
            .unwrap()
            .fixed_offset();
        assert_eq!(safe_format(&dt, "%Q"), "12:34");
    }

    #[test]
    fn now_in_invalid_tz_does_not_panic() {
        let _ = now_in(Some("Not/AZone"));
    }

    #[test]
    fn julian_day_of_j2000_epoch() {
        let d = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
        assert_eq!(julian_day(d), 2_451_545);
    }
}
