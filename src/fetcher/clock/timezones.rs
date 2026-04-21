//! `clock_timezones` — world-clock strip. Each configured IANA zone becomes one row.
//! Parse failures per zone surface as `"??"` so one bad entry doesn't swallow the widget.

use chrono::{DateTime, FixedOffset, Utc};
use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::payload::{Body, EntriesData, Entry, LinesData, Payload};
use crate::render::Shape;

use super::common;

const SHAPES: &[Shape] = &[Shape::Lines, Shape::Entries];

pub struct ClockTimezonesFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub timezones: Vec<String>,
}

impl RealtimeFetcher for ClockTimezonesFetcher {
    fn name(&self) -> &str {
        "clock_timezones"
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
        let fmt = ctx.format.as_deref().unwrap_or(common::DEFAULT_FORMAT);
        let shape = ctx.shape.unwrap_or(Shape::Lines);
        let rows: Vec<(String, String)> = opts
            .timezones
            .iter()
            .map(|name| (name.clone(), format_time(name, fmt)))
            .collect();
        let body = match shape {
            Shape::Entries => Body::Entries(EntriesData {
                items: rows
                    .into_iter()
                    .map(|(k, v)| Entry {
                        key: k,
                        value: Some(v),
                        status: None,
                    })
                    .collect(),
            }),
            _ => Body::Lines(LinesData {
                lines: rows.into_iter().map(|(n, t)| format!("{n} {t}")).collect(),
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

fn format_time(zone: &str, fmt: &str) -> String {
    resolve(zone)
        .map(|dt| common::safe_format(&dt, fmt))
        .unwrap_or_else(|| "??".into())
}

fn resolve(name: &str) -> Option<DateTime<FixedOffset>> {
    let tz = common::parse_tz(name)?;
    Some(Utc::now().with_timezone(&tz).fixed_offset())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn ctx(shape: Option<Shape>, options: &str) -> FetchContext {
        FetchContext {
            widget_id: "tz".into(),
            timeout: Duration::from_secs(1),
            shape,
            options: Some(toml::from_str(options).unwrap()),
            ..Default::default()
        }
    }

    #[test]
    fn lines_emits_one_row_per_zone() {
        let p = ClockTimezonesFetcher.compute(&ctx(
            Some(Shape::Lines),
            "timezones = [\"Asia/Tokyo\", \"UTC\"]",
        ));
        match p.body {
            Body::Lines(d) => assert_eq!(d.lines.len(), 2),
            _ => panic!("expected lines"),
        }
    }

    #[test]
    fn entries_emits_one_entry_per_zone() {
        let p = ClockTimezonesFetcher.compute(&ctx(
            Some(Shape::Entries),
            "timezones = [\"Asia/Tokyo\", \"Europe/London\"]",
        ));
        match p.body {
            Body::Entries(d) => assert_eq!(d.items.len(), 2),
            _ => panic!("expected entries"),
        }
    }

    #[test]
    fn invalid_zone_renders_placeholder_time() {
        let p =
            ClockTimezonesFetcher.compute(&ctx(Some(Shape::Lines), "timezones = [\"Not/AZone\"]"));
        match p.body {
            Body::Lines(d) => assert!(d.lines[0].ends_with("??")),
            _ => panic!("expected lines"),
        }
    }
}
