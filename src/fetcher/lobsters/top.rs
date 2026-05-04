//! `lobsters_top` — Lobsters story listings (hottest / newest / active), optionally filtered
//! by a single tag.
//!
//! Safety::Safe because the host is hardcoded (`lobste.rs`). Config picks the listing kind, the
//! row count, and an optional tag; the tag is normalized to a strict ASCII-alphanum-plus-`+_-`
//! subset before being interpolated into the path so no config can redirect traffic off-host.
//!
//! Lobsters' tag listing only exposes the hottest variant — `/t/<tag>/newest.json` and
//! `/t/<tag>/active.json` 404. When a `tag` is set, `kind` is silently forced to `hottest`.

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, parse_timestamp, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData,
    MarkdownTextBlockData, Payload, TextBlockData, TextData, TimelineData, TimelineEvent,
};
use crate::render::Shape;
use crate::samples;

use super::client::{SITE_BASE, get};

const DEFAULT_COUNT: u32 = 10;
const MIN_COUNT: u32 = 1;
const MAX_COUNT: u32 = 25;
const TAG_MAX_LEN: usize = 32;

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::TextBlock,
    Shape::Text,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::Timeline,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=25)",
        required: false,
        default: Some("10"),
        description: "Number of stories to display.",
    },
    OptionSchema {
        name: "kind",
        type_hint: "\"hottest\" | \"newest\" | \"active\"",
        required: false,
        default: Some("\"hottest\""),
        description: "Which Lobsters listing to query. Forced to `hottest` when `tag` is set (the tag listing only exposes the hottest variant).",
    },
    OptionSchema {
        name: "tag",
        type_hint: "string",
        required: false,
        default: None,
        description: "Filter to a single Lobsters tag (e.g. `\"rust\"`, `\"c++\"`). Allowed chars: `[a-z0-9_+-]`.",
    },
];

pub struct LobstersTopFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub kind: Option<Kind>,
    #[serde(default)]
    pub tag: Option<String>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Hottest,
    Newest,
    Active,
}

impl Kind {
    fn endpoint(self) -> &'static str {
        match self {
            Self::Hottest => "hottest",
            Self::Newest => "newest",
            Self::Active => "active",
        }
    }
}

#[async_trait]
impl Fetcher for LobstersTopFetcher {
    fn name(&self) -> &str {
        "lobsters_top"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Lobsters front-page listings — `hottest` / `newest` / `active`, optionally filtered by a single tag. Each row shows score, comment count, and title, linked to the story URL (or the Lobsters comment page when there isn't one)."
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
            Shape::Text => samples::text("75pt 17c  Show: terminal dashboard with local trust"),
            Shape::TextBlock => samples::text_block(&[
                "75pt 17c  Show: terminal dashboard with local trust",
                "41pt 6c   Fake Notepad++ for Mac",
                "29pt 2c   This Wasm interpreter fits in a QR code",
            ]),
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "75pt 17c  Show: terminal dashboard with local trust",
                    Some("https://example.com/show"),
                ),
                (
                    "41pt 6c   Fake Notepad++ for Mac",
                    Some("https://example.com/np"),
                ),
                (
                    "29pt 2c   This Wasm interpreter fits in a QR code",
                    Some("https://lobste.rs/s/abc123"),
                ),
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "- **75pt 17c**  [Show: terminal dashboard with local trust](https://example.com/show)\n\
                 - **41pt 6c**   [Fake Notepad++ for Mac](https://example.com/np)\n\
                 - **29pt 2c**   [This Wasm interpreter fits in a QR code](https://lobste.rs/s/abc123)",
            ),
            Shape::Entries => samples::entries(&[
                ("Show: terminal dashboard with local trust", "75pt 17c"),
                ("Fake Notepad++ for Mac", "41pt 6c"),
                ("This Wasm interpreter fits in a QR code", "29pt 2c"),
            ]),
            Shape::Bars => Body::Bars(BarsData {
                bars: vec![
                    Bar {
                        label: "Show: terminal dashboard …".into(),
                        value: 75,
                    },
                    Bar {
                        label: "Fake Notepad++ for Mac".into(),
                        value: 41,
                    },
                    Bar {
                        label: "This Wasm interpreter fits …".into(),
                        value: 29,
                    },
                ],
            }),
            Shape::Timeline => samples::timeline(&[
                (
                    1_777_708_434,
                    "Show: terminal dashboard with local trust",
                    Some("75pt 17c"),
                ),
                (1_777_705_489, "Fake Notepad++ for Mac", Some("41pt 6c")),
                (
                    1_777_705_205,
                    "This Wasm interpreter fits in a QR code",
                    Some("29pt 2c"),
                ),
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
        let tag = opts.tag.as_deref().map(normalize_tag).transpose()?;
        let path = listing_path(kind, tag.as_deref());
        let items = fetch_stories(&path, count).await?;
        Ok(payload(render_body(
            &items,
            ctx.shape.unwrap_or(Shape::LinkedTextBlock),
        )))
    }
}

fn listing_path(kind: Kind, tag: Option<&str>) -> String {
    match tag {
        // Tag listing only exposes hottest — silently force-route there.
        Some(t) => format!("/t/{t}.json"),
        None => format!("/{}.json", kind.endpoint()),
    }
}

fn normalize_tag(raw: &str) -> Result<String, FetchError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(FetchError::Failed("tag must not be empty".into()));
    }
    if trimmed.len() > TAG_MAX_LEN {
        return Err(FetchError::Failed("tag is too long".into()));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-'))
    {
        return Err(FetchError::Failed(
            "tag must contain only [a-z0-9_+-]".into(),
        ));
    }
    Ok(trimmed.to_ascii_lowercase())
}

async fn fetch_stories(path: &str, count: usize) -> Result<Vec<Item>, FetchError> {
    let items: Vec<Item> = get(&format!("{SITE_BASE}{path}")).await?;
    Ok(items.into_iter().take(count).collect())
}

#[derive(Debug, Deserialize)]
struct Item {
    #[serde(default)]
    short_id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    score: Option<i64>,
    #[serde(default)]
    comment_count: Option<u64>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    comments_url: Option<String>,
    #[serde(default)]
    short_id_url: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

fn link_for(it: &Item) -> Option<String> {
    it.url
        .clone()
        .filter(|u| !u.is_empty())
        .or_else(|| it.comments_url.clone())
        .or_else(|| it.short_id_url.clone())
        .or_else(|| {
            it.short_id
                .as_deref()
                .map(|id| format!("{SITE_BASE}/s/{id}"))
        })
}

fn render_body(items: &[Item], shape: Shape) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: items
                .first()
                .map(|it| format!("{}  {}", meta_label(it), title_or_placeholder(it)))
                .unwrap_or_default(),
        }),
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
        Shape::Bars => Body::Bars(BarsData {
            bars: items
                .iter()
                .map(|it| Bar {
                    label: title_or_placeholder(it),
                    value: it.score.unwrap_or(0).max(0) as u64,
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
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: items
                .iter()
                .map(markdown_row)
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Timeline => Body::Timeline(TimelineData {
            events: items
                .iter()
                .map(|it| TimelineEvent {
                    timestamp: it.created_at.as_deref().map(parse_timestamp).unwrap_or(0),
                    title: title_or_placeholder(it),
                    detail: Some(meta_label(it)),
                    status: None,
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
    it.title
        .as_deref()
        .filter(|t| !t.is_empty())
        .unwrap_or("(no title)")
        .to_string()
}

fn meta_label(it: &Item) -> String {
    let score = it.score.unwrap_or(0);
    let comments = it.comment_count.unwrap_or(0);
    format!("{score}pt {comments}c")
}

fn markdown_row(it: &Item) -> String {
    let title = title_or_placeholder(it);
    let meta = meta_label(it);
    match link_for(it) {
        Some(url) => format!("- **{meta}**  [{title}]({url})"),
        None => format!("- **{meta}**  {title}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetcher::FetchContext;

    fn item(title: Option<&str>, score: Option<i64>, comments: Option<u64>) -> Item {
        Item {
            short_id: Some("abc123".into()),
            title: title.map(String::from),
            score,
            comment_count: comments,
            url: None,
            comments_url: None,
            short_id_url: None,
            created_at: None,
        }
    }

    fn ctx(shape: Shape, options: &str) -> FetchContext {
        FetchContext {
            shape: Some(shape),
            options: Some(toml::from_str(options).unwrap()),
            ..FetchContext::default()
        }
    }

    #[test]
    fn options_default_to_none_for_each_field() {
        let opts = Options::default();
        assert!(opts.count.is_none());
        assert!(opts.kind.is_none());
        assert!(opts.tag.is_none());
    }

    #[test]
    fn options_deserialize_full_set() {
        let raw: toml::Value =
            toml::from_str("count = 5\nkind = \"newest\"\ntag = \"rust\"").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.count, Some(5));
        assert_eq!(opts.kind, Some(Kind::Newest));
        assert_eq!(opts.tag.as_deref(), Some("rust"));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("count = 3\nbogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn kind_endpoint_covers_every_variant() {
        assert_eq!(Kind::Hottest.endpoint(), "hottest");
        assert_eq!(Kind::Newest.endpoint(), "newest");
        assert_eq!(Kind::Active.endpoint(), "active");
    }

    #[test]
    fn fetcher_catalog_surface_matches_contract() {
        let f = LobstersTopFetcher;
        assert_eq!(f.name(), "lobsters_top");
        assert_eq!(f.safety(), Safety::Safe);
        assert_eq!(f.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(f.shapes(), SHAPES);
        assert_eq!(f.option_schemas().len(), 3);
        assert_eq!(f.option_schemas()[0].name, "count");
        assert_eq!(f.option_schemas()[1].name, "kind");
        assert_eq!(f.option_schemas()[2].name, "tag");
        assert!(f.description().contains("Lobsters"));
    }

    #[test]
    fn sample_body_supports_each_declared_shape() {
        let f = LobstersTopFetcher;
        for shape in SHAPES {
            assert!(
                f.sample_body(*shape).is_some(),
                "missing sample for {shape:?}",
            );
        }
        assert!(f.sample_body(Shape::Heatmap).is_none());
    }

    #[test]
    fn cache_key_changes_with_shape_and_options() {
        let f = LobstersTopFetcher;
        let base = f.cache_key(&ctx(
            Shape::LinkedTextBlock,
            "count = 5\nkind = \"hottest\"",
        ));
        let same = f.cache_key(&ctx(
            Shape::LinkedTextBlock,
            "count = 5\nkind = \"hottest\"",
        ));
        let other_shape = f.cache_key(&ctx(Shape::Entries, "count = 5\nkind = \"hottest\""));
        let other_kind = f.cache_key(&ctx(Shape::LinkedTextBlock, "count = 5\nkind = \"newest\""));
        let with_tag = f.cache_key(&ctx(
            Shape::LinkedTextBlock,
            "count = 5\nkind = \"hottest\"\ntag = \"rust\"",
        ));
        assert_eq!(base, same);
        assert_ne!(base, other_shape);
        assert_ne!(base, other_kind);
        assert_ne!(base, with_tag);
    }

    #[test]
    fn listing_path_uses_kind_endpoint_when_no_tag() {
        assert_eq!(listing_path(Kind::Hottest, None), "/hottest.json");
        assert_eq!(listing_path(Kind::Newest, None), "/newest.json");
        assert_eq!(listing_path(Kind::Active, None), "/active.json");
    }

    #[test]
    fn listing_path_routes_through_tag_when_set_ignoring_kind() {
        // Tag listing only exposes hottest — `kind` is ignored when `tag` is set.
        assert_eq!(listing_path(Kind::Hottest, Some("rust")), "/t/rust.json");
        assert_eq!(listing_path(Kind::Newest, Some("rust")), "/t/rust.json");
        assert_eq!(listing_path(Kind::Active, Some("c++")), "/t/c++.json");
    }

    #[test]
    fn normalize_tag_lowercases_and_trims() {
        assert_eq!(normalize_tag("  Rust  ").unwrap(), "rust");
        assert_eq!(normalize_tag("C++").unwrap(), "c++");
        assert_eq!(normalize_tag("a11y").unwrap(), "a11y");
    }

    #[test]
    fn normalize_tag_rejects_empty() {
        assert!(matches!(normalize_tag(""), Err(FetchError::Failed(_))));
        assert!(matches!(normalize_tag("   "), Err(FetchError::Failed(_))));
    }

    #[test]
    fn normalize_tag_rejects_overlength() {
        let long = "a".repeat(TAG_MAX_LEN + 1);
        let err = normalize_tag(&long).unwrap_err();
        assert!(matches!(err, FetchError::Failed(msg) if msg.contains("too long")));
    }

    #[test]
    fn normalize_tag_rejects_disallowed_chars() {
        for raw in ["rust!", "rust/system", "rust space", "rust?", "rust#"] {
            assert!(
                matches!(normalize_tag(raw), Err(FetchError::Failed(_))),
                "expected rejection for {raw:?}",
            );
        }
    }

    #[test]
    fn text_uses_first_item_headline() {
        let body = render_body(&[item(Some("hello"), Some(123), Some(45))], Shape::Text);
        assert_eq!(
            body,
            Body::Text(TextData {
                value: "123pt 45c  hello".into(),
            })
        );
    }

    #[test]
    fn text_block_line_includes_score_comments_and_title() {
        let body = render_body(
            &[item(Some("hello"), Some(123), Some(45))],
            Shape::TextBlock,
        );
        assert_eq!(
            body,
            Body::TextBlock(TextBlockData {
                lines: vec!["123pt 45c  hello".into()],
            })
        );
    }

    #[test]
    fn entries_use_title_as_key_and_meta_as_value() {
        let body = render_body(&[item(Some("hello"), Some(7), None)], Shape::Entries);
        assert_eq!(
            body,
            Body::Entries(EntriesData {
                items: vec![Entry {
                    key: "hello".into(),
                    value: Some("7pt 0c".into()),
                    status: None,
                }],
            })
        );
    }

    #[test]
    fn bars_clamp_negative_score_to_zero() {
        let body = render_body(&[item(Some("flagged"), Some(-3), Some(0))], Shape::Bars);
        assert_eq!(
            body,
            Body::Bars(BarsData {
                bars: vec![Bar {
                    label: "flagged".into(),
                    value: 0,
                }],
            })
        );
    }

    #[test]
    fn linked_text_block_prefers_story_url_when_present() {
        let it = Item {
            short_id: Some("abc".into()),
            title: Some("show".into()),
            score: Some(1),
            comment_count: Some(0),
            url: Some("https://example.com/post".into()),
            comments_url: Some("https://lobste.rs/s/abc/show".into()),
            short_id_url: Some("https://lobste.rs/s/abc".into()),
            created_at: None,
        };
        let body = render_body(&[it], Shape::LinkedTextBlock);
        assert_eq!(
            body,
            Body::LinkedTextBlock(LinkedTextBlockData {
                items: vec![LinkedLine {
                    text: "1pt 0c  show".into(),
                    url: Some("https://example.com/post".into()),
                }],
            })
        );
    }

    #[test]
    fn linked_text_block_url_falls_back_to_comments_when_story_url_empty() {
        // Lobsters Ask-style posts have an empty `url` and only a `comments_url`.
        let it = Item {
            short_id: Some("abc".into()),
            title: Some("ask".into()),
            score: Some(81),
            comment_count: Some(50),
            url: Some(String::new()),
            comments_url: Some("https://lobste.rs/s/abc/ask".into()),
            short_id_url: Some("https://lobste.rs/s/abc".into()),
            created_at: None,
        };
        let body = render_body(&[it], Shape::LinkedTextBlock);
        assert_eq!(
            body,
            Body::LinkedTextBlock(LinkedTextBlockData {
                items: vec![LinkedLine {
                    text: "81pt 50c  ask".into(),
                    url: Some("https://lobste.rs/s/abc/ask".into()),
                }],
            })
        );
    }

    #[test]
    fn linked_text_block_drops_url_when_every_link_field_is_missing() {
        let it = Item {
            short_id: None,
            title: Some("orphan".into()),
            score: Some(1),
            comment_count: Some(0),
            url: None,
            comments_url: None,
            short_id_url: None,
            created_at: None,
        };
        let body = render_body(&[it], Shape::LinkedTextBlock);
        assert_eq!(
            body,
            Body::LinkedTextBlock(LinkedTextBlockData {
                items: vec![LinkedLine {
                    text: "1pt 0c  orphan".into(),
                    url: None,
                }],
            })
        );
    }

    #[test]
    fn markdown_row_includes_meta_link_and_title() {
        let it = Item {
            short_id: Some("abc".into()),
            title: Some("hello".into()),
            score: Some(7),
            comment_count: Some(2),
            url: Some("https://example.com/h".into()),
            comments_url: None,
            short_id_url: None,
            created_at: None,
        };
        assert_eq!(
            markdown_row(&it),
            "- **7pt 2c**  [hello](https://example.com/h)",
        );
    }

    #[test]
    fn markdown_row_omits_link_syntax_when_no_url_resolvable() {
        let it = Item {
            short_id: None,
            title: Some("hello".into()),
            score: Some(0),
            comment_count: Some(0),
            url: None,
            comments_url: None,
            short_id_url: None,
            created_at: None,
        };
        assert_eq!(markdown_row(&it), "- **0pt 0c**  hello");
    }

    #[test]
    fn timeline_uses_parsed_created_at_seconds() {
        let it = Item {
            short_id: Some("abc".into()),
            title: Some("hello".into()),
            score: Some(1),
            comment_count: Some(2),
            url: None,
            comments_url: None,
            short_id_url: None,
            created_at: Some("2026-04-22T10:15:30Z".into()),
        };
        let Body::Timeline(TimelineData { events }) = render_body(&[it], Shape::Timeline) else {
            panic!("expected Timeline body");
        };
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].timestamp, 1_776_852_930);
        assert_eq!(events[0].title, "hello");
        assert_eq!(events[0].detail.as_deref(), Some("1pt 2c"));
    }

    #[test]
    fn timeline_falls_back_to_zero_when_created_at_missing_or_garbage() {
        let it = item(Some("x"), Some(0), Some(0));
        let Body::Timeline(TimelineData { events }) = render_body(&[it], Shape::Timeline) else {
            panic!("expected Timeline body");
        };
        assert_eq!(events[0].timestamp, 0);
    }

    #[test]
    fn missing_title_falls_back_to_placeholder() {
        let body = render_body(&[item(None, Some(0), Some(0))], Shape::TextBlock);
        assert_eq!(
            body,
            Body::TextBlock(TextBlockData {
                lines: vec!["0pt 0c  (no title)".into()],
            })
        );
    }

    #[test]
    fn empty_items_renders_empty_bodies_for_each_shape() {
        for shape in SHAPES {
            let body = render_body(&[], *shape);
            match body {
                Body::Text(t) => assert!(t.value.is_empty()),
                Body::TextBlock(t) => assert!(t.lines.is_empty()),
                Body::LinkedTextBlock(t) => assert!(t.items.is_empty()),
                Body::MarkdownTextBlock(t) => assert!(t.value.is_empty()),
                Body::Entries(t) => assert!(t.items.is_empty()),
                Body::Bars(t) => assert!(t.bars.is_empty()),
                Body::Timeline(t) => assert!(t.events.is_empty()),
                other => panic!("unexpected body for {shape:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn item_deserializes_from_lobsters_payload() {
        let raw = r#"{"short_id":"abc","created_at":"2026-04-22T10:15:30Z","title":"hi","score":99,"flags":0,"comment_count":12,"url":"https://example.com","short_id_url":"https://lobste.rs/s/abc","comments_url":"https://lobste.rs/s/abc/hi"}"#;
        let it: Item = serde_json::from_str(raw).unwrap();
        assert_eq!(it.title.as_deref(), Some("hi"));
        assert_eq!(it.score, Some(99));
        assert_eq!(it.comment_count, Some(12));
        assert_eq!(it.url.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn item_deserializes_when_optional_fields_are_missing() {
        let raw = r#"{"short_id":"abc"}"#;
        let it: Item = serde_json::from_str(raw).unwrap();
        assert!(it.title.is_none());
        assert!(it.score.is_none());
        assert!(it.comment_count.is_none());
    }

    #[test]
    fn count_clamps_extremes() {
        assert_eq!(0u32.clamp(MIN_COUNT, MAX_COUNT), MIN_COUNT);
        assert_eq!(999u32.clamp(MIN_COUNT, MAX_COUNT), MAX_COUNT);
        assert_eq!(DEFAULT_COUNT.clamp(MIN_COUNT, MAX_COUNT), DEFAULT_COUNT);
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_options() {
        let f = LobstersTopFetcher;
        let err = f
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
        let f = LobstersTopFetcher;
        let err = f
            .fetch(&ctx(Shape::LinkedTextBlock, "kind = \"frontpage\""))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(msg) if msg.contains("unknown variant `frontpage`")
        ));
    }

    #[tokio::test]
    async fn fetch_rejects_disallowed_tag_chars() {
        let f = LobstersTopFetcher;
        let err = f
            .fetch(&ctx(Shape::LinkedTextBlock, "tag = \"rust system\""))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(msg) if msg.contains("[a-z0-9_+-]")
        ));
    }

    /// Live smoke test — hits Lobsters. `#[ignore]` keeps CI offline-safe; run with
    /// `cargo test -- --ignored fetcher::lobsters::top::tests::live` to verify real shape.
    #[tokio::test]
    #[ignore]
    async fn live_hottest_returns_at_least_one_story() {
        let items = fetch_stories("/hottest.json", 3).await.unwrap();
        assert!(!items.is_empty());
        for it in &items {
            eprintln!(
                "{}pt {}c  {}",
                it.score.unwrap_or(0),
                it.comment_count.unwrap_or(0),
                it.title.as_deref().unwrap_or(""),
            );
        }
    }
}
