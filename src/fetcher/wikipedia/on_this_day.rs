//! `wikipedia_on_this_day` — historical events that occurred on today's calendar date,
//! linked to their Wikipedia articles.
//!
//! Safety::Safe: host is `*.wikipedia.org`. The `lang` option picks a language subdomain;
//! it can't redirect traffic outside Wikipedia.

use async_trait::async_trait;
use chrono::{Datelike, NaiveDate, Utc};
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    Body, LinkedLine, LinkedTextBlockData, Payload, TextBlockData, TextData, TimelineData,
    TimelineEvent,
};
use crate::render::Shape;
use crate::samples;

use super::client::{DEFAULT_LANG, PageSummary, get, rest_api_base};

const DEFAULT_LIMIT: u32 = 5;
const MIN_LIMIT: u32 = 1;
const MAX_LIMIT: u32 = 30;

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::TextBlock,
    Shape::Text,
    Shape::Timeline,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "kind",
        type_hint: "\"selected\" | \"events\" | \"births\" | \"deaths\" | \"holidays\" | \"all\"",
        required: false,
        default: Some("\"selected\""),
        description: "Which slice of the on-this-day feed to fetch. `selected` is the curated highlights.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("5"),
        description: "Maximum number of events to display.",
    },
    OptionSchema {
        name: "lang",
        type_hint: "string (Wikipedia language code, e.g. `\"en\"` / `\"ja\"`)",
        required: false,
        default: Some("\"en\""),
        description: "Wikipedia language edition to query.",
    },
];

pub struct WikipediaOnThisDayFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub kind: Option<Kind>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub lang: Option<String>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Selected,
    Events,
    Births,
    Deaths,
    Holidays,
    All,
}

impl Kind {
    fn endpoint(self) -> &'static str {
        match self {
            Self::Selected => "selected",
            Self::Events => "events",
            Self::Births => "births",
            Self::Deaths => "deaths",
            Self::Holidays => "holidays",
            Self::All => "all",
        }
    }
}

#[async_trait]
impl Fetcher for WikipediaOnThisDayFetcher {
    fn name(&self) -> &str {
        "wikipedia_on_this_day"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Events that happened on today's calendar date in history, sourced from Wikipedia's on-this-day feed (selected highlights, events, births, deaths, holidays, or all). Use `wikipedia_featured` for today's featured article or `wikipedia_random` for an arbitrary page."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let extra = ctx
            .options
            .as_ref()
            .and_then(|v| toml::to_string(v).ok())
            .unwrap_or_default();
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "1969 — Apollo 11 lands on the Moon",
                    Some("https://en.wikipedia.org/wiki/Apollo_11"),
                ),
                (
                    "1492 — Columbus departs Palos de la Frontera",
                    Some("https://en.wikipedia.org/wiki/Christopher_Columbus"),
                ),
                (
                    "1989 — Tim Berners-Lee proposes the World Wide Web",
                    Some("https://en.wikipedia.org/wiki/World_Wide_Web"),
                ),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "1969 — Apollo 11 lands on the Moon",
                "1492 — Columbus departs Palos de la Frontera",
                "1989 — Tim Berners-Lee proposes the World Wide Web",
            ]),
            Shape::Text => samples::text("1969 — Apollo 11 lands on the Moon"),
            Shape::Timeline => samples::timeline(&[
                (
                    -86_400 * 365 * 2, // doesn't matter much; demo only
                    "1969 — Apollo 11 lands on the Moon",
                    None,
                ),
                (
                    -86_400 * 365 * 4,
                    "1492 — Columbus departs Palos de la Frontera",
                    None,
                ),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let kind = opts.kind.unwrap_or_default();
        let lang = opts.lang.as_deref().unwrap_or(DEFAULT_LANG);
        let limit = opts
            .limit
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(MIN_LIMIT, MAX_LIMIT) as usize;
        let events = fetch_events(lang, kind).await?;
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        Ok(payload(render_body(&events, shape, limit)))
    }
}

async fn fetch_events(lang: &str, kind: Kind) -> Result<Vec<EventEntry>, FetchError> {
    let now = Utc::now();
    let (month, day) = (now.month(), now.day());
    let url = format!(
        "{}/feed/onthisday/{}/{:02}/{:02}",
        rest_api_base(lang),
        kind.endpoint(),
        month,
        day
    );
    let response: OnThisDayResponse = get(&url).await?;
    Ok(response.into_events(kind, now.year(), month, day))
}

#[derive(Debug, Default, Deserialize)]
struct OnThisDayResponse {
    #[serde(default)]
    selected: Vec<RawEvent>,
    #[serde(default)]
    events: Vec<RawEvent>,
    #[serde(default)]
    births: Vec<RawEvent>,
    #[serde(default)]
    deaths: Vec<RawEvent>,
    #[serde(default)]
    holidays: Vec<RawEvent>,
}

impl OnThisDayResponse {
    fn into_events(self, kind: Kind, today_year: i32, month: u32, day: u32) -> Vec<EventEntry> {
        let raw = match kind {
            Kind::Selected => self.selected,
            Kind::Events => self.events,
            Kind::Births => self.births,
            Kind::Deaths => self.deaths,
            Kind::Holidays => self.holidays,
            Kind::All => {
                let mut all = self.selected;
                all.extend(self.events);
                all.extend(self.births);
                all.extend(self.deaths);
                all.extend(self.holidays);
                all
            }
        };
        raw.into_iter()
            .map(|r| EventEntry::from_raw(r, today_year, month, day))
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct RawEvent {
    #[serde(default)]
    text: String,
    #[serde(default)]
    year: Option<i32>,
    #[serde(default)]
    pages: Vec<PageSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EventEntry {
    label: String,
    url: Option<String>,
    timestamp: i64,
}

impl EventEntry {
    fn from_raw(raw: RawEvent, today_year: i32, month: u32, day: u32) -> Self {
        let label = match raw.year {
            Some(year) => format!("{year} — {}", raw.text),
            None => raw.text,
        };
        let url = raw.pages.first().and_then(|p| p.url());
        let year = raw.year.unwrap_or(today_year);
        Self {
            label,
            url,
            timestamp: anniversary_timestamp(year, month, day),
        }
    }
}

fn anniversary_timestamp(year: i32, month: u32, day: u32) -> i64 {
    let date = NaiveDate::from_ymd_opt(year, month, day)
        .or_else(|| NaiveDate::from_ymd_opt(year, month, 1))
        .or_else(|| NaiveDate::from_ymd_opt(year, 1, 1));
    date.and_then(|d| d.and_hms_opt(12, 0, 0))
        .map(|dt| dt.and_utc().timestamp())
        .unwrap_or(0)
}

fn render_body(events: &[EventEntry], shape: Shape, limit: usize) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: events.first().map(|e| e.label.clone()).unwrap_or_default(),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: events.iter().take(limit).map(|e| e.label.clone()).collect(),
        }),
        Shape::Timeline => Body::Timeline(TimelineData {
            events: events
                .iter()
                .take(limit)
                .map(|e| TimelineEvent {
                    timestamp: e.timestamp,
                    title: e.label.clone(),
                    detail: None,
                    status: None,
                })
                .collect(),
        }),
        _ => Body::LinkedTextBlock(LinkedTextBlockData {
            items: events
                .iter()
                .take(limit)
                .map(|e| LinkedLine {
                    text: e.label.clone(),
                    url: e.url.clone(),
                })
                .collect(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetcher::wikipedia::client::{ContentUrls, UrlsByPlatform};

    fn page(title: &str, url: &str) -> PageSummary {
        PageSummary {
            title: title.into(),
            extract: None,
            content_urls: Some(ContentUrls {
                desktop: Some(UrlsByPlatform { page: url.into() }),
                mobile: None,
            }),
        }
    }

    fn raw_event(year: Option<i32>, text: &str, pages: Vec<PageSummary>) -> RawEvent {
        RawEvent {
            text: text.into(),
            year,
            pages,
        }
    }

    #[test]
    fn options_default_to_none_for_every_field() {
        let opts = Options::default();
        assert!(opts.kind.is_none());
        assert!(opts.limit.is_none());
        assert!(opts.lang.is_none());
    }

    #[test]
    fn options_deserialize_kind_lang_limit() {
        let raw: toml::Value =
            toml::from_str("kind = \"births\"\nlimit = 3\nlang = \"ja\"").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.kind, Some(Kind::Births));
        assert_eq!(opts.limit, Some(3));
        assert_eq!(opts.lang.as_deref(), Some("ja"));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("kind = \"selected\"\nbogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn kind_endpoint_covers_every_variant() {
        assert_eq!(Kind::Selected.endpoint(), "selected");
        assert_eq!(Kind::Events.endpoint(), "events");
        assert_eq!(Kind::Births.endpoint(), "births");
        assert_eq!(Kind::Deaths.endpoint(), "deaths");
        assert_eq!(Kind::Holidays.endpoint(), "holidays");
        assert_eq!(Kind::All.endpoint(), "all");
    }

    #[test]
    fn event_entry_prefixes_year_when_present() {
        let e = EventEntry::from_raw(
            raw_event(Some(1969), "Apollo 11 lands.", vec![]),
            2026,
            4,
            27,
        );
        assert_eq!(e.label, "1969 — Apollo 11 lands.");
        assert!(e.url.is_none());
    }

    #[test]
    fn event_entry_omits_year_dash_when_year_missing() {
        // Holidays don't have a `year` field — label stays as the raw text.
        let e = EventEntry::from_raw(raw_event(None, "Independence Day", vec![]), 2026, 4, 27);
        assert_eq!(e.label, "Independence Day");
    }

    #[test]
    fn event_entry_url_uses_first_page() {
        let e = EventEntry::from_raw(
            raw_event(
                Some(1969),
                "Apollo 11.",
                vec![
                    page("Apollo 11", "https://en.wikipedia.org/wiki/Apollo_11"),
                    page("Moon", "https://en.wikipedia.org/wiki/Moon"),
                ],
            ),
            2026,
            4,
            27,
        );
        assert_eq!(
            e.url.as_deref(),
            Some("https://en.wikipedia.org/wiki/Apollo_11")
        );
    }

    #[test]
    fn event_entry_timestamp_uses_event_year_with_today_month_day() {
        let e = EventEntry::from_raw(raw_event(Some(1969), "Apollo 11.", vec![]), 2026, 7, 20);
        let date = NaiveDate::from_ymd_opt(1969, 7, 20)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(e.timestamp, date);
    }

    #[test]
    fn event_entry_timestamp_falls_back_to_today_year_when_missing() {
        let e = EventEntry::from_raw(raw_event(None, "Independence Day", vec![]), 2026, 7, 4);
        let date = NaiveDate::from_ymd_opt(2026, 7, 4)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(e.timestamp, date);
    }

    fn entry(label: &str, url: Option<&str>) -> EventEntry {
        EventEntry {
            label: label.into(),
            url: url.map(String::from),
            timestamp: 0,
        }
    }

    #[test]
    fn linked_text_block_default_carries_per_event_url() {
        let events = vec![
            entry("1969 — A", Some("https://en.wikipedia.org/A")),
            entry("1066 — B", None),
        ];
        let body = render_body(&events, Shape::LinkedTextBlock, 5);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked text block");
        };
        assert_eq!(b.items.len(), 2);
        assert_eq!(b.items[0].text, "1969 — A");
        assert_eq!(
            b.items[0].url.as_deref(),
            Some("https://en.wikipedia.org/A")
        );
        assert!(b.items[1].url.is_none());
    }

    #[test]
    fn text_block_drops_links_keeps_label_lines() {
        let events = vec![
            entry("1969 — A", Some("https://en.wikipedia.org/A")),
            entry("1066 — B", None),
        ];
        let body = render_body(&events, Shape::TextBlock, 5);
        let Body::TextBlock(t) = body else {
            panic!("expected text block");
        };
        assert_eq!(t.lines, vec!["1969 — A", "1066 — B"]);
    }

    #[test]
    fn text_shape_emits_first_event_label() {
        let events = vec![entry("1969 — A", None), entry("1066 — B", None)];
        let body = render_body(&events, Shape::Text, 5);
        let Body::Text(t) = body else {
            panic!("expected text");
        };
        assert_eq!(t.value, "1969 — A");
    }

    #[test]
    fn text_shape_is_empty_when_no_events() {
        let body = render_body(&[], Shape::Text, 5);
        let Body::Text(t) = body else {
            panic!("expected text");
        };
        assert!(t.value.is_empty());
    }

    #[test]
    fn limit_caps_emitted_rows() {
        let events: Vec<_> = (0..10).map(|i| entry(&format!("e{i}"), None)).collect();
        let body = render_body(&events, Shape::LinkedTextBlock, 3);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked text block");
        };
        assert_eq!(b.items.len(), 3);
    }

    #[test]
    fn empty_events_produce_empty_linked_text_block() {
        let body = render_body(&[], Shape::LinkedTextBlock, 5);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked text block");
        };
        assert!(b.items.is_empty());
    }

    #[test]
    fn into_events_kind_selected_returns_only_selected_slice() {
        let resp = OnThisDayResponse {
            selected: vec![raw_event(Some(1), "s", vec![])],
            events: vec![raw_event(Some(2), "e", vec![])],
            ..Default::default()
        };
        let events = resp.into_events(Kind::Selected, 2026, 4, 27);
        assert_eq!(events.len(), 1);
        assert!(events[0].label.contains("s"));
    }

    #[test]
    fn into_events_kind_all_concatenates_every_slice() {
        let resp = OnThisDayResponse {
            selected: vec![raw_event(Some(1), "s", vec![])],
            events: vec![raw_event(Some(2), "e", vec![])],
            births: vec![raw_event(Some(3), "b", vec![])],
            deaths: vec![raw_event(Some(4), "d", vec![])],
            holidays: vec![raw_event(None, "h", vec![])],
        };
        let events = resp.into_events(Kind::All, 2026, 4, 27);
        assert_eq!(events.len(), 5);
    }

    #[test]
    fn limit_clamps_extremes() {
        assert_eq!(0u32.clamp(MIN_LIMIT, MAX_LIMIT), MIN_LIMIT);
        assert_eq!(999u32.clamp(MIN_LIMIT, MAX_LIMIT), MAX_LIMIT);
        assert_eq!(DEFAULT_LIMIT.clamp(MIN_LIMIT, MAX_LIMIT), DEFAULT_LIMIT);
    }

    #[test]
    fn response_deserializes_realistic_payload() {
        let raw = r#"{
            "selected": [
                { "year": 1969, "text": "Apollo 11.", "pages": [
                    { "title": "Apollo 11",
                      "content_urls": { "desktop": { "page": "https://en.wikipedia.org/wiki/Apollo_11" } } }
                ]}
            ],
            "holidays": [
                { "text": "Independence Day", "pages": [] }
            ]
        }"#;
        let r: OnThisDayResponse = serde_json::from_str(raw).unwrap();
        let selected = r.into_events(Kind::Selected, 2026, 4, 27);
        assert_eq!(selected[0].label, "1969 — Apollo 11.");
        assert_eq!(
            selected[0].url.as_deref(),
            Some("https://en.wikipedia.org/wiki/Apollo_11")
        );
    }

    #[test]
    fn timeline_body_carries_anniversary_timestamps() {
        let events = vec![
            EventEntry {
                label: "1969 — A".into(),
                url: None,
                timestamp: 12345,
            },
            EventEntry {
                label: "1066 — B".into(),
                url: None,
                timestamp: 67890,
            },
        ];
        let body = render_body(&events, Shape::Timeline, 5);
        let Body::Timeline(t) = body else {
            panic!("expected timeline");
        };
        assert_eq!(t.events.len(), 2);
        assert_eq!(t.events[0].timestamp, 12345);
        assert_eq!(t.events[0].title, "1969 — A");
        assert_eq!(t.events[1].timestamp, 67890);
    }

    /// Live smoke test — hits Wikipedia REST API. `#[ignore]` keeps CI offline-safe.
    #[tokio::test]
    #[ignore]
    async fn live_returns_at_least_one_event() {
        let events = fetch_events("en", Kind::Selected).await.unwrap();
        assert!(!events.is_empty());
        for e in &events {
            eprintln!("{} → {}", e.label, e.url.as_deref().unwrap_or("(no url)"));
        }
    }
}
