//! `deariary_today` — the most recent auto-generated diary entry.
//!
//! deariary.com aggregates external tools (GitHub, Calendar, Slack, Linear, ...) into a
//! written entry each morning, covering roughly the previous 24 hours. This fetcher targets
//! whichever entry is freshest right now: usually today's, falling back to yesterday's when
//! today's hasn't been generated yet (e.g. early morning splash). Requires the deariary
//! Advanced plan and a Bearer token via `options.token` or `DEARIARY_TOKEN` env.
//!
//! Safety::Safe: host `api.deariary.com` is hardcoded.

use async_trait::async_trait;
use chrono::{Local, NaiveDate};
use serde::Deserialize;

use super::client::{
    ApiEntry, cache_extra, cached_get_entries, cached_get_entry, entry_url, resolve_token,
};
use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData, MarkdownTextBlockData,
    Payload, Status, TextBlockData, TextData,
};
use crate::render::Shape;
use crate::samples;

const SHAPES: &[Shape] = &[
    Shape::TextBlock,
    Shape::Text,
    Shape::MarkdownTextBlock,
    Shape::LinkedTextBlock,
    Shape::Entries,
    Shape::Badge,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "token",
    type_hint: "string",
    required: false,
    default: None,
    description: "Deariary API token (Bearer). Falls back to the `DEARIARY_TOKEN` env var.",
}];

pub struct DeariaryToday;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    token: Option<String>,
}

#[async_trait]
impl Fetcher for DeariaryToday {
    fn name(&self) -> &str {
        "deariary_today"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "The most recent auto-generated deariary.com entry — typically today's, falling back to yesterday's when today's hasn't generated yet. Aggregated from external tools (GitHub, Calendar, Slack, Linear, ...); requires the Advanced plan. `TextBlock` shows the body, `MarkdownTextBlock` preserves formatting for `text_markdown`, `LinkedTextBlock` is a one-line clickable headline that opens the entry on deariary.com, `Entries` summarises title / date / tags / sources, `Text` is a one-line headline, `Badge` is a status pill."
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
            Shape::Text => samples::text("📔 My adventurous day"),
            Shape::TextBlock => samples::text_block(&[
                "Caught the morning train to the office and shipped the heatmap renderer.",
                "Reviewed two pull requests with the team and wrote release notes.",
                "Wrapped up with a calm dinner and a long walk along the river.",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "## My adventurous day\n\nCaught the morning train to the office and shipped the heatmap renderer.\n\n- 🐱 GitHub: 3 commits, 2 PRs reviewed\n- 📅 Calendar: 1:1 with manager",
            ),
            Shape::LinkedTextBlock => samples::linked_text_block(&[(
                "📔 My adventurous day",
                Some("https://app.deariary.com/entries/2026/04/27"),
            )]),
            Shape::Entries => samples::entries(&[
                ("title", "My adventurous day"),
                ("date", "2026-04-27"),
                ("tags", "work, travel"),
                ("sources", "github, calendar"),
            ]),
            Shape::Badge => Body::Badge(BadgeData {
                status: Status::Ok,
                label: "📔 today".into(),
            }),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let token = resolve_token(opts.token.as_deref())?;
        let today = Local::now().date_naive();
        let entry = latest_entry(&token, today).await?;
        let shape = ctx.shape.unwrap_or(Shape::TextBlock);
        Ok(payload(render_body(entry.as_ref(), shape, today)))
    }
}

/// Pulls the freshest entry, but only when it's recent enough to be honest about. The
/// fetcher's description promises "typically today's, falling back to yesterday's when
/// today's hasn't generated yet" — surfacing a 5-day-old entry as "today" would be a
/// silent lie. If the newest available entry isn't from today or yesterday, return
/// `Ok(None)` and let the empty-state placeholder render instead.
///
/// The list call is cached and shared with `deariary_recent`; the per-date lookup is
/// cached and shared with `deariary_on_this_day` — `/entries` list responses may strip
/// content for size, so we re-fetch the full body via the date endpoint.
async fn latest_entry(token: &str, today: NaiveDate) -> Result<Option<ApiEntry>, FetchError> {
    let entries = cached_get_entries(token, None).await?;
    let Some(latest) = entries.first() else {
        return Ok(None);
    };
    if !is_today_or_yesterday(&latest.date, today) {
        return Ok(None);
    }
    cached_get_entry(token, &latest.date).await
}

fn is_today_or_yesterday(raw: &str, today: NaiveDate) -> bool {
    let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") else {
        return false;
    };
    let days_back = (today - date).num_days();
    (0..=1).contains(&days_back)
}

fn render_body(entry: Option<&ApiEntry>, shape: Shape, today: NaiveDate) -> Body {
    match entry {
        None => empty_body(shape),
        Some(e) => match shape {
            Shape::Text => render_text(e),
            Shape::MarkdownTextBlock => render_markdown(e),
            Shape::LinkedTextBlock => render_linked(e),
            Shape::Entries => render_entries(e),
            Shape::Badge => render_badge(e, today),
            _ => render_text_block(e),
        },
    }
}

fn render_text(e: &ApiEntry) -> Body {
    Body::Text(TextData {
        value: format!("📔 {}", e.title),
    })
}

fn render_text_block(e: &ApiEntry) -> Body {
    Body::TextBlock(TextBlockData {
        lines: content_lines(e),
    })
}

fn render_markdown(e: &ApiEntry) -> Body {
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: e.content.clone().unwrap_or_default(),
    })
}

/// Single-row clickable headline opening the entry on deariary.com. Multi-line content is
/// the job of `TextBlock` / `MarkdownTextBlock`; here the user just wants one line they can
/// click to keep reading.
fn render_linked(e: &ApiEntry) -> Body {
    Body::LinkedTextBlock(LinkedTextBlockData {
        items: vec![LinkedLine {
            text: format!("📔 {}", e.title),
            url: Some(entry_url(&e.date)),
        }],
    })
}

fn render_entries(e: &ApiEntry) -> Body {
    let items = [
        ("title", Some(e.title.clone())),
        ("date", Some(e.date.clone())),
        ("tags", (!e.tags.is_empty()).then(|| e.tags.join(", "))),
        (
            "sources",
            (!e.sources.is_empty()).then(|| e.sources.join(", ")),
        ),
    ]
    .into_iter()
    .filter_map(|(key, value)| {
        value.map(|v| Entry {
            key: key.into(),
            value: Some(v),
            status: None,
        })
    })
    .collect();
    Body::Entries(EntriesData { items })
}

fn render_badge(e: &ApiEntry, today: NaiveDate) -> Body {
    Body::Badge(BadgeData {
        status: Status::Ok,
        label: format!("📔 {}", relative_day(&e.date, today)),
    })
}

/// `today` / `yesterday` / falls back to the raw ISO date for older entries — keeps the badge
/// label honest when today's hasn't generated yet and the fetcher returns yesterday's instead.
fn relative_day(raw: &str, today: NaiveDate) -> String {
    let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") else {
        return raw.to_string();
    };
    match (today - date).num_days() {
        0 => "today".into(),
        1 => "yesterday".into(),
        _ => raw.to_string(),
    }
}

fn content_lines(e: &ApiEntry) -> Vec<String> {
    e.content
        .as_deref()
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn empty_body(shape: Shape) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: String::new(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: String::new(),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData { items: vec![] }),
        Shape::Entries => Body::Entries(EntriesData { items: vec![] }),
        Shape::Badge => Body::Badge(BadgeData {
            status: Status::Warn,
            label: "📔 no entry yet".into(),
        }),
        _ => Body::TextBlock(TextBlockData { lines: vec![] }),
    }
}

#[cfg(test)]
mod tests {
    use std::{future::Future, time::Duration};

    use super::*;

    fn entry() -> ApiEntry {
        ApiEntry {
            date: "2026-04-27".into(),
            title: "My Day".into(),
            content: Some("# Hello\n\nWorld".into()),
            tags: vec!["work".into(), "travel".into()],
            sources: vec!["github".into(), "calendar".into()],
            generated_at: Some("2026-04-27T08:00:00Z".into()),
            word_count: Some(482),
        }
    }

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn ctx(options: Option<&str>, shape: Option<Shape>, format: Option<&str>) -> FetchContext {
        FetchContext {
            widget_id: "deariary-today".into(),
            format: format.map(str::to_string),
            timeout: Duration::from_secs(1),
            file_format: None,
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            timezone: None,
            locale: None,
        }
    }

    fn run_async<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn fetcher_catalog_surface_matches_contract() {
        let fetcher = DeariaryToday;
        assert_eq!(fetcher.name(), "deariary_today");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.default_shape(), Shape::TextBlock);
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.option_schemas().len(), 1);
        assert_eq!(fetcher.option_schemas()[0].name, "token");
        assert!(fetcher.description().contains("most recent auto-generated"));
    }

    #[test]
    fn cache_key_changes_with_token_shape_and_format() {
        let fetcher = DeariaryToday;
        let base = fetcher.cache_key(&ctx(
            Some("token = \"alpha\""),
            Some(Shape::TextBlock),
            Some("plain"),
        ));
        let same = fetcher.cache_key(&ctx(
            Some("token = \"alpha\""),
            Some(Shape::TextBlock),
            Some("plain"),
        ));
        let different_token = fetcher.cache_key(&ctx(
            Some("token = \"beta\""),
            Some(Shape::TextBlock),
            Some("plain"),
        ));
        let different_shape = fetcher.cache_key(&ctx(
            Some("token = \"alpha\""),
            Some(Shape::Badge),
            Some("plain"),
        ));
        let different_format = fetcher.cache_key(&ctx(
            Some("token = \"alpha\""),
            Some(Shape::TextBlock),
            Some("json"),
        ));

        assert_eq!(base, same);
        assert_ne!(base, different_token);
        assert_ne!(base, different_shape);
        assert_ne!(base, different_format);
        assert!(
            !fetcher
                .cache_key(&ctx(Some("bogus = true"), None, None))
                .is_empty()
        );
    }

    #[test]
    fn text_body_uses_title_only() {
        assert!(matches!(
            render_body(Some(&entry()), Shape::Text, ymd(2026, 4, 27)),
            Body::Text(TextData { value }) if value.contains("My Day") && !value.contains("482")
        ));
    }

    #[test]
    fn text_block_splits_content_into_lines() {
        assert!(matches!(
            render_body(Some(&entry()), Shape::TextBlock, ymd(2026, 4, 27)),
            Body::TextBlock(TextBlockData { lines })
                if lines
                    == vec![
                        "# Hello".to_string(),
                        String::new(),
                        "World".to_string(),
                    ]
        ));
    }

    #[test]
    fn linked_text_block_emits_one_clickable_headline() {
        assert!(matches!(
            render_body(Some(&entry()), Shape::LinkedTextBlock, ymd(2026, 4, 27)),
            Body::LinkedTextBlock(LinkedTextBlockData { items })
                if items.len() == 1
                    && items[0].text.contains("My Day")
                    && items[0].url.as_deref()
                        == Some("https://app.deariary.com/entries/2026/04/27")
        ));
    }

    #[test]
    fn markdown_block_passes_content_through_verbatim() {
        assert!(matches!(
            render_body(Some(&entry()), Shape::MarkdownTextBlock, ymd(2026, 4, 27)),
            Body::MarkdownTextBlock(MarkdownTextBlockData { value })
                if value == "# Hello\n\nWorld"
        ));
    }

    #[test]
    fn entries_includes_only_present_metadata_keys() {
        let mut bare = entry();
        bare.tags.clear();
        bare.sources.clear();
        assert!(matches!(
            render_body(Some(&bare), Shape::Entries, ymd(2026, 4, 27)),
            Body::Entries(EntriesData { items })
                if items.iter().map(|item| item.key.as_str()).collect::<Vec<_>>()
                    == vec!["title", "date"]
        ));
    }

    #[test]
    fn entries_joins_tags_and_sources_when_present() {
        assert!(matches!(
            render_body(Some(&entry()), Shape::Entries, ymd(2026, 4, 27)),
            Body::Entries(EntriesData { items })
                if items
                    .iter()
                    .find(|item| item.key == "tags")
                    .and_then(|item| item.value.as_deref())
                    == Some("work, travel")
        ));
    }

    #[test]
    fn badge_label_says_today_when_entry_date_matches_today() {
        assert!(matches!(
            render_body(Some(&entry()), Shape::Badge, ymd(2026, 4, 27)),
            Body::Badge(BadgeData { status: Status::Ok, label }) if label == "📔 today"
        ));
    }

    #[test]
    fn badge_label_says_yesterday_when_today_has_not_generated_yet() {
        assert!(matches!(
            render_body(Some(&entry()), Shape::Badge, ymd(2026, 4, 28)),
            Body::Badge(BadgeData { label, .. }) if label == "📔 yesterday"
        ));
    }

    #[test]
    fn badge_label_falls_back_to_iso_date_for_older_entries() {
        assert!(matches!(
            render_body(Some(&entry()), Shape::Badge, ymd(2026, 5, 1)),
            Body::Badge(BadgeData { label, .. }) if label == "📔 2026-04-27"
        ));
    }

    #[test]
    fn freshness_gate_accepts_today_and_yesterday_only() {
        // The fetcher's contract is "typically today's, falling back to yesterday's" — a
        // 5-day-old entry must NOT be served as "today" or even "yesterday". Bound the
        // gate to a 0/1-day delta and let `Ok(None)` fall through to the empty-state
        // placeholder for older surrogates.
        let today = ymd(2026, 4, 27);
        assert!(is_today_or_yesterday("2026-04-27", today));
        assert!(is_today_or_yesterday("2026-04-26", today));
        assert!(!is_today_or_yesterday("2026-04-25", today));
        assert!(!is_today_or_yesterday("2026-04-22", today));
        // Future-dated entries (clock skew, time-zone weirdness on the API side) shouldn't
        // be silently surfaced either — guard against them too.
        assert!(!is_today_or_yesterday("2026-04-28", today));
        // Unparseable dates fall through to false; the surrogate placeholder beats
        // surfacing an entry the fetcher can't reason about.
        assert!(!is_today_or_yesterday("not-a-date", today));
    }

    #[test]
    fn missing_entry_emits_empty_text_block() {
        assert!(matches!(
            render_body(None, Shape::TextBlock, ymd(2026, 4, 27)),
            Body::TextBlock(TextBlockData { lines }) if lines.is_empty()
        ));
    }

    #[test]
    fn missing_entry_covers_other_empty_shapes() {
        let today = ymd(2026, 4, 27);
        assert!(matches!(
            render_body(None, Shape::Text, today),
            Body::Text(TextData { value }) if value.is_empty()
        ));
        assert!(matches!(
            render_body(None, Shape::MarkdownTextBlock, today),
            Body::MarkdownTextBlock(MarkdownTextBlockData { value }) if value.is_empty()
        ));
        assert!(matches!(
            render_body(None, Shape::LinkedTextBlock, today),
            Body::LinkedTextBlock(LinkedTextBlockData { items }) if items.is_empty()
        ));
        assert!(matches!(
            render_body(None, Shape::Entries, today),
            Body::Entries(EntriesData { items }) if items.is_empty()
        ));
    }

    #[test]
    fn missing_entry_emits_warn_badge() {
        assert!(matches!(
            render_body(None, Shape::Badge, ymd(2026, 4, 27)),
            Body::Badge(BadgeData {
                status: Status::Warn,
                label,
            }) if label.contains("no entry")
        ));
    }

    #[test]
    fn relative_day_returns_raw_text_for_invalid_dates() {
        assert_eq!(relative_day("not-a-date", ymd(2026, 4, 27)), "not-a-date");
    }

    #[test]
    fn sample_body_provides_value_for_each_supported_shape() {
        let f = DeariaryToday;
        for s in SHAPES {
            assert!(f.sample_body(*s).is_some(), "missing sample for {s:?}");
        }
        assert!(f.sample_body(Shape::Heatmap).is_none());
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("token = \"abc\"\nbogus = 1").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_options() {
        let fetcher = DeariaryToday;
        let err = fetcher
            .fetch(&ctx(
                Some("token = \"abc\"\nbogus = true"),
                Some(Shape::TextBlock),
                None,
            ))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(msg) if msg.contains("unknown field `bogus`")
        ));
    }

    #[test]
    fn fetch_requires_token_before_network() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var("DEARIARY_TOKEN").ok();
        unsafe { std::env::remove_var("DEARIARY_TOKEN") };

        let fetcher = DeariaryToday;
        let err = run_async(fetcher.fetch(&ctx(None, Some(Shape::TextBlock), None))).unwrap_err();

        assert!(matches!(
            err,
            FetchError::Failed(msg)
                if msg == "deariary token missing: set options.token or DEARIARY_TOKEN"
        ));

        unsafe {
            match previous {
                Some(value) => std::env::set_var("DEARIARY_TOKEN", value),
                None => std::env::remove_var("DEARIARY_TOKEN"),
            }
        }
    }
}
