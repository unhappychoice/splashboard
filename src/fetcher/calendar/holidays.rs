//! `calendar_holidays` — public-holiday data for a country, served as Calendar / Text /
//! TextBlock.
//!
//! Safety::Safe: host `date.nager.at` is hardcoded. The only config-driven input is the
//! ISO 3166-1 alpha-2 country code, validated locally — config can't redirect traffic.

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{Datelike, NaiveDate, Utc};
use reqwest::Client;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, CalendarData, Payload, TextBlockData, TextData};
use crate::render::Shape;

const API_BASE: &str = "https://date.nager.at/api/v3/PublicHolidays";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_LIMIT: u32 = 5;
const MIN_LIMIT: u32 = 1;
const MAX_LIMIT: u32 = 30;

const SHAPES: &[Shape] = &[Shape::Calendar, Shape::Text, Shape::TextBlock];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "country",
        type_hint: "string (ISO 3166-1 alpha-2, e.g. \"JP\" / \"US\")",
        required: true,
        default: None,
        description: "Country whose public holidays to fetch.",
    },
    OptionSchema {
        name: "language",
        type_hint: "\"local\" | \"english\"",
        required: false,
        default: Some("\"local\""),
        description: "Display-name source. `local` uses the country's localized name; `english` uses the English label.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("5"),
        description: "Maximum number of upcoming holidays to list (TextBlock shape only).",
    },
];

pub struct CalendarHolidaysFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub country: Option<String>,
    #[serde(default)]
    pub language: Option<Language>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    Local,
    English,
}

#[async_trait]
impl Fetcher for CalendarHolidaysFetcher {
    fn name(&self) -> &str {
        "calendar_holidays"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
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
            Shape::Calendar => sample_calendar(),
            Shape::Text => Body::Text(TextData {
                value: "🎌 Showa Day".into(),
            }),
            Shape::TextBlock => Body::TextBlock(TextBlockData {
                lines: vec![
                    "04/29 — Showa Day".into(),
                    "05/03 — Constitution Day".into(),
                    "05/04 — Greenery Day".into(),
                    "05/05 — Children's Day".into(),
                ],
            }),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let country = require_country(opts.country.as_deref())?;
        let language = opts.language.unwrap_or_default();
        let limit = opts
            .limit
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(MIN_LIMIT, MAX_LIMIT) as usize;
        let today = Utc::now().date_naive();
        let holidays = fetch_year(today.year(), &country).await?;
        let shape = ctx.shape.unwrap_or(Shape::Calendar);
        Ok(payload(render_body(
            &holidays, shape, today, language, limit,
        )))
    }
}

fn require_country(raw: Option<&str>) -> Result<String, FetchError> {
    let raw = raw.filter(|s| !s.is_empty()).ok_or_else(|| {
        FetchError::Failed("calendar_holidays requires `country` (ISO 3166-1 alpha-2)".into())
    })?;
    encode_country(raw).ok_or_else(|| {
        FetchError::Failed(format!(
            "invalid country code (need ISO 3166-1 alpha-2): {raw:?}"
        ))
    })
}

/// ISO 3166-1 alpha-2 sanitisation — strips non-letters, uppercases, requires exactly 2 chars.
/// Defense-in-depth even though the host is fixed: keeps the URL path segment trivial.
fn encode_country(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase();
    (cleaned.len() == 2).then_some(cleaned)
}

async fn fetch_year(year: i32, country: &str) -> Result<Vec<Holiday>, FetchError> {
    let url = format!("{API_BASE}/{year}/{country}");
    let res = http()
        .get(&url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("calendar_holidays request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("calendar_holidays {status}")));
    }
    res.json()
        .await
        .map_err(|e| FetchError::Failed(format!("calendar_holidays json parse: {e}")))
}

#[derive(Debug, Clone, Deserialize)]
struct Holiday {
    date: String,
    #[serde(rename = "localName")]
    local_name: String,
    name: String,
}

impl Holiday {
    fn parse_date(&self) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(&self.date, "%Y-%m-%d").ok()
    }
    fn label(&self, language: Language) -> &str {
        match language {
            Language::Local => &self.local_name,
            Language::English => &self.name,
        }
    }
}

fn render_body(
    holidays: &[Holiday],
    shape: Shape,
    today: NaiveDate,
    language: Language,
    limit: usize,
) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: today_label(holidays, today, language).unwrap_or_default(),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: upcoming_lines(holidays, today, language, limit),
        }),
        _ => Body::Calendar(month_calendar(holidays, today)),
    }
}

fn today_label(holidays: &[Holiday], today: NaiveDate, language: Language) -> Option<String> {
    holidays
        .iter()
        .find(|h| h.parse_date() == Some(today))
        .map(|h| format!("🎌 {}", h.label(language)))
}

fn upcoming_lines(
    holidays: &[Holiday],
    today: NaiveDate,
    language: Language,
    limit: usize,
) -> Vec<String> {
    let mut dated: Vec<(NaiveDate, &Holiday)> = holidays
        .iter()
        .filter_map(|h| Some((h.parse_date()?, h)))
        .filter(|(d, _)| *d >= today)
        .collect();
    dated.sort_by_key(|(d, _)| *d);
    dated
        .into_iter()
        .take(limit)
        .map(|(d, h)| format!("{:02}/{:02} — {}", d.month(), d.day(), h.label(language)))
        .collect()
}

fn month_calendar(holidays: &[Holiday], today: NaiveDate) -> CalendarData {
    let events: Vec<u8> = holidays
        .iter()
        .filter_map(|h| h.parse_date())
        .filter(|d| d.year() == today.year() && d.month() == today.month())
        .map(|d| d.day() as u8)
        .collect();
    CalendarData {
        year: today.year(),
        month: today.month() as u8,
        day: Some(today.day() as u8),
        events,
    }
}

fn sample_calendar() -> Body {
    Body::Calendar(CalendarData {
        year: 2026,
        month: 4,
        day: Some(29),
        events: vec![29],
    })
}

fn http() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .gzip(true)
            .build()
            .expect("reqwest client should build with default config")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(date: &str, local: &str, name: &str) -> Holiday {
        Holiday {
            date: date.into(),
            local_name: local.into(),
            name: name.into(),
        }
    }

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn options_default_to_none_for_every_field() {
        let opts = Options::default();
        assert!(opts.country.is_none());
        assert!(opts.language.is_none());
        assert!(opts.limit.is_none());
    }

    #[test]
    fn options_deserialize_country_language_limit() {
        let raw: toml::Value =
            toml::from_str("country = \"JP\"\nlanguage = \"english\"\nlimit = 3").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.country.as_deref(), Some("JP"));
        assert_eq!(opts.language, Some(Language::English));
        assert_eq!(opts.limit, Some(3));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("country = \"JP\"\nbogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn require_country_errors_when_missing_or_blank() {
        assert!(require_country(None).is_err());
        assert!(require_country(Some("")).is_err());
    }

    #[test]
    fn require_country_uppercases_and_validates_length() {
        assert_eq!(require_country(Some("jp")).unwrap(), "JP");
        assert!(require_country(Some("Japan")).is_err());
        assert!(require_country(Some("J")).is_err());
        assert!(require_country(Some("J1")).is_err());
    }

    #[test]
    fn encode_country_strips_non_letters_then_validates_length() {
        assert_eq!(encode_country("us").as_deref(), Some("US"));
        // Non-letters are filtered, then 2-char rule applies — "U.S" normalises to "US".
        assert_eq!(encode_country("U.S").as_deref(), Some("US"));
        assert_eq!(encode_country("USA").as_deref(), None);
        assert_eq!(encode_country("").as_deref(), None);
        assert_eq!(encode_country("12").as_deref(), None);
    }

    #[test]
    fn holiday_parse_date_reads_iso_format() {
        assert_eq!(
            h("2026-04-29", "x", "y").parse_date(),
            Some(ymd(2026, 4, 29))
        );
        assert!(h("not-a-date", "x", "y").parse_date().is_none());
    }

    #[test]
    fn holiday_label_switches_on_language() {
        let entry = h("2026-04-29", "昭和の日", "Showa Day");
        assert_eq!(entry.label(Language::Local), "昭和の日");
        assert_eq!(entry.label(Language::English), "Showa Day");
    }

    #[test]
    fn today_label_emits_emoji_prefix_when_today_matches() {
        let list = vec![h("2026-04-29", "昭和の日", "Showa Day")];
        assert_eq!(
            today_label(&list, ymd(2026, 4, 29), Language::Local),
            Some("🎌 昭和の日".into())
        );
    }

    #[test]
    fn today_label_returns_none_when_no_match() {
        let list = vec![h("2026-04-29", "昭和の日", "Showa Day")];
        assert!(today_label(&list, ymd(2026, 4, 30), Language::Local).is_none());
    }

    #[test]
    fn upcoming_lines_includes_today_and_filters_past() {
        let list = vec![
            h("2026-04-29", "Showa", "Showa Day"),
            h("2026-04-01", "April Fool", "April Fool"),
            h("2026-05-03", "Constitution", "Constitution Day"),
        ];
        let lines = upcoming_lines(&list, ymd(2026, 4, 29), Language::English, 5);
        assert_eq!(lines, vec!["04/29 — Showa Day", "05/03 — Constitution Day"]);
    }

    #[test]
    fn upcoming_lines_sorts_chronologically_and_caps_to_limit() {
        let list = vec![
            h("2026-12-23", "A", "A"),
            h("2026-05-03", "B", "B"),
            h("2026-07-19", "C", "C"),
        ];
        let lines = upcoming_lines(&list, ymd(2026, 5, 1), Language::English, 2);
        assert_eq!(lines, vec!["05/03 — B", "07/19 — C"]);
    }

    #[test]
    fn upcoming_lines_empty_when_all_past() {
        let list = vec![h("2026-01-01", "x", "x")];
        assert!(upcoming_lines(&list, ymd(2026, 12, 31), Language::Local, 5).is_empty());
    }

    #[test]
    fn month_calendar_filters_events_to_current_month() {
        let list = vec![
            h("2026-04-29", "x", "x"),
            h("2026-05-03", "y", "y"),
            h("2026-04-01", "z", "z"),
        ];
        let cal = month_calendar(&list, ymd(2026, 4, 15));
        assert_eq!(cal.year, 2026);
        assert_eq!(cal.month, 4);
        assert_eq!(cal.day, Some(15));
        assert_eq!(cal.events, vec![29, 1]);
    }

    #[test]
    fn render_body_text_emits_today_match_or_empty_string() {
        let list = vec![h("2026-04-29", "Showa", "Showa Day")];
        let body = render_body(&list, Shape::Text, ymd(2026, 4, 29), Language::English, 5);
        let Body::Text(t) = body else {
            panic!("expected Text")
        };
        assert_eq!(t.value, "🎌 Showa Day");

        let body = render_body(&list, Shape::Text, ymd(2026, 5, 1), Language::English, 5);
        let Body::Text(t) = body else {
            panic!("expected Text")
        };
        assert!(t.value.is_empty());
    }

    #[test]
    fn render_body_textblock_emits_upcoming() {
        let list = vec![h("2026-05-03", "x", "x")];
        let body = render_body(
            &list,
            Shape::TextBlock,
            ymd(2026, 5, 1),
            Language::English,
            5,
        );
        let Body::TextBlock(t) = body else {
            panic!("expected TextBlock")
        };
        assert_eq!(t.lines, vec!["05/03 — x"]);
    }

    #[test]
    fn render_body_calendar_default_branch_emits_calendar() {
        let body = render_body(&[], Shape::Calendar, ymd(2026, 4, 15), Language::Local, 5);
        let Body::Calendar(c) = body else {
            panic!("expected Calendar")
        };
        assert_eq!((c.year, c.month, c.day), (2026, 4, Some(15)));
    }

    #[test]
    fn sample_body_provides_a_value_for_each_supported_shape() {
        let f = CalendarHolidaysFetcher;
        for s in [Shape::Calendar, Shape::Text, Shape::TextBlock] {
            assert!(f.sample_body(s).is_some(), "missing sample for {s:?}");
        }
        assert!(f.sample_body(Shape::Heatmap).is_none());
    }

    #[test]
    fn holiday_deserializes_real_nager_payload() {
        let raw = r#"[
            { "date": "2026-04-29", "localName": "昭和の日", "name": "Showa Day", "countryCode": "JP" },
            { "date": "2026-05-03", "localName": "憲法記念日", "name": "Constitution Memorial Day", "countryCode": "JP" }
        ]"#;
        let holidays: Vec<Holiday> = serde_json::from_str(raw).unwrap();
        assert_eq!(holidays.len(), 2);
        assert_eq!(holidays[0].label(Language::Local), "昭和の日");
        assert_eq!(
            holidays[1].label(Language::English),
            "Constitution Memorial Day"
        );
    }

    #[test]
    fn limit_clamps_extremes() {
        assert_eq!(0u32.clamp(MIN_LIMIT, MAX_LIMIT), MIN_LIMIT);
        assert_eq!(999u32.clamp(MIN_LIMIT, MAX_LIMIT), MAX_LIMIT);
        assert_eq!(DEFAULT_LIMIT.clamp(MIN_LIMIT, MAX_LIMIT), DEFAULT_LIMIT);
    }

    /// Live smoke test — hits date.nager.at. `#[ignore]` keeps CI offline-safe.
    #[tokio::test]
    #[ignore]
    async fn live_jp_returns_holidays_for_current_year() {
        let today = Utc::now().date_naive();
        let holidays = fetch_year(today.year(), "JP").await.unwrap();
        assert!(!holidays.is_empty());
        for h in &holidays {
            eprintln!("{} — {} / {}", h.date, h.local_name, h.name);
        }
    }
}
