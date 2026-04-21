use std::fmt::Write;

use chrono::{DateTime, Datelike, Local, Timelike};

use crate::payload::{Body, CalendarData, EntriesData, Entry, LinesData, Payload};
use crate::render::Shape;

use super::{FetchContext, RealtimeFetcher, Safety};

const DEFAULT_FORMAT: &str = "%H:%M";
const SHAPES: &[Shape] = &[Shape::Lines, Shape::Entries, Shape::Calendar];

/// Renders the current local time. `format` follows chrono's strftime conventions; default is
/// `%H:%M` (24h clock). Classified as [`RealtimeFetcher`] so the value is recomputed on every
/// frame instead of being pulled from a stale cache entry.
///
/// Multi-shape: one `Local::now()` read can be projected as a formatted `Lines` string (default),
/// an `Entries` breakdown (year / month / day / hour / minute / second rows) for a key-value
/// clock panel, or a `Calendar` with today highlighted for an ambient month view.
pub struct ClockFetcher;

impl RealtimeFetcher for ClockFetcher {
    fn name(&self) -> &str {
        "clock"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let now = Local::now();
        let shape = ctx.shape.unwrap_or(Shape::Lines);
        let body = match shape {
            Shape::Entries => Body::Entries(EntriesData {
                items: clock_entries(&now),
            }),
            Shape::Calendar => Body::Calendar(CalendarData {
                year: now.year(),
                month: now.month() as u8,
                day: Some(now.day() as u8),
                events: Vec::new(),
            }),
            // Lines is the default; anything the validator let through but we don't explicitly
            // handle also lands here so realtime stays infallible.
            _ => Body::Lines(LinesData {
                lines: vec![format_now(
                    &now,
                    ctx.format.as_deref().unwrap_or(DEFAULT_FORMAT),
                )],
            }),
        };
        Payload {
            icon: None,
            status: None,
            format: None,
            body,
        }
    }
}

/// Formats the current local time using `fmt`. An invalid strftime directive would cause
/// chrono's `DelayedFormat::to_string()` to panic (`Display::fmt` returns `Err`, and
/// `ToString::to_string` panics on that). We capture the error via `write!` and fall back to
/// the default format so a typo in a user's config can never crash the splash.
fn format_now(now: &DateTime<Local>, fmt: &str) -> String {
    let mut buf = String::new();
    if write!(&mut buf, "{}", now.format(fmt)).is_ok() {
        return buf;
    }
    let mut fallback = String::new();
    let _ = write!(&mut fallback, "{}", now.format(DEFAULT_FORMAT));
    fallback
}

/// Zero-padded date/time fields for the `Entries` shape. Named keys (year / month / …) rather
/// than strftime tokens so a config author reading the table doesn't need chrono knowledge.
fn clock_entries(now: &DateTime<Local>) -> Vec<Entry> {
    let fields = [
        ("year", format!("{:04}", now.year())),
        ("month", format!("{:02}", now.month())),
        ("day", format!("{:02}", now.day())),
        ("hour", format!("{:02}", now.hour())),
        ("minute", format!("{:02}", now.minute())),
        ("second", format!("{:02}", now.second())),
    ];
    fields
        .into_iter()
        .map(|(k, v)| Entry {
            key: k.into(),
            value: Some(v),
            status: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn ctx(format: Option<&str>) -> FetchContext {
        FetchContext {
            widget_id: "clock".into(),
            format: format.map(String::from),
            timeout: Duration::from_secs(1),
            ..Default::default()
        }
    }

    fn ctx_with_shape(shape: Shape) -> FetchContext {
        FetchContext {
            widget_id: "clock".into(),
            timeout: Duration::from_secs(1),
            shape: Some(shape),
            ..Default::default()
        }
    }

    fn first_line(p: Payload) -> String {
        match p.body {
            Body::Lines(d) => d.lines.into_iter().next().unwrap_or_default(),
            _ => panic!("expected lines body"),
        }
    }

    #[test]
    fn default_format_is_hh_mm() {
        let text = first_line(ClockFetcher.compute(&ctx(None)));
        assert_eq!(text.len(), 5, "{text:?} should be HH:MM");
        assert_eq!(text.chars().nth(2), Some(':'));
    }

    #[test]
    fn custom_format_is_honored() {
        let text = first_line(ClockFetcher.compute(&ctx(Some("%Y"))));
        assert_eq!(text.len(), 4, "{text:?} should be YYYY");
    }

    #[test]
    fn invalid_format_falls_back_without_panicking() {
        let text = first_line(ClockFetcher.compute(&ctx(Some("%Q"))));
        assert_eq!(text.len(), 5, "expected fallback HH:MM, got {text:?}");
        assert_eq!(text.chars().nth(2), Some(':'));
    }

    #[test]
    fn literal_percent_in_format_does_not_crash() {
        let text = first_line(ClockFetcher.compute(&ctx(Some("now: %H:%M"))));
        assert!(text.starts_with("now: "));
    }

    #[test]
    fn declares_lines_entries_and_calendar_shapes() {
        assert_eq!(
            RealtimeFetcher::shapes(&ClockFetcher),
            &[Shape::Lines, Shape::Entries, Shape::Calendar]
        );
    }

    #[test]
    fn entries_shape_emits_six_named_rows() {
        let p = ClockFetcher.compute(&ctx_with_shape(Shape::Entries));
        let items = match p.body {
            Body::Entries(d) => d.items,
            _ => panic!("expected entries body"),
        };
        let keys: Vec<String> = items.iter().map(|e| e.key.clone()).collect();
        assert_eq!(keys, ["year", "month", "day", "hour", "minute", "second"]);
        for e in &items {
            let v = e.value.as_deref().unwrap();
            assert!(v.chars().all(|c| c.is_ascii_digit()), "{v:?} not digits");
        }
        assert_eq!(items[0].value.as_deref().unwrap().len(), 4);
        for e in &items[1..] {
            assert_eq!(e.value.as_deref().unwrap().len(), 2);
        }
    }

    #[test]
    fn calendar_shape_highlights_today() {
        let p = ClockFetcher.compute(&ctx_with_shape(Shape::Calendar));
        let cal = match p.body {
            Body::Calendar(d) => d,
            _ => panic!("expected calendar body"),
        };
        let today = Local::now();
        assert_eq!(cal.year, today.year());
        assert_eq!(cal.month, today.month() as u8);
        assert_eq!(cal.day, Some(today.day() as u8));
    }
}
