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
        let Body::LinkedTextBlock(data) = body else {
            panic!("expected linked text block");
        };
        assert_eq!(data.items[0].text, "r/rust · 9↑  Hello from a comment");
        assert_eq!(
            data.items[0].url.as_deref(),
            Some("https://www.reddit.com/r/rust/comments/abc/post/def/"),
        );
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
        let Body::TextBlock(t) = body else {
            panic!("expected text block");
        };
        assert!(t.lines[0].contains(MISSING_BODY_PLACEHOLDER));
    }
}
