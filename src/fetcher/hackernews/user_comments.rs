//! `hackernews_user_comments` — recent comments by an HN account. Walks `submitted`, filters
//! to `type = "comment"`, and renders each one as `<snippet>` linked to its HN comment page.
//! Comment bodies are HTML — we strip tags and truncate to keep the splash readable.

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, LinkedLine, LinkedTextBlockData, Payload, TextBlockData};
use crate::render::Shape;
use crate::samples;

use super::client::{API_BASE, HN_ITEM_URL, get};

const DEFAULT_COUNT: u32 = 10;
const MIN_COUNT: u32 = 1;
const MAX_COUNT: u32 = 30;
const SCAN_LIMIT: usize = 80;
const SNIPPET_CHARS: usize = 100;

const SHAPES: &[Shape] = &[Shape::LinkedTextBlock, Shape::TextBlock];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "user",
        type_hint: "string (HN login)",
        required: true,
        default: None,
        description: "Hacker News login whose comments to list.",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Maximum number of comments to display.",
    },
];

pub struct HackernewsUserCommentsFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub count: Option<u32>,
}

#[async_trait]
impl Fetcher for HackernewsUserCommentsFetcher {
    fn name(&self) -> &str {
        "hackernews_user_comments"
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
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "Yes, that's exactly the problem with the current rate limiter ...",
                    Some("https://news.ycombinator.com/item?id=1"),
                ),
                (
                    "Have you tried running `cargo +nightly` to see if the issue ...",
                    Some("https://news.ycombinator.com/item?id=2"),
                ),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "Yes, that's exactly the problem with the current rate limiter ...",
                "Have you tried running `cargo +nightly` to see if the issue ...",
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let login = opts.user.ok_or_else(|| {
            FetchError::Failed("hackernews_user_comments requires `user` option".into())
        })?;
        let count = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_COUNT) as usize;
        let comments = fetch_comments(&login, count).await?;
        Ok(payload(render_body(
            &comments,
            ctx.shape.unwrap_or(Shape::LinkedTextBlock),
        )))
    }
}

#[derive(Debug, Deserialize)]
struct UserStub {
    #[serde(default)]
    submitted: Vec<u64>,
}

#[derive(Debug, Deserialize)]
struct Item {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default, rename = "type")]
    item_type: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

async fn fetch_comments(login: &str, want: usize) -> Result<Vec<Item>, FetchError> {
    let user: UserStub = get(&format!("{API_BASE}/user/{login}.json")).await?;
    let candidates: Vec<u64> = user.submitted.into_iter().take(SCAN_LIMIT).collect();
    let handles: Vec<_> = candidates
        .into_iter()
        .map(|id| tokio::spawn(async move { fetch_item(id).await }))
        .collect();
    let mut comments = Vec::with_capacity(want);
    for h in handles {
        if comments.len() >= want {
            break;
        }
        if let Ok(Ok(it)) = h.await
            && it.item_type.as_deref() == Some("comment")
        {
            comments.push(it);
        }
    }
    Ok(comments)
}

async fn fetch_item(id: u64) -> Result<Item, FetchError> {
    get(&format!("{API_BASE}/item/{id}.json")).await
}

fn render_body(items: &[Item], shape: Shape) -> Body {
    match shape {
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: items
                .iter()
                .map(|it| LinkedLine {
                    text: snippet(it),
                    url: it.id.map(|id| format!("{HN_ITEM_URL}{id}")),
                })
                .collect(),
        }),
        _ => Body::TextBlock(TextBlockData {
            lines: items.iter().map(snippet).collect(),
        }),
    }
}

fn snippet(it: &Item) -> String {
    let raw = it.text.as_deref().unwrap_or("");
    let stripped = strip_tags(raw);
    truncate(&stripped, SNIPPET_CHARS)
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut out: String = s.chars().take(max_chars).collect();
    if s.chars().count() > max_chars {
        out.push_str(" ...");
    }
    out
}

/// Mirrors the strip in `user.rs::strip_tags` — HN embeds raw HTML in its comment / about
/// fields, so we drop the markup and decode the common entities to keep the splash readable.
fn strip_tags(raw: &str) -> String {
    let with_breaks = raw.replace("<p>", " ").replace("<br>", " ");
    let mut out = String::with_capacity(with_breaks.len());
    let mut in_tag = false;
    for ch in with_breaks.chars() {
        match (ch, in_tag) {
            ('<', _) => in_tag = true,
            ('>', _) => in_tag = false,
            (c, false) => out.push(c),
            _ => {}
        }
    }
    decode_entities(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#x2F;", "/")
        .replace("&#39;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment(id: u64, text: &str) -> Item {
        Item {
            id: Some(id),
            item_type: Some("comment".into()),
            text: Some(text.into()),
        }
    }

    #[test]
    fn options_require_user() {
        let opts: Options = toml::from_str("user = \"pg\"\ncount = 4").unwrap();
        assert_eq!(opts.user.as_deref(), Some("pg"));
        assert_eq!(opts.count, Some(4));
    }

    #[test]
    fn snippet_strips_html_and_truncates() {
        let it = comment(1, "<p>Hello <i>there</i> friend &amp; greetings.</p>");
        assert_eq!(snippet(&it), "Hello there friend & greetings.");
    }

    #[test]
    fn truncate_appends_ellipsis_when_too_long() {
        let long = "x".repeat(200);
        let out = truncate(&long, 10);
        assert!(out.ends_with("..."));
        assert_eq!(out.chars().filter(|c| *c == 'x').count(), 10);
    }

    #[test]
    fn linked_text_block_links_to_hn_item_page() {
        let it = comment(42, "hello");
        let body = render_body(&[it], Shape::LinkedTextBlock);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked text block");
        };
        assert_eq!(
            b.items[0].url.as_deref(),
            Some("https://news.ycombinator.com/item?id=42")
        );
        assert_eq!(b.items[0].text, "hello");
    }

    #[test]
    fn text_block_shape_emits_snippet_per_comment() {
        let body = render_body(
            &[comment(1, "<p>first</p>"), comment(2, "<p>second</p>")],
            Shape::TextBlock,
        );
        let Body::TextBlock(t) = body else {
            panic!("expected text block");
        };
        assert_eq!(t.lines, vec!["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn missing_text_emits_empty_snippet() {
        let it = Item {
            id: Some(1),
            item_type: Some("comment".into()),
            text: None,
        };
        let body = render_body(&[it], Shape::LinkedTextBlock);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked text block");
        };
        assert_eq!(b.items[0].text, "");
    }

    #[test]
    fn missing_id_drops_url() {
        let it = Item {
            id: None,
            item_type: Some("comment".into()),
            text: Some("orphan".into()),
        };
        let body = render_body(&[it], Shape::LinkedTextBlock);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked text block");
        };
        assert!(b.items[0].url.is_none());
    }

    #[test]
    fn truncate_preserves_short_input() {
        assert_eq!(truncate("short", 100), "short");
        assert_eq!(truncate("", 10), "");
    }

    #[test]
    fn snippet_handles_nested_tags_and_entities() {
        let it = comment(1, "<p>this is <a href=\"/x\"><i>fine</i></a> &amp; ok</p>");
        assert_eq!(snippet(&it), "this is fine & ok");
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("user = \"pg\"\nbogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn empty_items_renders_empty_linked_text_block() {
        let body = render_body(&[], Shape::LinkedTextBlock);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked text block");
        };
        assert!(b.items.is_empty());
    }

    #[test]
    fn unaccepted_shape_falls_through_to_text_block() {
        // Entries isn't in SHAPES; render_body should default to the TextBlock branch.
        let body = render_body(&[comment(1, "hi")], Shape::Entries);
        assert!(matches!(body, Body::TextBlock(_)));
    }
}
