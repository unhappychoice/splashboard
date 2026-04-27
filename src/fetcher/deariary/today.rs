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

use super::client::{ApiEntry, cached_get_entries, cached_get_entry, entry_url, resolve_token};
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
        let extra = ctx
            .options
            .as_ref()
            .and_then(|v| toml::to_string(v).ok())
            .unwrap_or_default();
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
        let entry = latest_entry(&token).await?;
        let shape = ctx.shape.unwrap_or(Shape::TextBlock);
        let today = Local::now().date_naive();
        Ok(payload(render_body(entry.as_ref(), shape, today)))
    }
}

/// Pulls the freshest entry: list call (cached, shared with `deariary_recent`) gives the
/// newest date, then a per-date lookup (cached, shared with `deariary_on_this_day`) returns
/// the body with `content` populated — `/entries` list responses may strip content for size.
async fn latest_entry(token: &str) -> Result<Option<ApiEntry>, FetchError> {
    let entries = cached_get_entries(token, None).await?;
    let Some(latest) = entries.first() else {
        return Ok(None);
    };
    cached_get_entry(token, &latest.date).await
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

    #[test]
    fn text_body_uses_title_only() {
        let Body::Text(t) = render_body(Some(&entry()), Shape::Text, ymd(2026, 4, 27)) else {
            panic!("expected Text");
        };
        assert!(t.value.contains("My Day"));
        assert!(!t.value.contains("482"));
    }

    #[test]
    fn text_block_splits_content_into_lines() {
        let Body::TextBlock(t) = render_body(Some(&entry()), Shape::TextBlock, ymd(2026, 4, 27))
        else {
            panic!("expected TextBlock");
        };
        assert_eq!(t.lines, vec!["# Hello", "", "World"]);
    }

    #[test]
    fn linked_text_block_emits_one_clickable_headline() {
        let Body::LinkedTextBlock(l) =
            render_body(Some(&entry()), Shape::LinkedTextBlock, ymd(2026, 4, 27))
        else {
            panic!("expected LinkedTextBlock");
        };
        assert_eq!(l.items.len(), 1);
        assert!(l.items[0].text.contains("My Day"));
        assert_eq!(
            l.items[0].url.as_deref(),
            Some("https://app.deariary.com/entries/2026/04/27")
        );
    }

    #[test]
    fn markdown_block_passes_content_through_verbatim() {
        let Body::MarkdownTextBlock(m) =
            render_body(Some(&entry()), Shape::MarkdownTextBlock, ymd(2026, 4, 27))
        else {
            panic!("expected MarkdownTextBlock");
        };
        assert_eq!(m.value, "# Hello\n\nWorld");
    }

    #[test]
    fn entries_includes_only_present_metadata_keys() {
        let mut bare = entry();
        bare.tags.clear();
        bare.sources.clear();
        let Body::Entries(e) = render_body(Some(&bare), Shape::Entries, ymd(2026, 4, 27)) else {
            panic!("expected Entries");
        };
        let keys: Vec<&str> = e.items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, vec!["title", "date"]);
    }

    #[test]
    fn entries_joins_tags_and_sources_when_present() {
        let Body::Entries(e) = render_body(Some(&entry()), Shape::Entries, ymd(2026, 4, 27)) else {
            panic!("expected Entries");
        };
        let tags = e.items.iter().find(|i| i.key == "tags").unwrap();
        assert_eq!(tags.value.as_deref(), Some("work, travel"));
    }

    #[test]
    fn badge_label_says_today_when_entry_date_matches_today() {
        let Body::Badge(b) = render_body(Some(&entry()), Shape::Badge, ymd(2026, 4, 27)) else {
            panic!("expected Badge");
        };
        assert_eq!(b.status, Status::Ok);
        assert_eq!(b.label, "📔 today");
    }

    #[test]
    fn badge_label_says_yesterday_when_today_has_not_generated_yet() {
        let Body::Badge(b) = render_body(Some(&entry()), Shape::Badge, ymd(2026, 4, 28)) else {
            panic!("expected Badge");
        };
        assert_eq!(b.label, "📔 yesterday");
    }

    #[test]
    fn badge_label_falls_back_to_iso_date_for_older_entries() {
        let Body::Badge(b) = render_body(Some(&entry()), Shape::Badge, ymd(2026, 5, 1)) else {
            panic!("expected Badge");
        };
        assert_eq!(b.label, "📔 2026-04-27");
    }

    #[test]
    fn missing_entry_emits_empty_text_block() {
        let Body::TextBlock(t) = render_body(None, Shape::TextBlock, ymd(2026, 4, 27)) else {
            panic!("expected TextBlock");
        };
        assert!(t.lines.is_empty());
    }

    #[test]
    fn missing_entry_emits_warn_badge() {
        let Body::Badge(b) = render_body(None, Shape::Badge, ymd(2026, 4, 27)) else {
            panic!("expected Badge");
        };
        assert_eq!(b.status, Status::Warn);
        assert!(b.label.contains("no entry"));
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
}
