//! `reddit_user_comments` — recent comments for a Reddit user.

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::github::common::{parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData, Payload, TextBlockData,
};
use crate::render::Shape;
use crate::samples;

use super::client::{SITE_BASE, fetch_listing};
use super::common::{
    SHAPES, cache_key_for, network_unavailable_body, normalize_user, normalized_count,
};

const DEFAULT_USER: &str = "spez";
const DEFAULT_COMMENT_CHARS: usize = 110;
const MIN_COMMENT_CHARS: usize = 32;
const MAX_COMMENT_CHARS: usize = 240;
const MISSING_BODY_PLACEHOLDER: &str = "(no text)";

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "user",
        type_hint: "string",
        required: false,
        default: Some("\"spez\""),
        description: "Reddit username (without `/u/` prefix).",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Number of comments to display.",
    },
    OptionSchema {
        name: "max_chars",
        type_hint: "integer (32..=240)",
        required: false,
        default: Some("110"),
        description: "Max preview length per comment line.",
    },
];

pub struct RedditUserCommentsFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    count: Option<u32>,
    #[serde(default)]
    max_chars: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct Comment {
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    score: Option<i64>,
    #[serde(default)]
    subreddit: Option<String>,
    #[serde(default)]
    permalink: Option<String>,
}

#[async_trait]
impl Fetcher for RedditUserCommentsFetcher {
    fn name(&self) -> &str {
        "reddit_user_comments"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Recent comments by a single Reddit user, each prefixed with subreddit and score and linked to the comment's permalink. Use `reddit_user_posts` for that user's submissions or `reddit_subreddit_posts` for a subreddit's listing."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        cache_key_for(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        sample_comment_body(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let count = normalized_count(opts.count);
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let max_chars = opts
            .max_chars
            .unwrap_or(DEFAULT_COMMENT_CHARS)
            .clamp(MIN_COMMENT_CHARS, MAX_COMMENT_CHARS);
        let user = normalize_user(opts.user.as_deref().unwrap_or(DEFAULT_USER))?;
        match fetch_user_comments(&user, count).await {
            Ok(comments) => Ok(payload(render_comments(&comments, max_chars, shape))),
            Err(err) => Ok(payload(network_unavailable_body(shape, &format!("{err}")))),
        }
    }
}

async fn fetch_user_comments(user: &str, count: usize) -> Result<Vec<Comment>, FetchError> {
    let path = format!("/user/{user}/comments.json?limit={count}&raw_json=1");
    fetch_listing(&path).await
}

fn render_comments(comments: &[Comment], max_chars: usize, shape: Shape) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: comments
                .iter()
                .map(|comment| Entry {
                    key: truncate(comment_body(comment), max_chars),
                    value: Some(comment_meta(comment)),
                    status: None,
                })
                .collect(),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: comments
                .iter()
                .map(|comment| LinkedLine {
                    text: format!(
                        "{}  {}",
                        comment_meta(comment),
                        truncate(comment_body(comment), max_chars),
                    ),
                    url: comment_link(comment),
                })
                .collect(),
        }),
        _ => Body::TextBlock(TextBlockData {
            lines: comments
                .iter()
                .map(|comment| {
                    format!(
                        "{}  {}",
                        comment_meta(comment),
                        truncate(comment_body(comment), max_chars),
                    )
                })
                .collect(),
        }),
    }
}

fn comment_body(comment: &Comment) -> &str {
    comment.body.as_deref().unwrap_or(MISSING_BODY_PLACEHOLDER)
}

fn comment_meta(comment: &Comment) -> String {
    let subreddit = comment
        .subreddit
        .as_deref()
        .map(|name| format!("r/{name} · "))
        .unwrap_or_default();
    let score = comment.score.unwrap_or(0);
    format!("{subreddit}{score}↑")
}

fn comment_link(comment: &Comment) -> Option<String> {
    comment
        .permalink
        .as_deref()
        .map(|path| format!("{SITE_BASE}{path}"))
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.into();
    }
    let keep = max_chars.saturating_sub(1);
    let clipped: String = value.chars().take(keep).collect();
    format!("{clipped}…")
}

fn sample_comment_body(shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::LinkedTextBlock => samples::linked_text_block(&[
            (
                "r/rust · 42↑  This migration guide made the update painless...",
                Some("https://www.reddit.com/r/rust/comments/abc123/topic/def456/"),
            ),
            (
                "r/programming · 18↑  +1 for clear release notes and rollback steps.",
                Some("https://www.reddit.com/r/programming/comments/ghi789/topic/jkl012/"),
            ),
        ]),
        Shape::TextBlock => samples::text_block(&[
            "r/rust · 42↑  This migration guide made the update painless...",
            "r/programming · 18↑  +1 for clear release notes and rollback steps.",
        ]),
        Shape::Entries => samples::entries(&[
            (
                "This migration guide made the update painless...",
                "r/rust · 42↑",
            ),
            (
                "+1 for clear release notes and rollback steps.",
                "r/programming · 18↑",
            ),
        ]),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ctx(shape: Option<Shape>, options: Option<toml::Value>) -> FetchContext {
        FetchContext {
            widget_id: "reddit-user-comments".into(),
            timeout: Duration::from_secs(1),
            shape,
            options,
            ..Default::default()
        }
    }

    #[test]
    fn options_parse_max_chars() {
        let raw: toml::Value =
            toml::from_str("user = \"spez\"\ncount = 5\nmax_chars = 80").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.user.as_deref(), Some("spez"));
        assert_eq!(opts.count, Some(5));
        assert_eq!(opts.max_chars, Some(80));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("user = \"spez\"\nbogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn truncate_adds_ellipsis_when_needed() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 4), "abc");
    }

    #[test]
    fn comment_meta_handles_missing_subreddit() {
        let comment = Comment {
            body: Some("hello".into()),
            score: Some(7),
            subreddit: None,
            permalink: None,
        };
        assert_eq!(comment_meta(&comment), "7↑");
    }

    #[test]
    fn comment_meta_supports_negative_score() {
        let comment = Comment {
            body: Some("hi".into()),
            score: Some(-3),
            subreddit: Some("rust".into()),
            permalink: None,
        };
        assert_eq!(comment_meta(&comment), "r/rust · -3↑");
    }

    #[test]
    fn render_comments_linked_text_block_uses_permalink() {
        let body = render_comments(
            &[Comment {
                body: Some("Hello from a comment".into()),
                score: Some(9),
                subreddit: Some("rust".into()),
                permalink: Some("/r/rust/comments/abc/post/def/".into()),
            }],
            80,
            Shape::LinkedTextBlock,
        );
        assert!(matches!(&body, Body::LinkedTextBlock(_)));
        if let Body::LinkedTextBlock(data) = body {
            assert_eq!(data.items[0].text, "r/rust · 9↑  Hello from a comment");
            assert_eq!(
                data.items[0].url.as_deref(),
                Some("https://www.reddit.com/r/rust/comments/abc/post/def/"),
            );
        }
    }

    #[test]
    fn render_comments_falls_back_to_placeholder_when_body_missing() {
        let body = render_comments(
            &[Comment {
                body: None,
                score: Some(0),
                subreddit: None,
                permalink: None,
            }],
            80,
            Shape::TextBlock,
        );
        assert!(matches!(&body, Body::TextBlock(_)));
        if let Body::TextBlock(text) = body {
            assert!(text.lines[0].contains(MISSING_BODY_PLACEHOLDER));
        }
    }

    #[test]
    fn render_comments_entries_truncate_long_body() {
        let body = render_comments(
            &[Comment {
                body: Some("a".repeat(200)),
                score: Some(1),
                subreddit: Some("rust".into()),
                permalink: None,
            }],
            32,
            Shape::Entries,
        );
        assert!(matches!(&body, Body::Entries(_)));
        if let Body::Entries(entries) = body {
            assert!(entries.items[0].key.ends_with('…'));
            assert_eq!(entries.items[0].key.chars().count(), 32);
            assert_eq!(entries.items[0].value.as_deref(), Some("r/rust · 1↑"));
        }
    }

    #[test]
    fn render_comments_drops_url_when_permalink_missing() {
        let body = render_comments(
            &[Comment {
                body: Some("hi".into()),
                score: Some(1),
                subreddit: Some("rust".into()),
                permalink: None,
            }],
            80,
            Shape::LinkedTextBlock,
        );
        assert!(matches!(&body, Body::LinkedTextBlock(_)));
        if let Body::LinkedTextBlock(block) = body {
            assert!(block.items[0].url.is_none());
        }
    }

    #[test]
    fn comment_link_prepends_site_base() {
        let comment = Comment {
            body: None,
            score: None,
            subreddit: None,
            permalink: Some("/r/rust/comments/abc/x/".into()),
        };
        assert_eq!(
            comment_link(&comment).as_deref(),
            Some("https://www.reddit.com/r/rust/comments/abc/x/"),
        );
    }

    #[test]
    fn comment_deserializes_full_shape() {
        let raw = r#"{
          "body": "hello",
          "score": -2,
          "subreddit": "rust",
          "permalink": "/r/rust/comments/abc/x/"
        }"#;
        let c: Comment = serde_json::from_str(raw).unwrap();
        assert_eq!(c.body.as_deref(), Some("hello"));
        assert_eq!(c.score, Some(-2));
        assert_eq!(c.subreddit.as_deref(), Some("rust"));
        assert_eq!(c.permalink.as_deref(), Some("/r/rust/comments/abc/x/"));
    }

    #[test]
    fn truncate_preserves_multibyte_safely() {
        // Multi-byte chars must not be cut mid-codepoint; chars().take() is the contract.
        let s = "あいうえおかきくけこ"; // 10 chars, 30 bytes UTF-8
        assert_eq!(truncate(s, 5).chars().count(), 5);
        assert!(truncate(s, 5).ends_with('…'));
    }

    #[test]
    fn sample_comment_body_returns_some_for_supported_shapes() {
        for shape in [Shape::LinkedTextBlock, Shape::TextBlock, Shape::Entries] {
            assert!(
                sample_comment_body(shape).is_some(),
                "expected sample for {shape:?}"
            );
        }
    }

    #[test]
    fn sample_comment_body_returns_none_for_unsupported_shape() {
        assert!(sample_comment_body(Shape::Ratio).is_none());
    }

    #[test]
    fn options_default_to_none() {
        let opts = Options::default();
        assert!(opts.user.is_none());
        assert!(opts.count.is_none());
        assert!(opts.max_chars.is_none());
    }

    #[test]
    fn fetcher_exposes_catalog_metadata_and_samples() {
        let fetcher = RedditUserCommentsFetcher;
        assert_eq!(fetcher.name(), "reddit_user_comments");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(
            fetcher
                .option_schemas()
                .iter()
                .map(|schema| schema.name)
                .collect::<Vec<_>>(),
            vec!["user", "count", "max_chars"]
        );
        assert!(matches!(
            fetcher.sample_body(Shape::LinkedTextBlock),
            Some(Body::LinkedTextBlock(_))
        ));
        assert!(matches!(
            fetcher.sample_body(Shape::TextBlock),
            Some(Body::TextBlock(_))
        ));
        assert!(matches!(
            fetcher.sample_body(Shape::Entries),
            Some(Body::Entries(_))
        ));
        assert!(fetcher.sample_body(Shape::Ratio).is_none());
    }

    #[test]
    fn cache_key_changes_with_shape_and_options() {
        let fetcher = RedditUserCommentsFetcher;
        let linked = fetcher.cache_key(&ctx(Some(Shape::LinkedTextBlock), None));
        let entries = fetcher.cache_key(&ctx(Some(Shape::Entries), None));
        let configured = fetcher.cache_key(&ctx(
            Some(Shape::LinkedTextBlock),
            Some(toml::from_str("user = \"spez\"\ncount = 5\nmax_chars = 80").unwrap()),
        ));
        assert_ne!(linked, entries);
        assert_ne!(linked, configured);
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_options_before_network() {
        let err = RedditUserCommentsFetcher
            .fetch(&ctx(
                Some(Shape::TextBlock),
                Some(toml::from_str("bogus = true").unwrap()),
            ))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("unknown field"));
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_user_before_network() {
        let err = RedditUserCommentsFetcher
            .fetch(&ctx(
                None,
                Some(toml::from_str("user = \"/u/\"\ncount = 0\nmax_chars = 999").unwrap()),
            ))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("user must not be empty"));
    }
}
