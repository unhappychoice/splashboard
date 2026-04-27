//! `clock_countdown` — remaining time to `target` (single, Text) or `targets = [...]` (multi,
//! TextBlock / Entries). Past targets render as `"passed"` so the widget keeps rendering through
//! the event boundary.

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, TimeZone, Utc};
use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    Body, CalendarData, EntriesData, Entry, Payload, RatioData, TextBlockData, TextData,
};
use crate::render::Shape;
use crate::samples;

use super::common;

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::Entries,
    Shape::Ratio,
    Shape::Calendar,
];

const DEFAULT_WINDOW_DAYS: u32 = 30;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "timezone",
        type_hint: "IANA timezone (e.g. \"Asia/Tokyo\")",
        required: false,
        default: Some("system local"),
        description: "Timezone used when interpreting `target` / `targets`. Omit to follow the system clock.",
    },
    OptionSchema {
        name: "target",
        type_hint: "RFC3339 datetime or YYYY-MM-DD",
        required: false,
        default: None,
        description: "Single countdown target. Mutually exclusive with `targets`. Date-only values are treated as UTC midnight.",
    },
    OptionSchema {
        name: "target_label",
        type_hint: "string",
        required: false,
        default: None,
        description: "Optional label prefixed to the single-target line (e.g. \"Ship:\").",
    },
    OptionSchema {
        name: "targets",
        type_hint: "array of `{ label, target }`",
        required: false,
        default: None,
        description: "Multiple labelled countdowns. Rendered as `TextBlock` by default or `Entries` when the renderer expects a key/value shape.",
    },
    OptionSchema {
        name: "window_days",
        type_hint: "integer (1..=3650)",
        required: false,
        default: Some("30"),
        description: "Approach window used by `Ratio`. Progress is `(now - (target - window))/window`, clamped to `0..=1`, so a 30-day window starts filling 30 days out from the target.",
    },
];

pub struct ClockCountdownFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub target_label: Option<String>,
    #[serde(default)]
    pub targets: Option<Vec<TargetEntry>>,
    #[serde(default)]
    pub window_days: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetEntry {
    pub label: String,
    pub target: String,
}

impl RealtimeFetcher for ClockCountdownFetcher {
    fn name(&self) -> &str {
        "clock_countdown"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Time remaining until one configured `target` date or a list of labelled `targets`, formatted as `Nd Nh` / `Nh Nm` / `Nm`. Past targets keep rendering as `passed` so the widget survives the event boundary. `Ratio` exposes a 30-day approach progress (override with `window_days`); `Calendar` highlights the target day within its month."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("Ship v2: 42d 3h"),
            Shape::TextBlock => samples::text_block(&["Ship v2: 42d 3h", "Release: 7d"]),
            Shape::Entries => samples::entries(&[("Ship v2", "42d 3h"), ("Release", "7d")]),
            Shape::Ratio => samples::ratio(0.77, "Ship v2 · 7d to go"),
            Shape::Calendar => Body::Calendar(CalendarData {
                year: 2026,
                month: 12,
                day: Some(31),
                events: Vec::new(),
            }),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        let now = common::now_in(opts.timezone.as_deref());
        let shape = ctx.shape.unwrap_or(Shape::Text);
        let window_days = opts
            .window_days
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(1, 3650);
        let primary_target = opts.target.as_deref().or_else(|| {
            opts.targets
                .as_ref()
                .and_then(|v| v.first())
                .map(|t| t.target.as_str())
        });
        let body = match shape {
            Shape::Ratio => ratio_body(
                &now,
                primary_target,
                opts.target_label.as_deref(),
                &opts.targets,
                window_days,
            ),
            Shape::Calendar => calendar_body(&now, primary_target, &opts.targets),
            Shape::Entries => match &opts.targets {
                Some(list) => Body::Entries(EntriesData {
                    items: list
                        .iter()
                        .map(|t| Entry {
                            key: t.label.clone(),
                            value: Some(format_remaining(&now, &t.target)),
                            status: None,
                        })
                        .collect(),
                }),
                None => single_text_body(&now, &opts),
            },
            Shape::TextBlock => match &opts.targets {
                Some(list) => Body::TextBlock(TextBlockData {
                    lines: list
                        .iter()
                        .map(|t| format!("{}: {}", t.label, format_remaining(&now, &t.target)))
                        .collect(),
                }),
                None => single_text_body(&now, &opts),
            },
            _ => single_text_body(&now, &opts),
        };
        Payload {
            icon: None,
            status: None,
            format: None,
            body,
        }
    }
}

fn single_text_body(now: &DateTime<FixedOffset>, opts: &Options) -> Body {
    Body::Text(TextData {
        value: match opts.target.as_deref() {
            Some(target) => format_single(now, target, opts.target_label.as_deref()),
            None => "no target configured".into(),
        },
    })
}

fn ratio_body(
    now: &DateTime<FixedOffset>,
    primary: Option<&str>,
    label: Option<&str>,
    targets: &Option<Vec<TargetEntry>>,
    window_days: u32,
) -> Body {
    let Some(target_str) = primary else {
        return Body::Ratio(RatioData {
            value: 0.0,
            label: Some("no target configured".into()),
            denominator: None,
        });
    };
    let Some(target) = parse_target(target_str) else {
        return Body::Ratio(RatioData {
            value: 0.0,
            label: Some("invalid target".into()),
            denominator: None,
        });
    };
    let value = approach_ratio(now, &target, window_days);
    let label = label
        .map(String::from)
        .or_else(|| {
            targets
                .as_ref()
                .and_then(|v| v.first())
                .map(|t| t.label.clone())
        })
        .map(|l| format!("{l} · {}", format_remaining(now, target_str)))
        .unwrap_or_else(|| format_remaining(now, target_str));
    Body::Ratio(RatioData {
        value,
        label: Some(label),
        denominator: None,
    })
}

fn approach_ratio(
    now: &DateTime<FixedOffset>,
    target: &DateTime<FixedOffset>,
    window_days: u32,
) -> f64 {
    let secs = target.signed_duration_since(*now).num_seconds();
    if secs <= 0 {
        return 1.0;
    }
    let window = i64::from(window_days) * 86_400;
    if window <= 0 {
        return 0.0;
    }
    (1.0 - secs as f64 / window as f64).clamp(0.0, 1.0)
}

fn calendar_body(
    now: &DateTime<FixedOffset>,
    primary: Option<&str>,
    targets: &Option<Vec<TargetEntry>>,
) -> Body {
    let primary_dt = primary.and_then(parse_target);
    let (year, month, day) = match primary_dt {
        Some(dt) => (dt.year(), dt.month() as u8, Some(dt.day() as u8)),
        None => (now.year(), now.month() as u8, Some(now.day() as u8)),
    };
    let events = targets
        .as_ref()
        .map(|list| {
            list.iter()
                .filter_map(|t| parse_target(&t.target))
                .filter(|dt| dt.year() == year && dt.month() == u32::from(month))
                .map(|dt| dt.day() as u8)
                .collect()
        })
        .unwrap_or_default();
    Body::Calendar(CalendarData {
        year,
        month,
        day,
        events,
    })
}

fn format_single(now: &DateTime<FixedOffset>, target: &str, label: Option<&str>) -> String {
    let remaining = format_remaining(now, target);
    match label {
        Some(l) => format!("{l}: {remaining}"),
        None => remaining,
    }
}

fn format_remaining(now: &DateTime<FixedOffset>, target: &str) -> String {
    let Some(target) = parse_target(target) else {
        return "invalid target".into();
    };
    let secs = target.signed_duration_since(*now).num_seconds();
    if secs <= 0 {
        return "passed".into();
    }
    humanize(secs)
}

/// Accepts RFC3339 (`2026-12-31T18:00:00+09:00`) or date-only (`2026-12-31`). Date-only parses
/// as UTC midnight so a configured date means the same wall instant across user timezones.
fn parse_target(raw: &str) -> Option<DateTime<FixedOffset>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt);
    }
    let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok()?;
    let midnight = date.and_hms_opt(0, 0, 0)?;
    Some(Utc.from_utc_datetime(&midnight).fixed_offset())
}

fn humanize(secs: i64) -> String {
    const MIN: i64 = 60;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;
    let days = secs / DAY;
    let hours = (secs % DAY) / HOUR;
    let minutes = (secs % HOUR) / MIN;
    match (days, hours, minutes) {
        (0, 0, m) => format!("{m}m"),
        (0, h, m) => format!("{h}h {m}m"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn ctx(shape: Option<Shape>, options: &str) -> FetchContext {
        FetchContext {
            widget_id: "c".into(),
            timeout: Duration::from_secs(1),
            shape,
            options: Some(toml::from_str(options).unwrap()),
            ..Default::default()
        }
    }

    #[test]
    fn single_target_emits_text() {
        let p = ClockCountdownFetcher.compute(&ctx(Some(Shape::Text), "target = \"2099-12-31\""));
        match p.body {
            Body::Text(d) => assert!(!d.value.is_empty()),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn multi_entries_yields_one_per_target() {
        let p = ClockCountdownFetcher.compute(&ctx(
            Some(Shape::Entries),
            r#"targets = [{label = "A", target = "2099-01-01"}, {label = "B", target = "2099-06-01"}]"#,
        ));
        match p.body {
            Body::Entries(d) => assert_eq!(d.items.len(), 2),
            _ => panic!("expected entries"),
        }
    }

    #[test]
    fn past_target_renders_passed() {
        let p = ClockCountdownFetcher.compute(&ctx(Some(Shape::Text), "target = \"2000-01-01\""));
        match p.body {
            Body::Text(d) => assert!(d.value.contains("passed")),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn humanize_buckets_by_granularity() {
        assert_eq!(humanize(45 * 60), "45m");
        assert_eq!(humanize(3 * 3600 + 5 * 60), "3h 5m");
        assert_eq!(humanize(2 * 86_400 + 3 * 3600), "2d 3h");
    }

    #[test]
    fn approach_ratio_starts_at_zero_and_completes_at_target() {
        let now = Utc
            .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
            .unwrap()
            .fixed_offset();
        let far = Utc
            .with_ymd_and_hms(2027, 1, 1, 0, 0, 0)
            .unwrap()
            .fixed_offset();
        // 1 year out, 30-day window — completely outside the window so 0.0.
        assert_eq!(approach_ratio(&now, &far, 30), 0.0);
        let mid = Utc
            .with_ymd_and_hms(2026, 1, 16, 0, 0, 0)
            .unwrap()
            .fixed_offset();
        // 15 days out, 30-day window — half full.
        assert!((approach_ratio(&now, &mid, 30) - 0.5).abs() < 1e-6);
        // Past target → fully filled.
        let past = Utc
            .with_ymd_and_hms(2025, 12, 1, 0, 0, 0)
            .unwrap()
            .fixed_offset();
        assert_eq!(approach_ratio(&now, &past, 30), 1.0);
    }

    #[test]
    fn ratio_shape_emits_ratio_body() {
        let p = ClockCountdownFetcher.compute(&ctx(Some(Shape::Ratio), "target = \"2099-12-31\""));
        let Body::Ratio(d) = p.body else {
            panic!("expected ratio")
        };
        assert!(d.value >= 0.0 && d.value <= 1.0);
        assert!(d.label.is_some());
    }

    #[test]
    fn ratio_with_no_target_label_says_so() {
        let p = ClockCountdownFetcher.compute(&ctx(Some(Shape::Ratio), ""));
        let Body::Ratio(d) = p.body else {
            panic!("expected ratio")
        };
        assert_eq!(d.value, 0.0);
        assert_eq!(d.label.as_deref(), Some("no target configured"));
    }

    #[test]
    fn calendar_shape_pins_target_day() {
        let p =
            ClockCountdownFetcher.compute(&ctx(Some(Shape::Calendar), "target = \"2099-12-31\""));
        let Body::Calendar(d) = p.body else {
            panic!("expected calendar")
        };
        assert_eq!(d.year, 2099);
        assert_eq!(d.month, 12);
        assert_eq!(d.day, Some(31));
    }

    #[test]
    fn calendar_with_multi_targets_marks_event_days_in_primary_month() {
        let p = ClockCountdownFetcher.compute(&ctx(
            Some(Shape::Calendar),
            r#"targets = [
                {label = "A", target = "2099-12-05"},
                {label = "B", target = "2099-12-20"},
                {label = "C", target = "2100-01-10"}
            ]"#,
        ));
        let Body::Calendar(d) = p.body else {
            panic!("expected calendar")
        };
        assert_eq!(d.year, 2099);
        assert_eq!(d.month, 12);
        assert_eq!(d.day, Some(5));
        assert!(d.events.contains(&20));
        assert!(!d.events.contains(&10));
    }
}
