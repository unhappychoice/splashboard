//! Cross-cutting helpers for the `reddit_*` family: option parsing, name normalization,
//! the shared `Post` DTO, and `Post`-based rendering / sample bodies.

use serde::Deserialize;

use crate::fetcher::FetchContext;
use crate::fetcher::FetchError;
use crate::fetcher::github::common::cache_key;
use crate::payload::{Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData, TextBlockData};
use crate::render::Shape;
use crate::samples;

use super::client::SITE_BASE;

pub(super) const SHAPES: &[Shape] = &[Shape::LinkedTextBlock, Shape::TextBlock, Shape::Entries];

pub(super) const DEFAULT_COUNT: u32 = 10;
pub(super) const MIN_COUNT: u32 = 1;
pub(super) const MAX_COUNT: u32 = 30;

const SUBREDDIT_NAME_MAX: usize = 21;
const USER_NAME_MAX: usize = 20;

pub(super) fn cache_key_for(name: &str, ctx: &FetchContext) -> String {
    let extra = ctx
        .options
        .as_ref()
        .and_then(|value| toml::to_string(value).ok())
        .unwrap_or_default();
    cache_key(name, ctx, &extra)
}

pub(super) fn normalized_count(value: Option<u32>) -> usize {
    value.unwrap_or(DEFAULT_COUNT).clamp(MIN_COUNT, MAX_COUNT) as usize
}

pub(super) fn normalize_subreddit(raw: &str) -> Result<String, FetchError> {
    let trimmed = raw
        .trim()
        .trim_start_matches("https://www.reddit.com/r/")
        .trim_start_matches("/r/")
        .trim_matches('/');
    if trimmed.is_empty() {
        return Err(FetchError::Failed("subreddit must not be empty".into()));
    }
    if trimmed.len() > SUBREDDIT_NAME_MAX {
        return Err(FetchError::Failed("subreddit is too long".into()));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(FetchError::Failed(
            "subreddit must contain only [A-Za-z0-9_]".into(),
        ));
    }
    Ok(trimmed.to_ascii_lowercase())
}

pub(super) fn normalize_user(raw: &str) -> Result<String, FetchError> {
    let trimmed = raw
        .trim()
        .trim_start_matches("https://www.reddit.com/user/")
        .trim_start_matches("/u/")
        .trim_start_matches("u/")
        .trim_matches('/');
    if trimmed.is_empty() {
        return Err(FetchError::Failed("user must not be empty".into()));
    }
    if trimmed.len() > USER_NAME_MAX {
        return Err(FetchError::Failed("user is too long".into()));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(FetchError::Failed(
            "user must contain only [A-Za-z0-9_-]".into(),
        ));
    }
    Ok(trimmed.to_ascii_lowercase())
}

pub(super) fn network_unavailable_body(shape: Shape, err: &str) -> Body {
    let line = format!("⚠ {err}");
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: vec![Entry {
                key: "reddit unavailable".into(),
                value: Some(err.into()),
                status: None,
            }],
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: vec![LinkedLine {
                text: line,
                url: None,
            }],
        }),
        _ => Body::TextBlock(TextBlockData { lines: vec![line] }),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct Post {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub score: Option<i64>,
    #[serde(default)]
    pub num_comments: Option<u64>,
    #[serde(default)]
    pub subreddit: Option<String>,
    #[serde(default)]
    pub permalink: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

pub(super) fn render_posts(posts: &[Post], shape: Shape) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: posts
                .iter()
                .map(|post| Entry {
                    key: post_title(post),
                    value: Some(post_meta(post)),
                    status: None,
                })
                .collect(),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: posts
                .iter()
                .map(|post| LinkedLine {
                    text: format!("{}  {}", post_meta(post), post_title(post)),
                    url: post_link(post),
                })
                .collect(),
        }),
        _ => Body::TextBlock(TextBlockData {
            lines: posts
                .iter()
                .map(|post| format!("{}  {}", post_meta(post), post_title(post)))
                .collect(),
        }),
    }
}

pub(super) fn sample_post_body(shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::LinkedTextBlock => samples::linked_text_block(&[
            (
                "r/programming · 1520↑ 218c  Show: terminal dashboard with local trust model",
                Some("https://www.reddit.com/r/programming/comments/abc123/demo/"),
            ),
            (
                "r/rust · 934↑ 104c  Why async Rust feels different from Go",
                Some("https://example.com/async-rust-vs-go"),
            ),
        ]),
        Shape::TextBlock => samples::text_block(&[
            "r/programming · 1520↑ 218c  Show: terminal dashboard with local trust model",
            "r/rust · 934↑ 104c  Why async Rust feels different from Go",
        ]),
        Shape::Entries => samples::entries(&[
            (
                "Show: terminal dashboard with local trust model",
                "r/programming · 1520↑ 218c",
            ),
            (
                "Why async Rust feels different from Go",
                "r/rust · 934↑ 104c",
            ),
        ]),
        _ => return None,
    })
}

fn post_title(post: &Post) -> String {
    post.title.as_deref().unwrap_or("(no title)").to_string()
}

fn post_meta(post: &Post) -> String {
    let subreddit = post
        .subreddit
        .as_deref()
        .map(|name| format!("r/{name} · "))
        .unwrap_or_default();
    let score = post.score.unwrap_or(0);
    let comments = post.num_comments.unwrap_or(0);
    format!("{subreddit}{score}↑ {comments}c")
}

fn post_link(post: &Post) -> Option<String> {
    post.url
        .as_deref()
        .filter(|url| url.starts_with("http://") || url.starts_with("https://"))
        .map(String::from)
        .or_else(|| {
            post.permalink
                .as_deref()
                .map(|path| format!("{SITE_BASE}{path}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_subreddit_accepts_r_prefix_and_lowercases() {
        assert_eq!(normalize_subreddit("/r/Rust").unwrap(), "rust");
    }

    #[test]
    fn normalize_subreddit_strips_full_url() {
        assert_eq!(
            normalize_subreddit("https://www.reddit.com/r/Programming/").unwrap(),
            "programming"
        );
    }

    #[test]
    fn normalize_subreddit_rejects_empty() {
        assert!(normalize_subreddit("").is_err());
        assert!(normalize_subreddit("/r/").is_err());
    }

    #[test]
    fn normalize_subreddit_rejects_overlength() {
        let long = "a".repeat(SUBREDDIT_NAME_MAX + 1);
        let err = normalize_subreddit(&long).unwrap_err();
        assert!(format!("{err}").contains("too long"));
    }

    #[test]
    fn normalize_subreddit_rejects_invalid_chars() {
        assert!(normalize_subreddit("rust!").is_err());
        assert!(normalize_subreddit("a-b").is_err());
    }

    #[test]
    fn normalize_user_accepts_u_prefix_and_lowercases() {
        assert_eq!(normalize_user("/u/Spez").unwrap(), "spez");
    }

    #[test]
    fn normalize_user_rejects_invalid_chars() {
        let err = normalize_user("name.with.dot").unwrap_err();
        assert!(format!("{err}").contains("[A-Za-z0-9_-]"));
    }

    #[test]
    fn normalized_count_clamps_to_bounds() {
        assert_eq!(normalized_count(None), DEFAULT_COUNT as usize);
        assert_eq!(normalized_count(Some(0)), MIN_COUNT as usize);
        assert_eq!(normalized_count(Some(999)), MAX_COUNT as usize);
    }

    #[test]
    fn post_meta_includes_subreddit_score_and_comments() {
        let post = Post {
            title: Some("Hello".into()),
            score: Some(12),
            num_comments: Some(3),
            subreddit: Some("rust".into()),
            permalink: None,
            url: None,
        };
        assert_eq!(post_meta(&post), "r/rust · 12↑ 3c");
    }

    #[test]
    fn post_meta_supports_negative_score() {
        let post = Post {
            title: None,
            score: Some(-5),
            num_comments: Some(0),
            subreddit: Some("rust".into()),
            permalink: None,
            url: None,
        };
        assert_eq!(post_meta(&post), "r/rust · -5↑ 0c");
    }

    #[test]
    fn post_link_prefers_external_url_then_permalink() {
        let external = sample_post(Some("https://example.com/post"));
        assert_eq!(
            post_link(&external).as_deref(),
            Some("https://example.com/post"),
        );

        let permalink_only = sample_post(Some("not-a-http-url"));
        assert_eq!(
            post_link(&permalink_only).as_deref(),
            Some("https://www.reddit.com/r/rust/comments/abc/t/"),
        );
    }

    #[test]
    fn network_unavailable_body_matches_requested_shape() {
        let linked = network_unavailable_body(Shape::LinkedTextBlock, "reddit 403 Forbidden");
        let Body::LinkedTextBlock(linked) = linked else {
            panic!("expected linked_text_block");
        };
        assert!(linked.items[0].text.contains("reddit 403 Forbidden"));

        let entries = network_unavailable_body(Shape::Entries, "reddit timeout");
        let Body::Entries(entries) = entries else {
            panic!("expected entries");
        };
        assert_eq!(entries.items[0].key, "reddit unavailable");
        assert_eq!(entries.items[0].value.as_deref(), Some("reddit timeout"));

        let text = network_unavailable_body(Shape::TextBlock, "boom");
        let Body::TextBlock(t) = text else {
            panic!("expected text_block");
        };
        assert!(t.lines[0].contains("boom"));
    }

    #[test]
    fn render_posts_text_block_lines_include_meta_and_title() {
        let posts = vec![sample_post(Some("https://example.com/a"))];
        let body = render_posts(&posts, Shape::TextBlock);
        let Body::TextBlock(t) = body else {
            panic!("expected text_block");
        };
        assert_eq!(t.lines, vec!["r/rust · 1↑ 1c  t".to_string()]);
    }

    #[test]
    fn render_posts_entries_use_title_as_key_and_meta_as_value() {
        let posts = vec![sample_post(None)];
        let body = render_posts(&posts, Shape::Entries);
        let Body::Entries(e) = body else {
            panic!("expected entries");
        };
        assert_eq!(e.items[0].key, "t");
        assert_eq!(e.items[0].value.as_deref(), Some("r/rust · 1↑ 1c"));
    }

    #[test]
    fn render_posts_linked_text_block_carries_link() {
        let posts = vec![sample_post(Some("https://example.com/a"))];
        let body = render_posts(&posts, Shape::LinkedTextBlock);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked_text_block");
        };
        assert_eq!(b.items[0].text, "r/rust · 1↑ 1c  t");
        assert_eq!(b.items[0].url.as_deref(), Some("https://example.com/a"));
    }

    #[test]
    fn render_posts_falls_back_to_permalink_when_url_missing() {
        let posts = vec![Post {
            title: Some("hello".into()),
            score: Some(2),
            num_comments: Some(0),
            subreddit: Some("rust".into()),
            permalink: Some("/r/rust/comments/xyz/hello/".into()),
            url: None,
        }];
        let body = render_posts(&posts, Shape::LinkedTextBlock);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked_text_block");
        };
        assert_eq!(
            b.items[0].url.as_deref(),
            Some("https://www.reddit.com/r/rust/comments/xyz/hello/"),
        );
    }

    #[test]
    fn render_posts_handles_missing_title_subreddit_and_score() {
        let posts = vec![Post {
            title: None,
            score: None,
            num_comments: None,
            subreddit: None,
            permalink: None,
            url: None,
        }];
        let body = render_posts(&posts, Shape::TextBlock);
        let Body::TextBlock(t) = body else {
            panic!("expected text_block");
        };
        assert_eq!(t.lines, vec!["0↑ 0c  (no title)".to_string()]);
    }

    #[test]
    fn render_posts_empty_input_produces_empty_body() {
        let body = render_posts(&[], Shape::Entries);
        let Body::Entries(e) = body else {
            panic!("expected entries");
        };
        assert!(e.items.is_empty());

        let body = render_posts(&[], Shape::LinkedTextBlock);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked_text_block");
        };
        assert!(b.items.is_empty());
    }

    #[test]
    fn post_deserializes_full_listing_envelope() {
        let raw = r#"{
          "data": {
            "children": [
              {
                "data": {
                  "title": "Post",
                  "score": 42,
                  "num_comments": 7,
                  "subreddit": "rust",
                  "permalink": "/r/rust/comments/abc/post/",
                  "url": "https://example.com/post"
                }
              }
            ]
          }
        }"#;
        // Reuse the same envelope shape as `client::Listing<T>` to prove the Post DTO
        // deserializes against a realistic Reddit response.
        #[derive(serde::Deserialize)]
        struct Envelope {
            data: EnvelopeData,
        }
        #[derive(serde::Deserialize)]
        struct EnvelopeData {
            children: Vec<EnvelopeChild>,
        }
        #[derive(serde::Deserialize)]
        struct EnvelopeChild {
            data: Post,
        }
        let parsed: Envelope = serde_json::from_str(raw).unwrap();
        let first = &parsed.data.children[0].data;
        assert_eq!(first.title.as_deref(), Some("Post"));
        assert_eq!(first.score, Some(42));
        assert_eq!(first.num_comments, Some(7));
        assert_eq!(first.subreddit.as_deref(), Some("rust"));
        assert_eq!(
            first.permalink.as_deref(),
            Some("/r/rust/comments/abc/post/")
        );
        assert_eq!(first.url.as_deref(), Some("https://example.com/post"));
    }

    #[test]
    fn post_deserializes_with_negative_score() {
        let raw = r#"{ "title": "downvoted", "score": -42 }"#;
        let post: Post = serde_json::from_str(raw).unwrap();
        assert_eq!(post.score, Some(-42));
    }

    #[test]
    fn sample_post_body_returns_some_for_supported_shapes() {
        for shape in [Shape::LinkedTextBlock, Shape::TextBlock, Shape::Entries] {
            assert!(
                sample_post_body(shape).is_some(),
                "expected sample for {shape:?}"
            );
        }
    }

    #[test]
    fn sample_post_body_returns_none_for_unsupported_shape() {
        assert!(sample_post_body(Shape::Ratio).is_none());
    }

    fn sample_post(url: Option<&str>) -> Post {
        Post {
            title: Some("t".into()),
            score: Some(1),
            num_comments: Some(1),
            subreddit: Some("rust".into()),
            permalink: Some("/r/rust/comments/abc/t/".into()),
            url: url.map(String::from),
        }
    }
}
