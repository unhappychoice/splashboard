//! `deariary_recent` — most recent auto-generated diary entries.
//!
//! Mirrors deariary.com's reverse-chronological list. The same data feeds Timeline (default),
//! a list-of-titles view (TextBlock / Entries / LinkedTextBlock / MarkdownTextBlock), a
//! coverage view (Calendar — which days this month received an entry), and Text / Badge
//! headlines.
//!
//! Safety::Safe: host `api.deariary.com` is hardcoded.

use async_trait::async_trait;
use chrono::{Datelike, Local, NaiveDate};
use serde::Deserialize;

use super::client::{
    ApiEntry, MAX_LIST_LIMIT, cache_extra, cached_get_entries, entry_url, resolve_token,
};
use crate::fetcher::github::common::{cache_key, parse_options, parse_timestamp, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Body, CalendarData, EntriesData, Entry, LinkedLine, LinkedTextBlockData,
    MarkdownTextBlockData, Payload, Status, TextBlockData, TextData, TimelineData, TimelineEvent,
};
use crate::render::Shape;
use crate::samples;

const DEFAULT_LIMIT: u32 = 10;
const MIN_LIMIT: u32 = 1;
const SHAPES: &[Shape] = &[
    Shape::Timeline,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::LinkedTextBlock,
    Shape::Entries,
    Shape::Badge,
    Shape::Calendar,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "token",
        type_hint: "string",
        required: false,
        default: None,
        description: "Deariary API token. Falls back to the `DEARIARY_TOKEN` env var.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=100)",
        required: false,
        default: Some("10"),
        description: "Maximum number of entries to render in row-shaped views.",
    },
    OptionSchema {
        name: "tag",
        type_hint: "string",
        required: false,
        default: None,
        description: "Restrict results to a single tag slug.",
    },
];

pub struct DeariaryRecent;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    tag: Option<String>,
}

#[async_trait]
impl Fetcher for DeariaryRecent {
    fn name(&self) -> &str {
        "deariary_recent"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Recent auto-generated deariary.com entries, newest first. `Timeline` (default) carries the chronological ranking; `TextBlock` / `Entries` / `MarkdownTextBlock` / `LinkedTextBlock` carry the same ranking in their respective row formats; `Calendar` highlights this month's entry days; `Text` and `Badge` summarise the headline."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let opts: Options = parse_options(ctx.options.as_ref()).unwrap_or_default();
        let extra = cache_extra(opts.token.as_deref(), ctx.options.as_ref());
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Timeline => samples::timeline(&[
                (1_777_478_400, "My adventurous day", Some("482w")),
                (1_777_392_000, "A quiet Sunday", Some("310w")),
                (1_777_305_600, "Shipped the heatmap renderer", Some("612w")),
            ]),
            Shape::Text => samples::text("📔 3 entries · latest: My adventurous day"),
            Shape::TextBlock => samples::text_block(&[
                "04/27 · My adventurous day",
                "04/26 · A quiet Sunday",
                "04/25 · Shipped the heatmap renderer",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "- **04/27** — My adventurous day\n- **04/26** — A quiet Sunday\n- **04/25** — Shipped the heatmap renderer",
            ),
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "04/27 · My adventurous day",
                    Some("https://app.deariary.com/entries/2026/04/27"),
                ),
                (
                    "04/26 · A quiet Sunday",
                    Some("https://app.deariary.com/entries/2026/04/26"),
                ),
                (
                    "04/25 · Shipped the heatmap renderer",
                    Some("https://app.deariary.com/entries/2026/04/25"),
                ),
            ]),
            Shape::Entries => samples::entries(&[
                ("2026-04-27", "My adventurous day"),
                ("2026-04-26", "A quiet Sunday"),
                ("2026-04-25", "Shipped the heatmap renderer"),
            ]),
            Shape::Badge => Body::Badge(BadgeData {
                status: Status::Ok,
                label: "📔 3 recent".into(),
            }),
            Shape::Calendar => samples::calendar(2026, 4, Some(27), &[25, 26, 27]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let token = resolve_token(opts.token.as_deref())?;
        let limit = opts
            .limit
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(MIN_LIMIT, MAX_LIST_LIMIT);
        let entries = cached_get_entries(&token, opts.tag.as_deref()).await?;
        let shape = ctx.shape.unwrap_or(Shape::Timeline);
        let today = Local::now().date_naive();
        Ok(payload(render_body(&entries, shape, limit as usize, today)))
    }
}

fn render_body(entries: &[ApiEntry], shape: Shape, limit: usize, today: NaiveDate) -> Body {
    let limited: Vec<&ApiEntry> = entries.iter().take(limit).collect();
    match shape {
        // Headline shapes (Text/Badge) report the real total, not the truncated count —
        // a `limit = 5` widget with 23 cached entries should still say "📔 23 recent",
        // not "📔 5 recent". The user-set `limit` only governs how many ROWS render.
        Shape::Text => render_text(entries),
        Shape::Badge => render_badge(entries),
        Shape::TextBlock => render_text_block(&limited),
        Shape::MarkdownTextBlock => render_markdown(&limited),
        Shape::LinkedTextBlock => render_linked(&limited),
        Shape::Entries => render_entries(&limited),
        Shape::Calendar => render_calendar(entries, today),
        _ => render_timeline(&limited),
    }
}

fn render_text(entries: &[ApiEntry]) -> Body {
    let value = entries.first().map_or(String::new(), |e| {
        format!("📔 {} entries · latest: {}", entries.len(), e.title)
    });
    Body::Text(TextData { value })
}

fn render_text_block(entries: &[&ApiEntry]) -> Body {
    Body::TextBlock(TextBlockData {
        lines: entries.iter().map(|e| line_for(e)).collect(),
    })
}

fn render_markdown(entries: &[&ApiEntry]) -> Body {
    let value = entries
        .iter()
        .map(|e| format!("- **{}** — {}", short_date(&e.date), e.title))
        .collect::<Vec<_>>()
        .join("\n");
    Body::MarkdownTextBlock(MarkdownTextBlockData { value })
}

fn render_linked(entries: &[&ApiEntry]) -> Body {
    Body::LinkedTextBlock(LinkedTextBlockData {
        items: entries
            .iter()
            .map(|e| LinkedLine {
                text: line_for(e),
                url: Some(entry_url(&e.date)),
            })
            .collect(),
    })
}

fn render_entries(entries: &[&ApiEntry]) -> Body {
    Body::Entries(EntriesData {
        items: entries
            .iter()
            .map(|e| Entry {
                key: e.date.clone(),
                value: Some(e.title.clone()),
                status: None,
            })
            .collect(),
    })
}

fn render_badge(entries: &[ApiEntry]) -> Body {
    Body::Badge(BadgeData {
        status: if entries.is_empty() {
            Status::Warn
        } else {
            Status::Ok
        },
        label: format!("📔 {} recent", entries.len()),
    })
}

fn render_timeline(entries: &[&ApiEntry]) -> Body {
    Body::Timeline(TimelineData {
        events: entries
            .iter()
            .map(|e| TimelineEvent {
                timestamp: timestamp_for(e),
                title: e.title.clone(),
                detail: None,
                status: None,
            })
            .collect(),
    })
}

/// This month's grid with the days that received an entry marked as events. Useful for
/// "did I write today / how many days this month did I write" at a glance.
fn render_calendar(entries: &[ApiEntry], today: NaiveDate) -> Body {
    let events = entries
        .iter()
        .filter_map(|e| NaiveDate::parse_from_str(&e.date, "%Y-%m-%d").ok())
        .filter(|d| d.year() == today.year() && d.month() == today.month())
        .map(|d| d.day() as u8)
        .collect();
    Body::Calendar(CalendarData {
        year: today.year(),
        month: today.month() as u8,
        day: Some(today.day() as u8),
        events,
    })
}

fn line_for(e: &ApiEntry) -> String {
    format!("{} · {}", short_date(&e.date), e.title)
}

fn short_date(raw: &str) -> String {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map(|d| d.format("%m/%d").to_string())
        .unwrap_or_else(|_| raw.to_string())
}

fn timestamp_for(e: &ApiEntry) -> i64 {
    e.generated_at
        .as_deref()
        .map(parse_timestamp)
        .filter(|ts| *ts > 0)
        .or_else(|| {
            NaiveDate::parse_from_str(&e.date, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(12, 0, 0))
                .map(|dt| dt.and_utc().timestamp())
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(date: &str, title: &str) -> ApiEntry {
        ApiEntry {
            date: date.into(),
            title: title.into(),
            content: None,
            tags: vec![],
            sources: vec![],
            generated_at: None,
            word_count: None,
        }
    }

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn fixture() -> Vec<ApiEntry> {
        vec![
            entry("2026-04-27", "My Day"),
            entry("2026-04-26", "Sunday"),
            entry("2026-04-25", "Heatmap"),
        ]
    }

    #[test]
    fn timeline_uses_date_when_generated_at_missing() {
        let entries = fixture();
        let Body::Timeline(t) = render_body(&entries, Shape::Timeline, 10, ymd(2026, 4, 27)) else {
            panic!("expected Timeline");
        };
        assert_eq!(t.events.len(), 3);
        assert!(t.events.iter().all(|e| e.timestamp > 0));
        assert_eq!(t.events[0].title, "My Day");
    }

    #[test]
    fn timeline_prefers_generated_at_when_present() {
        let mut entries = fixture();
        entries[0].generated_at = Some("2026-04-27T08:00:00Z".into());
        let Body::Timeline(t) = render_body(&entries, Shape::Timeline, 10, ymd(2026, 4, 27)) else {
            panic!("expected Timeline");
        };
        assert_eq!(
            t.events[0].timestamp,
            parse_timestamp("2026-04-27T08:00:00Z")
        );
    }

    #[test]
    fn text_includes_count_and_latest_title() {
        let entries = fixture();
        let Body::Text(t) = render_body(&entries, Shape::Text, 10, ymd(2026, 4, 27)) else {
            panic!("expected Text");
        };
        assert!(t.value.contains("3 entries"));
        assert!(t.value.contains("My Day"));
    }

    #[test]
    fn text_block_formats_short_date_and_title() {
        let entries = fixture();
        let Body::TextBlock(t) = render_body(&entries, Shape::TextBlock, 10, ymd(2026, 4, 27))
        else {
            panic!("expected TextBlock");
        };
        assert_eq!(t.lines[0], "04/27 · My Day");
    }

    #[test]
    fn linked_text_block_links_each_row_to_its_entry_page() {
        let entries = fixture();
        let Body::LinkedTextBlock(l) =
            render_body(&entries, Shape::LinkedTextBlock, 10, ymd(2026, 4, 27))
        else {
            panic!("expected LinkedTextBlock");
        };
        assert_eq!(l.items.len(), 3);
        assert_eq!(
            l.items[0].url.as_deref(),
            Some("https://app.deariary.com/entries/2026/04/27")
        );
    }

    #[test]
    fn markdown_uses_bold_dates() {
        let entries = fixture();
        let Body::MarkdownTextBlock(m) =
            render_body(&entries, Shape::MarkdownTextBlock, 10, ymd(2026, 4, 27))
        else {
            panic!("expected MarkdownTextBlock");
        };
        assert!(m.value.contains("**04/27**"));
    }

    #[test]
    fn entries_keys_with_iso_date() {
        let entries = fixture();
        let Body::Entries(e) = render_body(&entries, Shape::Entries, 10, ymd(2026, 4, 27)) else {
            panic!("expected Entries");
        };
        assert_eq!(e.items[0].key, "2026-04-27");
        assert_eq!(e.items[0].value.as_deref(), Some("My Day"));
    }

    #[test]
    fn calendar_marks_entries_for_current_month_only() {
        let mut entries = fixture();
        entries.push(entry("2026-03-15", "Old"));
        let Body::Calendar(c) = render_body(&entries, Shape::Calendar, 10, ymd(2026, 4, 27)) else {
            panic!("expected Calendar");
        };
        assert_eq!(c.year, 2026);
        assert_eq!(c.month, 4);
        assert_eq!(c.events, vec![27, 26, 25]);
    }

    #[test]
    fn limit_caps_returned_rows() {
        let entries = fixture();
        let Body::TextBlock(t) = render_body(&entries, Shape::TextBlock, 2, ymd(2026, 4, 27))
        else {
            panic!("expected TextBlock");
        };
        assert_eq!(t.lines.len(), 2);
    }

    #[test]
    fn text_headline_reports_total_count_not_limit() {
        // limit governs how many rows the *list* renderers show. A 23-entry cache shown
        // through a Text headline should still say "23 entries", not the truncated count.
        let entries = vec![
            entry("2026-04-27", "A"),
            entry("2026-04-26", "B"),
            entry("2026-04-25", "C"),
            entry("2026-04-24", "D"),
            entry("2026-04-23", "E"),
        ];
        let Body::Text(t) = render_body(&entries, Shape::Text, 2, ymd(2026, 4, 27)) else {
            panic!("expected Text");
        };
        assert!(
            t.value.contains("5 entries"),
            "headline must use real total: {}",
            t.value
        );
    }

    #[test]
    fn badge_headline_reports_total_count_not_limit() {
        let entries = vec![
            entry("2026-04-27", "A"),
            entry("2026-04-26", "B"),
            entry("2026-04-25", "C"),
        ];
        let Body::Badge(b) = render_body(&entries, Shape::Badge, 1, ymd(2026, 4, 27)) else {
            panic!("expected Badge");
        };
        assert_eq!(
            b.label, "📔 3 recent",
            "badge must reflect total entries, not the row limit"
        );
    }

    #[test]
    fn empty_input_yields_empty_body_and_warn_badge() {
        let Body::Timeline(t) = render_body(&[], Shape::Timeline, 10, ymd(2026, 4, 27)) else {
            panic!("expected Timeline");
        };
        assert!(t.events.is_empty());

        let Body::Badge(b) = render_body(&[], Shape::Badge, 10, ymd(2026, 4, 27)) else {
            panic!("expected Badge");
        };
        assert_eq!(b.status, Status::Warn);
    }

    #[test]
    fn sample_body_provides_value_for_each_supported_shape() {
        let f = DeariaryRecent;
        for s in SHAPES {
            assert!(f.sample_body(*s).is_some(), "missing sample for {s:?}");
        }
        assert!(f.sample_body(Shape::Image).is_none());
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("limit = 5\nbogus = 1").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn short_date_falls_back_to_raw_when_unparseable() {
        assert_eq!(short_date("not-a-date"), "not-a-date");
    }
}
