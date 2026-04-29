//! `clock_almanac` — daily astro / calendric rollup as `Entries`. Packs the values
//! that `clock_derived` exposes one-at-a-time (moon phase / season / zodiac / chinese
//! zodiac / iso week / day of year) into a single multi-row widget so they can share
//! a cell with `grid_calendar` in a 2-column layout.
//!
//! Every value comes from the same date-arithmetic functions `clock_derived` uses —
//! this fetcher is a thin shape adapter, not a new source of data.

use chrono::{DateTime, Datelike, FixedOffset};
use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::payload::{Body, CalendarData, EntriesData, Entry, Payload};
use crate::render::Shape;
use crate::samples;

use super::common;
use super::derived::{self, Hemisphere, Kind};

const SHAPES: &[Shape] = &[Shape::Entries, Shape::TextBlock, Shape::Calendar];

pub struct ClockAlmanacFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub timezone: Option<String>,
    /// Matches `clock_derived`'s option so the season row reads "Autumn" below the
    /// equator in April. Defaults to Northern Hemisphere for back-compat.
    #[serde(default)]
    pub hemisphere: Option<Hemisphere>,
}

impl RealtimeFetcher for ClockAlmanacFetcher {
    fn name(&self) -> &str {
        "clock_almanac"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "A multi-row rollup of date-derived facts (moon phase, season, zodiac, chinese zodiac, ISO week, day of year). Use this when you want every almanac value at once; pick `clock_derived` instead to surface a single value on its own line. `Calendar` highlights today within the current month so the same fetcher can drive a `grid_calendar` widget."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Entries => samples::entries(&[
                ("moon", "🌖 Waning Gibbous"),
                ("season", "Spring"),
                ("zodiac", "♉ Taurus"),
                ("chinese", "🐎 Horse"),
                ("iso week", "2026-W17"),
                ("day of year", "114 of 365"),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "moon:        🌖 Waning Gibbous",
                "season:      Spring",
                "zodiac:      ♉ Taurus",
                "chinese:     🐎 Horse",
                "iso week:    2026-W17",
                "day of year: 114 of 365",
            ]),
            Shape::Calendar => Body::Calendar(CalendarData {
                year: 2026,
                month: 4,
                day: Some(24),
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
        let rows = rollup(&now, opts.hemisphere.unwrap_or_default());
        let body = match ctx.shape.unwrap_or(Shape::Entries) {
            Shape::TextBlock => Body::TextBlock(crate::payload::TextBlockData {
                lines: rows.iter().map(|(k, v)| format!("{k}: {v}")).collect(),
            }),
            Shape::Calendar => Body::Calendar(CalendarData {
                year: now.year(),
                month: now.month() as u8,
                day: Some(now.day() as u8),
                events: Vec::new(),
            }),
            _ => Body::Entries(EntriesData {
                items: rows
                    .into_iter()
                    .map(|(k, v)| Entry {
                        key: k.into(),
                        value: Some(v),
                        status: None,
                    })
                    .collect(),
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

fn rollup(now: &DateTime<FixedOffset>, hemisphere: Hemisphere) -> Vec<(&'static str, String)> {
    vec![
        ("moon", derived::invoke(now, Kind::MoonPhase, hemisphere)),
        ("season", derived::invoke(now, Kind::Season, hemisphere)),
        ("zodiac", derived::invoke(now, Kind::Zodiac, hemisphere)),
        (
            "chinese",
            derived::invoke(now, Kind::ChineseZodiac, hemisphere),
        ),
        ("iso week", derived::invoke(now, Kind::IsoWeek, hemisphere)),
        (
            "day of year",
            format!("{} of {}", now.ordinal(), days_in_year(now)),
        ),
    ]
}

fn days_in_year(now: &DateTime<FixedOffset>) -> u32 {
    let year = now.year();
    if is_leap(year) { 366 } else { 365 }
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{TimeZone, Utc};

    use super::*;

    fn at(y: i32, m: u32, d: u32) -> DateTime<FixedOffset> {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0)
            .unwrap()
            .fixed_offset()
    }

    fn ctx(shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "a".into(),
            timeout: Duration::from_secs(1),
            shape,
            ..Default::default()
        }
    }

    /// Locks the fetcher's `now` to UTC so date-based assertions don't drift in CI hosts on
    /// JST/PST etc. where local-vs-UTC straddle midnight.
    fn ctx_utc(shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "a".into(),
            timeout: Duration::from_secs(1),
            shape,
            options: Some(toml::from_str("timezone = \"UTC\"").unwrap()),
            ..Default::default()
        }
    }

    #[test]
    fn default_shape_entries_has_six_rows() {
        let p = ClockAlmanacFetcher.compute(&ctx(None));
        let Body::Entries(d) = p.body else {
            panic!("expected entries");
        };
        assert_eq!(d.items.len(), 6);
        assert_eq!(d.items[0].key, "moon");
        assert_eq!(d.items[5].key, "day of year");
    }

    #[test]
    fn rollup_uses_hemisphere_for_season_row() {
        let rows = rollup(&at(2026, 4, 15), Hemisphere::South);
        let season = rows.iter().find(|(k, _)| *k == "season").unwrap();
        assert_eq!(season.1, "Autumn");
    }

    #[test]
    fn fetcher_contract_and_samples_cover_catalog_surface() {
        let fetcher = ClockAlmanacFetcher;
        assert_eq!(fetcher.name(), "clock_almanac");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("multi-row rollup"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(
            fetcher.sample_body(Shape::Entries),
            Some(samples::entries(&[
                ("moon", "🌖 Waning Gibbous"),
                ("season", "Spring"),
                ("zodiac", "♉ Taurus"),
                ("chinese", "🐎 Horse"),
                ("iso week", "2026-W17"),
                ("day of year", "114 of 365"),
            ]))
        );
        assert_eq!(
            fetcher.sample_body(Shape::TextBlock),
            Some(samples::text_block(&[
                "moon:        🌖 Waning Gibbous",
                "season:      Spring",
                "zodiac:      ♉ Taurus",
                "chinese:     🐎 Horse",
                "iso week:    2026-W17",
                "day of year: 114 of 365",
            ]))
        );
        assert_eq!(
            fetcher.sample_body(Shape::Calendar),
            Some(Body::Calendar(CalendarData {
                year: 2026,
                month: 4,
                day: Some(24),
                events: Vec::new(),
            }))
        );
        assert_eq!(fetcher.sample_body(Shape::Text), None);
    }

    #[test]
    fn calendar_shape_pins_today() {
        // Pin both sides to UTC — without this the test was flaky on hosts whose local date
        // differs from UTC at the moment of execution (e.g. JST evening near year-end).
        let p = ClockAlmanacFetcher.compute(&ctx_utc(Some(Shape::Calendar)));
        let Body::Calendar(d) = p.body else {
            panic!("expected calendar");
        };
        let now = Utc::now();
        assert_eq!(d.year, now.year());
        assert_eq!(d.month, now.month() as u8);
        assert_eq!(d.day, Some(now.day() as u8));
    }

    #[test]
    fn text_block_shape_emits_one_line_per_fact() {
        let payload = ClockAlmanacFetcher.compute(&ctx_utc(Some(Shape::TextBlock)));
        assert!(matches!(
            &payload.body,
            Body::TextBlock(data)
                if data.lines.len() == 6
                    && data.lines[0].starts_with("moon: ")
                    && data.lines[5].starts_with("day of year: ")
        ));
    }

    #[test]
    fn invalid_options_return_placeholder() {
        let payload = ClockAlmanacFetcher.compute(&FetchContext {
            options: Some(toml::from_str("unexpected = 1").unwrap()),
            ..ctx(None)
        });
        assert!(matches!(
            &payload.body,
            Body::TextBlock(data)
                if data.lines[0].contains("invalid options")
                    && data.lines[1] == "check [widget.options] in config"
        ));
    }

    #[test]
    fn is_leap_2024_2025_2100_2000() {
        assert!(is_leap(2024));
        assert!(!is_leap(2025));
        assert!(!is_leap(2100));
        assert!(is_leap(2000));
    }
}
