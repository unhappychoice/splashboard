//! Sample-output helpers and shape-level fallbacks.
//!
//! Every fetcher declares what its output looks like via [`crate::fetcher::Fetcher::sample_body`]
//! (or the realtime equivalent). Those declarations stay next to the fetcher code so they can't
//! drift out of sync. This module supplies:
//!
//! - small constructors (`text`, `text_block`, `entries`, `ratio`, …) so the declarations stay one-liners;
//! - [`canonical_sample`], the shape-level fallback used when a fetcher doesn't override.
//!
//! The samples are consumed at docs-generation time by `xtask`, and are reachable at runtime by
//! any future CLI that wants to preview what a fetcher emits without hitting I/O.

use crate::payload::{
    BadgeData, Bar, BarsData, Body, CalendarData, EntriesData, Entry, HeatmapData, LinkedLine,
    LinkedTextBlockData, MarkdownTextBlockData, NumberSeriesData, PointSeries, PointSeriesData,
    RatioData, Status, TextBlockData, TextData, TimelineData, TimelineEvent,
};
use crate::render::Shape;

/// Shape-level fallback used when a fetcher doesn't override `sample_body`. Returns `None` for
/// `Image` — the image renderer needs a real file path on disk, which sample data can't supply
/// portably.
pub fn canonical_sample(shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::Text => text("splashboard"),
        Shape::TextBlock => text_block(&["splashboard", "greetings on cd"]),
        Shape::MarkdownTextBlock => markdown(
            "# splashboard\n\nMarkdown body with **bold**, *italic*, and `code`.\n\n- item one\n- item two",
        ),
        Shape::LinkedTextBlock => linked_text_block(&[
            ("splashboard greets on cd", Some("https://example.com/")),
            ("recent commit landed", None),
        ]),
        Shape::Entries => entries(&[("key", "value"), ("foo", "bar"), ("baz", "qux")]),
        Shape::Ratio => ratio(0.67, "used"),
        Shape::NumberSeries => {
            number_series(&[2, 5, 3, 8, 4, 9, 6, 11, 7, 3, 4, 6, 2, 5, 8, 10, 7, 4, 9, 6])
        }
        Shape::PointSeries => sine_points(),
        Shape::Bars => bars(&[("rust", 42), ("go", 28), ("ts", 17), ("py", 13)]),
        Shape::Image => return None,
        Shape::Calendar => calendar(2026, 4, Some(22), &[10, 15, 30]),
        Shape::Heatmap => heatmap_grid(3, 7),
        Shape::Badge => badge(Status::Ok, "passing"),
        Shape::Timeline => timeline(&[
            (1_700_000_000, "merged #42", Some("feat(render): heatmap")),
            (1_699_990_000, "opened #41", None),
            (1_699_900_000, "reverted #40", None),
        ]),
    })
}

pub fn text(s: &str) -> Body {
    Body::Text(TextData {
        value: s.to_string(),
    })
}

pub fn text_block(ss: &[&str]) -> Body {
    Body::TextBlock(TextBlockData {
        lines: ss.iter().map(|s| (*s).to_string()).collect(),
    })
}

pub fn markdown(s: &str) -> Body {
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: s.to_string(),
    })
}

pub fn linked_text_block(rows: &[(&str, Option<&str>)]) -> Body {
    Body::LinkedTextBlock(LinkedTextBlockData {
        items: rows
            .iter()
            .map(|(text, url)| LinkedLine {
                text: (*text).to_string(),
                url: url.map(String::from),
            })
            .collect(),
    })
}

pub fn entries(kvs: &[(&str, &str)]) -> Body {
    Body::Entries(EntriesData {
        items: kvs
            .iter()
            .map(|(k, v)| Entry {
                key: (*k).into(),
                value: Some((*v).into()),
                status: None,
            })
            .collect(),
    })
}

pub fn ratio(value: f64, label: &str) -> Body {
    Body::Ratio(RatioData {
        value,
        label: Some(label.into()),
        denominator: None,
    })
}

pub fn number_series(vs: &[u64]) -> Body {
    Body::NumberSeries(NumberSeriesData {
        values: vs.to_vec(),
    })
}

pub fn bars(bs: &[(&str, u64)]) -> Body {
    Body::Bars(BarsData {
        bars: bs
            .iter()
            .map(|(l, v)| Bar {
                label: (*l).into(),
                value: *v,
            })
            .collect(),
    })
}

pub fn badge(status: Status, label: &str) -> Body {
    Body::Badge(BadgeData {
        status,
        label: label.into(),
    })
}

pub fn calendar(year: i32, month: u8, day: Option<u8>, events: &[u8]) -> Body {
    Body::Calendar(CalendarData {
        year,
        month,
        day,
        events: events.to_vec(),
    })
}

/// Plausible-looking filler grid for heatmaps. The exact values don't matter — only that the
/// grid has contrast so renderer buckets are visible.
pub fn heatmap_grid(rows: u32, cols: u32) -> Body {
    let cells: Vec<Vec<u32>> = (0..rows)
        .map(|r| (0..cols).map(|c| (r * 3 + c * 2 + (r ^ c)) % 10).collect())
        .collect();
    Body::Heatmap(HeatmapData {
        cells,
        thresholds: None,
        row_labels: None,
        col_labels: None,
    })
}

pub fn sine_points() -> Body {
    Body::PointSeries(PointSeriesData {
        series: vec![PointSeries {
            name: "series".into(),
            points: (0..20)
                .map(|i| {
                    let x = i as f64;
                    (x, (x / 3.0).sin() * 5.0 + 10.0)
                })
                .collect(),
        }],
    })
}

pub fn timeline(events: &[(i64, &str, Option<&str>)]) -> Body {
    Body::Timeline(TimelineData {
        events: events
            .iter()
            .map(|(ts, title, detail)| TimelineEvent {
                timestamp: *ts,
                title: (*title).into(),
                detail: detail.map(String::from),
                status: None,
            })
            .collect(),
    })
}
