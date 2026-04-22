//! `clock_countdown` — remaining time to `target` (single, Lines) or `targets = [...]` (multi,
//! Lines / Entries). Past targets render as `"passed"` so the widget keeps rendering through the
//! event boundary.

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::payload::{Body, EntriesData, Entry, LinesData, Payload};
use crate::render::Shape;

use super::common;

const SHAPES: &[Shape] = &[Shape::Lines, Shape::Entries];

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
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        let now = common::now_in(opts.timezone.as_deref());
        let shape = ctx.shape.unwrap_or(Shape::Lines);
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
            (Some(list), _, _) => Body::Lines(LinesData {
                lines: list
                    .iter()
                    .map(|t| format!("{}: {}", t.label, format_remaining(&now, &t.target)))
                    .collect(),
            }),
            (None, Some(target), _) => Body::Lines(LinesData {
                lines: vec![format_single(&now, target, opts.target_label.as_deref())],
            }),
            (None, None, _) => Body::Lines(LinesData {
                lines: vec!["no target configured".into()],
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
    fn single_target_emits_one_line() {
        let p = ClockCountdownFetcher.compute(&ctx(Some(Shape::Lines), "target = \"2099-12-31\""));
        match p.body {
            Body::Lines(d) => assert_eq!(d.lines.len(), 1),
            _ => panic!("expected lines"),
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
        let p = ClockCountdownFetcher.compute(&ctx(Some(Shape::Lines), "target = \"2000-01-01\""));
        match p.body {
            Body::Lines(d) => assert!(d.lines[0].contains("passed")),
            _ => panic!("expected lines"),
        }
    }

    #[test]
    fn humanize_buckets_by_granularity() {
        assert_eq!(humanize(45 * 60), "45m");
        assert_eq!(humanize(3 * 3600 + 5 * 60), "3h 5m");
        assert_eq!(humanize(2 * 86_400 + 3 * 3600), "2d 3h");
    }
}
