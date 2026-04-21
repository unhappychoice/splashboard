//! `clock_state` — temporal boolean states (business-hours / weekend / night) as `Shape::Badge`.
//! `Ok` when active, `Warn` when not.

use chrono::{DateTime, Datelike, FixedOffset, Timelike, Weekday};
use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::payload::{BadgeData, Body, Payload, Status};
use crate::render::Shape;

use super::common;

const SHAPES: &[Shape] = &[Shape::Badge];

pub struct ClockStateFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub kind: Option<Kind>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    BusinessHours,
    Weekend,
    Night,
}

impl RealtimeFetcher for ClockStateFetcher {
    fn name(&self) -> &str {
        "clock_state"
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
        let (active, label) = match opts.kind.unwrap_or(Kind::BusinessHours) {
            Kind::BusinessHours => business_hours(&now),
            Kind::Weekend => weekend(&now),
            Kind::Night => night(&now),
        };
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Badge(BadgeData {
                status: if active { Status::Ok } else { Status::Warn },
                label: label.into(),
            }),
        }
    }
}

fn business_hours(now: &DateTime<FixedOffset>) -> (bool, &'static str) {
    let weekday = !matches!(now.weekday(), Weekday::Sat | Weekday::Sun);
    let in_hours = (9..17).contains(&now.hour());
    let active = weekday && in_hours;
    (active, if active { "open" } else { "closed" })
}

fn weekend(now: &DateTime<FixedOffset>) -> (bool, &'static str) {
    let active = matches!(now.weekday(), Weekday::Sat | Weekday::Sun);
    (active, if active { "weekend" } else { "weekday" })
}

fn night(now: &DateTime<FixedOffset>) -> (bool, &'static str) {
    let active = !(6..22).contains(&now.hour());
    (active, if active { "night" } else { "day" })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{TimeZone, Utc};

    use super::*;

    fn at(y: i32, mo: u32, d: u32, h: u32) -> DateTime<FixedOffset> {
        Utc.with_ymd_and_hms(y, mo, d, h, 0, 0)
            .unwrap()
            .fixed_offset()
    }

    fn ctx(options: &str) -> FetchContext {
        FetchContext {
            widget_id: "s".into(),
            timeout: Duration::from_secs(1),
            shape: Some(Shape::Badge),
            options: Some(toml::from_str(options).unwrap()),
            ..Default::default()
        }
    }

    #[test]
    fn business_hours_active_on_weekday_afternoon() {
        // 2026-04-22 is a Wednesday.
        assert!(business_hours(&at(2026, 4, 22, 14)).0);
    }

    #[test]
    fn weekend_active_on_sunday() {
        assert!(weekend(&at(2026, 4, 26, 12)).0);
    }

    #[test]
    fn night_active_at_23h() {
        assert!(night(&at(2026, 4, 22, 23)).0);
    }

    #[test]
    fn emits_badge_body() {
        let p = ClockStateFetcher.compute(&ctx(""));
        assert!(matches!(p.body, Body::Badge(_)));
    }
}
