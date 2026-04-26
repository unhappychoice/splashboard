//! `clock_countdown` — remaining time to `target` (single, Text) or `targets = [...]` (multi,
//! TextBlock / Entries). Past targets render as `"passed"` so the widget keeps rendering through
//! the event boundary.

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, EntriesData, Entry, Payload, TextBlockData, TextData};
use crate::render::Shape;
use crate::samples;

use super::common;

const SHAPES: &[Shape] = &[Shape::Text, Shape::TextBlock, Shape::Entries];

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
        "Time remaining until one configured `target` date or a list of labelled `targets`, formatted as `Nd Nh` / `Nh Nm` / `Nm`. Past targets keep rendering as `passed` so the widget survives the event boundary."
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
        let body = match (&opts.targets, &opts.target, shape) {
            (Some(list), _, Shape::Entries) => Body::Entries(EntriesData {
                items: list
                    .iter()
                    .map(|t| Entry {
                        key: t.label.clone(),
                        value: Some(format_remaining(&now, &t.target)),
                        status: None,
                    })
                    .collect(),
            }),
            (Some(list), _, _) => Body::TextBlock(TextBlockData {
                lines: list
                    .iter()
                    .map(|t| format!("{}: {}", t.label, format_remaining(&now, &t.target)))
                    .collect(),
            }),
            (None, Some(target), _) => Body::Text(TextData {
                value: format_single(&now, target, opts.target_label.as_deref()),
            }),
            (None, None, _) => Body::Text(TextData {
                value: "no target configured".into(),
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
}
