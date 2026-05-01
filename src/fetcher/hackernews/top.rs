//! `hackernews_top` — Hacker News stories (top / new / best / ask / show / job).
//!
//! Safety::Safe because the host is hardcoded (HN's read-only Firebase backend). Config picks
//! the listing kind and how many stories to show, never the URL — there's no way for a config
//! to redirect traffic off-host. No auth, no token leaves the machine.

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData, Payload, TextBlockData,
};
use crate::render::Shape;
use crate::samples;

use super::client::{API_BASE, HN_ITEM_URL, get};

const DEFAULT_COUNT: u32 = 10;
const MIN_COUNT: u32 = 1;
const MAX_COUNT: u32 = 30;

const SHAPES: &[Shape] = &[Shape::LinkedTextBlock, Shape::TextBlock, Shape::Entries];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Number of stories to display.",
    },
    OptionSchema {
        name: "kind",
        type_hint: "\"top\" | \"new\" | \"best\" | \"ask\" | \"show\" | \"job\"",
        required: false,
        default: Some("\"top\""),
        description: "Which Hacker News listing to query.",
    },
];

pub struct HackernewsTopFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub kind: Option<Kind>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Top,
    New,
    Best,
    Ask,
    Show,
    Job,
}

impl Kind {
    fn endpoint(self) -> &'static str {
        match self {
            Self::Top => "topstories",
            Self::New => "newstories",
            Self::Best => "beststories",
            Self::Ask => "askstories",
            Self::Show => "showstories",
            Self::Job => "jobstories",
        }
    }
}

#[async_trait]
impl Fetcher for HackernewsTopFetcher {
    fn name(&self) -> &str {
        "hackernews_top"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Hacker News front-page listings — `top` / `new` / `best` / `ask` / `show` / `job`. Each row shows score, comment count, and title, linked to the story URL (or the HN comment page when there isn't one)."
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
            Shape::TextBlock => samples::text_block(&[
                "234pt 56c  Show HN: I built a thing",
                "187pt 41c  Why X over Y",
                "152pt 88c  Ask HN: how do you ...",
            ]),
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "234pt 56c  Show HN: I built a thing",
                    Some("https://example.com/show-hn"),
                ),
                (
                    "187pt 41c  Why X over Y",
                    Some("https://example.com/article"),
                ),
                (
                    "152pt 88c  Ask HN: how do you ...",
                    Some("https://news.ycombinator.com/item?id=1"),
                ),
            ]),
            Shape::Entries => samples::entries(&[
                ("Show HN: I built a thing", "234pt 56c"),
                ("Why X over Y", "187pt 41c"),
                ("Ask HN: how do you ...", "152pt 88c"),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let count = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_COUNT) as usize;
        let kind = opts.kind.unwrap_or_default();
        let items = fetch_stories(kind, count).await?;
        Ok(payload(render_body(
            &items,
            ctx.shape.unwrap_or(Shape::LinkedTextBlock),
        )))
    }
}

async fn fetch_stories(kind: Kind, count: usize) -> Result<Vec<Item>, FetchError> {
    let ids: Vec<u64> = get(&format!("{API_BASE}/{}.json", kind.endpoint())).await?;
    let handles: Vec<_> = ids
        .into_iter()
        .take(count)
        .map(|id| tokio::spawn(async move { fetch_item(id).await }))
        .collect();
    let mut items = Vec::with_capacity(handles.len());
    for h in handles {
        if let Ok(Ok(it)) = h.await {
            items.push(it);
        }
    }
    Ok(items)
}

async fn fetch_item(id: u64) -> Result<Item, FetchError> {
    get(&format!("{API_BASE}/item/{id}.json")).await
}

#[derive(Debug, Deserialize)]
struct Item {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    score: Option<u64>,
    #[serde(default)]
    descendants: Option<u64>,
    #[serde(default)]
    url: Option<String>,
}

fn link_for(it: &Item) -> Option<String> {
    it.url
        .clone()
        .or_else(|| it.id.map(|id| format!("{HN_ITEM_URL}{id}")))
}

fn render_body(items: &[Item], shape: Shape) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: items
                .iter()
                .map(|it| Entry {
                    key: title_or_placeholder(it),
                    value: Some(meta_label(it)),
                    status: None,
                })
                .collect(),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: items
                .iter()
                .map(|it| LinkedLine {
                    text: format!("{}  {}", meta_label(it), title_or_placeholder(it)),
                    url: link_for(it),
                })
                .collect(),
        }),
        _ => Body::TextBlock(TextBlockData {
            lines: items
                .iter()
                .map(|it| format!("{}  {}", meta_label(it), title_or_placeholder(it)))
                .collect(),
        }),
    }
}

fn title_or_placeholder(it: &Item) -> String {
    it.title.clone().unwrap_or_else(|| "(no title)".into())
}

fn meta_label(it: &Item) -> String {
    let score = it.score.unwrap_or(0);
    let comments = it.descendants.unwrap_or(0);
    format!("{score}pt {comments}c")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetcher::FetchContext;

    #[test]
    fn options_default_to_none_for_both_fields() {
        let opts = Options::default();
        assert!(opts.count.is_none());
        assert!(opts.kind.is_none());
    }

    #[test]
    fn options_deserialize_count_and_kind() {
        let raw: toml::Value = toml::from_str("count = 5\nkind = \"new\"").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.count, Some(5));
        assert!(matches!(opts.kind, Some(Kind::New)));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("count = 3\nbogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn kind_endpoint_covers_every_variant() {
        assert_eq!(Kind::Top.endpoint(), "topstories");
        assert_eq!(Kind::New.endpoint(), "newstories");
        assert_eq!(Kind::Best.endpoint(), "beststories");
        assert_eq!(Kind::Ask.endpoint(), "askstories");
        assert_eq!(Kind::Show.endpoint(), "showstories");
        assert_eq!(Kind::Job.endpoint(), "jobstories");
    }

    fn ctx(shape: Shape, options: &str) -> FetchContext {
        FetchContext {
            shape: Some(shape),
            options: Some(toml::from_str(options).unwrap()),
            ..FetchContext::default()
        }
    }

    #[test]
    fn fetcher_catalog_surface_matches_contract() {
        let fetcher = HackernewsTopFetcher;
        assert_eq!(fetcher.name(), "hackernews_top");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.option_schemas().len(), 2);
        assert_eq!(fetcher.option_schemas()[0].name, "count");
        assert_eq!(fetcher.option_schemas()[1].name, "kind");
        assert!(fetcher.description().contains("front-page listings"));
    }

    #[test]
    fn sample_body_supports_each_declared_shape() {
        let fetcher = HackernewsTopFetcher;

        let block = fetcher.sample_body(Shape::TextBlock).unwrap();
        assert!(matches!(block, Body::TextBlock(_)));
        if let Body::TextBlock(block) = block {
            assert_eq!(block.lines.len(), 3);
            assert!(block.lines[0].contains("Show HN"));
        }

        let linked = fetcher.sample_body(Shape::LinkedTextBlock).unwrap();
        assert!(matches!(linked, Body::LinkedTextBlock(_)));
        if let Body::LinkedTextBlock(linked) = linked {
            assert_eq!(linked.items.len(), 3);
            assert!(linked.items[0].url.is_some());
        }

        let entries = fetcher.sample_body(Shape::Entries).unwrap();
        assert!(matches!(entries, Body::Entries(_)));
        if let Body::Entries(entries) = entries {
            assert_eq!(entries.items[0].key, "Show HN: I built a thing");
            assert_eq!(entries.items[0].value.as_deref(), Some("234pt 56c"));
        }

        assert!(fetcher.sample_body(Shape::Timeline).is_none());
    }

    #[test]
    fn cache_key_changes_with_shape_and_options() {
        let fetcher = HackernewsTopFetcher;
        let base = fetcher.cache_key(&ctx(Shape::LinkedTextBlock, "count = 5\nkind = \"top\""));
        let same = fetcher.cache_key(&ctx(Shape::LinkedTextBlock, "count = 5\nkind = \"top\""));
        let different_shape = fetcher.cache_key(&ctx(Shape::Entries, "count = 5\nkind = \"top\""));
        let different_options =
            fetcher.cache_key(&ctx(Shape::LinkedTextBlock, "count = 6\nkind = \"show\""));

        assert_eq!(base, same);
        assert_ne!(base, different_shape);
        assert_ne!(base, different_options);
    }

    fn item(title: Option<&str>, score: Option<u64>, descendants: Option<u64>) -> Item {
        Item {
            id: Some(1),
            title: title.map(String::from),
            score,
            descendants,
            url: None,
        }
    }

    #[test]
    fn text_block_line_includes_score_comments_and_title() {
        let body = render_body(
            &[item(Some("hello"), Some(123), Some(45))],
            Shape::TextBlock,
        );
        assert!(matches!(body, Body::TextBlock(_)));
        if let Body::TextBlock(text_block) = body {
            assert_eq!(text_block.lines, vec!["123pt 45c  hello".to_string()]);
        }
    }

    #[test]
    fn entries_use_title_as_key_and_meta_as_value() {
        let body = render_body(&[item(Some("hello"), Some(7), None)], Shape::Entries);
        assert!(matches!(body, Body::Entries(_)));
        if let Body::Entries(entries) = body {
            assert_eq!(entries.items[0].key, "hello");
            assert_eq!(entries.items[0].value.as_deref(), Some("7pt 0c"));
        }
    }

    #[test]
    fn empty_items_renders_empty_entries() {
        let body = render_body(&[], Shape::Entries);
        assert!(matches!(body, Body::Entries(_)));
        if let Body::Entries(entries) = body {
            assert!(entries.items.is_empty());
        }
    }

    #[test]
    fn linked_text_block_url_falls_back_to_hn_item_page_when_no_story_url() {
        let body = render_body(
            &[item(Some("ask"), Some(0), Some(0))],
            Shape::LinkedTextBlock,
        );
        assert!(matches!(body, Body::LinkedTextBlock(_)));
        if let Body::LinkedTextBlock(linked) = body {
            assert_eq!(
                linked.items[0].url.as_deref(),
                Some("https://news.ycombinator.com/item?id=1")
            );
        }
    }

    #[test]
    fn linked_text_block_url_prefers_story_url_when_present() {
        let it = Item {
            id: Some(42),
            title: Some("show".into()),
            score: Some(1),
            descendants: Some(0),
            url: Some("https://example.com/post".into()),
        };
        let body = render_body(&[it], Shape::LinkedTextBlock);
        assert!(matches!(body, Body::LinkedTextBlock(_)));
        if let Body::LinkedTextBlock(linked) = body {
            assert_eq!(
                linked.items[0].url.as_deref(),
                Some("https://example.com/post")
            );
            assert!(linked.items[0].text.contains("show"));
            assert!(linked.items[0].text.contains("1pt 0c"));
        }
    }

    #[test]
    fn missing_title_falls_back_to_placeholder() {
        let body = render_body(&[item(None, Some(0), Some(0))], Shape::TextBlock);
        assert!(matches!(body, Body::TextBlock(_)));
        if let Body::TextBlock(text_block) = body {
            assert!(text_block.lines[0].contains("(no title)"));
        }
    }

    #[test]
    fn item_deserializes_from_hn_payload() {
        let raw = r#"{"by":"x","descendants":12,"id":1,"score":99,"time":1700000000,"title":"hi","type":"story","url":"https://example.com"}"#;
        let it: Item = serde_json::from_str(raw).unwrap();
        assert_eq!(it.title.as_deref(), Some("hi"));
        assert_eq!(it.score, Some(99));
        assert_eq!(it.descendants, Some(12));
    }

    #[test]
    fn item_deserializes_when_optional_fields_are_missing() {
        let raw = r#"{"id":1,"type":"job"}"#;
        let it: Item = serde_json::from_str(raw).unwrap();
        assert!(it.title.is_none());
        assert!(it.score.is_none());
        assert!(it.descendants.is_none());
    }

    #[test]
    fn empty_items_renders_empty_body() {
        let body = render_body(&[], Shape::TextBlock);
        assert!(matches!(body, Body::TextBlock(_)));
        if let Body::TextBlock(text_block) = body {
            assert!(text_block.lines.is_empty());
        }
    }

    #[test]
    fn empty_items_renders_empty_linked_text_block() {
        let body = render_body(&[], Shape::LinkedTextBlock);
        assert!(matches!(body, Body::LinkedTextBlock(_)));
        if let Body::LinkedTextBlock(linked) = body {
            assert!(linked.items.is_empty());
        }
    }

    #[test]
    fn unsupported_shape_falls_back_to_text_block() {
        let body = render_body(&[item(Some("fallback"), Some(8), Some(3))], Shape::Text);
        assert!(matches!(body, Body::TextBlock(_)));
        if let Body::TextBlock(text_block) = body {
            assert_eq!(text_block.lines, vec!["8pt 3c  fallback".to_string()]);
        }
    }

    #[test]
    fn linked_text_block_drops_url_when_id_and_story_url_both_missing() {
        let it = Item {
            id: None,
            title: Some("orphan".into()),
            score: Some(1),
            descendants: Some(0),
            url: None,
        };
        let body = render_body(&[it], Shape::LinkedTextBlock);
        assert!(matches!(body, Body::LinkedTextBlock(_)));
        if let Body::LinkedTextBlock(linked) = body {
            assert!(linked.items[0].url.is_none());
        }
    }

    #[test]
    fn missing_score_and_comments_default_to_zero() {
        let body = render_body(&[item(Some("x"), None, None)], Shape::TextBlock);
        assert!(matches!(body, Body::TextBlock(_)));
        if let Body::TextBlock(text_block) = body {
            assert_eq!(text_block.lines[0], "0pt 0c  x");
        }
    }

    #[test]
    fn kind_default_is_top_in_options_and_endpoint() {
        let opts = Options::default();
        assert!(opts.kind.is_none());
        assert_eq!(Kind::default().endpoint(), "topstories");
    }

    #[test]
    fn renders_each_kind_via_options() {
        for raw in [
            "kind = \"top\"",
            "kind = \"new\"",
            "kind = \"best\"",
            "kind = \"ask\"",
            "kind = \"show\"",
            "kind = \"job\"",
        ] {
            let v: toml::Value = toml::from_str(raw).unwrap();
            let _: Options = v.try_into().expect("kind variant should parse");
        }
    }

    #[test]
    fn count_clamps_extremes() {
        // Fetch path uses `.clamp(MIN_COUNT, MAX_COUNT)` directly; verifying clamp semantics
        // here means a future tweak to either bound stays consistent.
        assert_eq!(0u32.clamp(MIN_COUNT, MAX_COUNT), MIN_COUNT);
        assert_eq!(999u32.clamp(MIN_COUNT, MAX_COUNT), MAX_COUNT);
        assert_eq!(DEFAULT_COUNT.clamp(MIN_COUNT, MAX_COUNT), DEFAULT_COUNT);
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_options() {
        let fetcher = HackernewsTopFetcher;
        let err = fetcher
            .fetch(&ctx(Shape::LinkedTextBlock, "count = 3\nbogus = true"))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(msg) if msg.contains("unknown field `bogus`")
        ));
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_kind_value() {
        let fetcher = HackernewsTopFetcher;
        let err = fetcher
            .fetch(&ctx(Shape::LinkedTextBlock, "kind = \"frontpage\""))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(msg) if msg.contains("unknown variant `frontpage`")
        ));
    }

    /// Live smoke test — hits HN's Firebase API. `#[ignore]` keeps CI offline-safe; run with
    /// `cargo test -- --ignored fetcher::hackernews_top::tests::live` to verify real shape.
    #[tokio::test]
    #[ignore]
    async fn live_top_returns_at_least_one_story() {
        let items = fetch_stories(Kind::Top, 3).await.unwrap();
        assert!(!items.is_empty());
        for it in &items {
            eprintln!(
                "{}pt {}c  {}",
                it.score.unwrap_or(0),
                it.descendants.unwrap_or(0),
                it.title.as_deref().unwrap_or(""),
            );
        }
    }
}
