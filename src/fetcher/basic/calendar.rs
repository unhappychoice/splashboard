//! `basic_calendar` — month view authored inline. `year` / `month` / `day` default to today
//! in the resolved timezone, so a fresh widget shows the current month without configuration;
//! `events` highlights extra days.

use chrono::Datelike;
use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, CalendarData, Payload};
use crate::render::Shape;
use crate::time as t;

use super::common;

const SHAPES: &[Shape] = &[Shape::Calendar];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "year",
        type_hint: "i32",
        required: false,
        default: Some("current year"),
        description: "ISO year. Defaults to today's year in the resolved timezone.",
    },
    OptionSchema {
        name: "month",
        type_hint: "u8 (1..=12)",
        required: false,
        default: Some("current month"),
        description: "Month number. Defaults to today's month.",
    },
    OptionSchema {
        name: "day",
        type_hint: "u8 (1..=31)",
        required: false,
        default: Some("current day"),
        description: "Focused / highlighted day. Defaults to today.",
    },
    OptionSchema {
        name: "events",
        type_hint: "list of u8 (1..=31)",
        required: false,
        default: Some("[]"),
        description: "Additional days to highlight in the month grid (1..=31).",
    },
    OptionSchema {
        name: "timezone",
        type_hint: "IANA timezone",
        required: false,
        default: Some("[general].timezone"),
        description: "Timezone for the today fallback. Omit to use the project-wide setting.",
    },
];

pub struct BasicCalendar;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(default)]
    pub month: Option<u8>,
    #[serde(default)]
    pub day: Option<u8>,
    #[serde(default)]
    pub events: Vec<u8>,
    #[serde(default)]
    pub timezone: Option<String>,
}

impl RealtimeFetcher for BasicCalendar {
    fn name(&self) -> &str {
        "basic_calendar"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Renders a `Calendar` payload from inline options. `year` / `month` / `day` default to today in the resolved timezone so a fresh config shows the current month with today highlighted; `events` lists extra days to mark."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        (shape == Shape::Calendar).then(|| {
            Body::Calendar(CalendarData {
                year: 2026,
                month: 5,
                day: Some(14),
                events: vec![3, 14, 27],
            })
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        let tz = opts.timezone.as_deref().or(ctx.timezone.as_deref());
        let now = t::now_in(tz);
        let year = opts.year.unwrap_or(now.year());
        let month = opts.month.unwrap_or(now.month() as u8);
        let day = opts.day.or(Some(now.day() as u8));
        if !(1..=12).contains(&month) {
            return common::placeholder("basic_calendar: `month` must be 1..=12");
        }
        if let Some(d) = day
            && !(1..=31).contains(&d)
        {
            return common::placeholder("basic_calendar: `day` must be 1..=31");
        }
        common::bare(Body::Calendar(CalendarData {
            year,
            month,
            day,
            events: opts.events,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compute(toml_src: &str) -> Payload {
        BasicCalendar.compute(&FetchContext {
            options: Some(toml::from_str(toml_src).unwrap()),
            ..Default::default()
        })
    }

    #[test]
    fn contract() {
        assert_eq!(BasicCalendar.name(), "basic_calendar");
        assert_eq!(BasicCalendar.shapes(), &[Shape::Calendar]);
    }

    #[test]
    fn explicit_year_month_day_overrides_today() {
        let p = compute(
            r#"
            year = 2024
            month = 6
            day = 15
            events = [1, 7, 14]
            "#,
        );
        assert_eq!(
            p.body,
            Body::Calendar(CalendarData {
                year: 2024,
                month: 6,
                day: Some(15),
                events: vec![1, 7, 14],
            })
        );
    }

    #[test]
    fn no_options_defaults_to_today() {
        let p = BasicCalendar.compute(&FetchContext::default());
        let Body::Calendar(d) = p.body else {
            panic!("expected Calendar");
        };
        // Year / month / day come from the system clock — assert they're populated and in range
        // rather than testing exact values (which would make the test flaky on month boundaries).
        assert!(d.year >= 2020);
        assert!((1..=12).contains(&d.month));
        assert!(matches!(d.day, Some(d) if (1..=31).contains(&d)));
    }

    #[test]
    fn invalid_month_renders_placeholder() {
        let p = compute(r#"month = 13"#);
        let Body::TextBlock(d) = p.body else {
            panic!("expected placeholder");
        };
        assert!(d.lines[0].contains("`month` must be 1..=12"));
    }

    #[test]
    fn invalid_day_renders_placeholder() {
        let p = compute(r#"day = 32"#);
        let Body::TextBlock(d) = p.body else {
            panic!("expected placeholder");
        };
        assert!(d.lines[0].contains("`day` must be 1..=31"));
    }

    #[test]
    fn metadata_methods_have_content() {
        let f = BasicCalendar;
        assert_eq!(f.safety(), Safety::Safe);
        assert!(!f.description().is_empty());
        let names: Vec<_> = f.option_schemas().iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["year", "month", "day", "events", "timezone"]);
    }

    #[test]
    fn sample_body_matches_declared_shape_only() {
        let f = BasicCalendar;
        assert!(matches!(
            f.sample_body(Shape::Calendar),
            Some(Body::Calendar(_))
        ));
        assert!(f.sample_body(Shape::Text).is_none());
    }

    #[test]
    fn invalid_options_render_placeholder() {
        let p = compute(r#"month = "june""#);
        let Body::TextBlock(d) = p.body else {
            panic!("expected placeholder");
        };
        assert!(d.lines[0].contains("invalid options"));
    }
}
