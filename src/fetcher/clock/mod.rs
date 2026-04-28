//! `clock_*` fetcher family. The base `clock` displays the current time (with optional
//! timezone and Calendar events); each sibling module handles a distinct temporal feature —
//! world-clock strip, countdowns, period ratios, business/weekend/night state, sunrise/sunset,
//! and date-derived values. Shared primitives (tz resolution, safe strftime, julian day) live in
//! `common`.

pub mod almanac;
mod common;
pub mod countdown;
pub mod derived;
pub mod ratio;
pub mod state;
pub mod sunrise;
pub mod timezones;

use std::sync::Arc;

use chrono::{DateTime, Datelike, FixedOffset, Timelike};
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, CalendarData, EntriesData, Entry, Payload, TextData};
use crate::render::Shape;
use crate::samples;

use super::{FetchContext, RealtimeFetcher, Safety};

const CLOCK_OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "timezone",
        type_hint: "IANA timezone (e.g. \"Asia/Tokyo\")",
        required: false,
        default: Some("system local"),
        description: "Timezone used for the displayed time. Omit to follow the system clock.",
    },
    OptionSchema {
        name: "events",
        type_hint: "array of integers (1..=31)",
        required: false,
        default: None,
        description: "Days of the current month highlighted in the `Calendar` shape. Ignored by other shapes.",
    },
];

pub fn realtime_fetchers() -> Vec<Arc<dyn RealtimeFetcher>> {
    vec![
        Arc::new(ClockFetcher),
        Arc::new(timezones::ClockTimezonesFetcher),
        Arc::new(countdown::ClockCountdownFetcher),
        Arc::new(ratio::ClockRatioFetcher),
        Arc::new(state::ClockStateFetcher),
        Arc::new(sunrise::ClockSunriseFetcher),
        Arc::new(derived::ClockDerivedFetcher),
        Arc::new(almanac::ClockAlmanacFetcher),
    ]
}

const BASE_SHAPES: &[Shape] = &[Shape::Text, Shape::Entries, Shape::Calendar];

/// Base clock — renders "now" as a formatted string, key/value breakdown, or month calendar.
/// Options: `timezone` (IANA name) and `events` (days-of-month highlighted in Calendar shape).
pub struct ClockFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockOptions {
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub events: Option<Vec<u8>>,
}

impl RealtimeFetcher for ClockFetcher {
    fn name(&self) -> &str {
        "clock"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Current local time. Renders as a formatted clock string, a key/value breakdown of year/month/day/hour/minute/second, or a month calendar grid with optional highlighted days."
    }
    fn shapes(&self) -> &[Shape] {
        BASE_SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        CLOCK_OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("14:35"),
            Shape::Entries => samples::entries(&[
                ("year", "2026"),
                ("month", "04"),
                ("day", "22"),
                ("hour", "14"),
                ("minute", "35"),
                ("second", "12"),
            ]),
            Shape::Calendar => samples::calendar(2026, 4, Some(22), &[10, 15, 30]),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: ClockOptions = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        let tz = opts.timezone.as_deref().or(ctx.timezone.as_deref());
        let now = common::now_in(tz);
        let shape = ctx.shape.unwrap_or(Shape::Text);
        let body = match shape {
            Shape::Entries => Body::Entries(EntriesData {
                items: entries(&now),
            }),
            Shape::Calendar => Body::Calendar(CalendarData {
                year: now.year(),
                month: now.month() as u8,
                day: Some(now.day() as u8),
                events: opts.events.unwrap_or_default(),
            }),
            _ => Body::Text(TextData {
                value: common::safe_format_with_locale(
                    &now,
                    ctx.format.as_deref().unwrap_or(common::DEFAULT_FORMAT),
                    ctx.locale.as_deref(),
                ),
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

fn entries(now: &DateTime<FixedOffset>) -> Vec<Entry> {
    [
        ("year", format!("{:04}", now.year())),
        ("month", format!("{:02}", now.month())),
        ("day", format!("{:02}", now.day())),
        ("hour", format!("{:02}", now.hour())),
        ("minute", format!("{:02}", now.minute())),
        ("second", format!("{:02}", now.second())),
    ]
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

    fn ctx(shape: Option<Shape>, options: Option<&str>) -> FetchContext {
        let options = options.map(|s| toml::from_str::<toml::Value>(s).unwrap());
        FetchContext {
            widget_id: "clock".into(),
            timeout: Duration::from_secs(1),
            shape,
            options,
            ..Default::default()
        }
    }

    fn ctx_with_format(format: &str) -> FetchContext {
        FetchContext {
            widget_id: "clock".into(),
            format: Some(format.into()),
            timeout: Duration::from_secs(1),
            shape: Some(Shape::Text),
            ..Default::default()
        }
    }

    #[test]
    fn default_shape_is_text_with_colon() {
        let p = ClockFetcher.compute(&ctx(None, None));
        match p.body {
            Body::Text(d) => assert!(d.value.contains(':')),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn custom_format_renders_date_template() {
        let p = ClockFetcher.compute(&ctx_with_format("%Y · %A"));
        match p.body {
            Body::Text(d) => {
                assert!(
                    d.value.contains(" · "),
                    "expected middot separator from template, got {:?}",
                    d.value
                );
                assert!(
                    d.value.chars().next().is_some_and(|c| c.is_ascii_digit()),
                    "expected year digits at start, got {:?}",
                    d.value
                );
            }
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn entries_shape_emits_six_rows() {
        let p = ClockFetcher.compute(&ctx(Some(Shape::Entries), None));
        match p.body {
            Body::Entries(d) => assert_eq!(d.items.len(), 6),
            _ => panic!("expected entries"),
        }
    }

    #[test]
    fn calendar_shape_accepts_events_option() {
        let p = ClockFetcher.compute(&ctx(Some(Shape::Calendar), Some("events = [3, 15]")));
        match p.body {
            Body::Calendar(d) => assert_eq!(d.events, vec![3, 15]),
            _ => panic!("expected calendar"),
        }
    }

    #[test]
    fn unknown_option_is_rejected_to_placeholder() {
        let p = ClockFetcher.compute(&ctx(Some(Shape::Text), Some("bogus = 1")));
        match p.body {
            Body::TextBlock(d) => assert!(d.lines[0].starts_with("⚠")),
            _ => panic!("expected placeholder block"),
        }
    }

    #[test]
    fn realtime_fetchers_registers_full_family() {
        let fetchers = realtime_fetchers();
        let names: Vec<&str> = fetchers.iter().map(|f| f.name()).collect();
        for expected in [
            "clock",
            "clock_timezones",
            "clock_countdown",
            "clock_ratio",
            "clock_state",
            "clock_sunrise",
            "clock_derived",
        ] {
            assert!(names.contains(&expected), "missing: {expected}");
        }
    }
}
