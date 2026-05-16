//! `reddit_trending` — site-wide trending via Reddit's `/r/popular.json` listing.
//!
//! Distinct from `reddit_subreddit_posts` (a single subreddit) and the `reddit_user_*` family
//! (one user's submissions / comments): this surfaces what Reddit's algorithm picks as hot
//! across *all* subreddits right now. Anonymous read on the fixed `www.reddit.com` host →
//! Safety::Safe.

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::github::common::{parse_options, payload};
use crate::fetcher::thumbnails;
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Bar, BarsData, Body, MarkdownTextBlockData, Payload, TextData};
use crate::render::Shape;
use crate::samples;

use super::client::fetch_listing;
use super::common::{
    Post, cache_key_for, network_unavailable_body, normalized_count, post_meta, post_title,
    render_posts, render_posts_image_linked,
};

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::ImageLinkedList,
    Shape::TextBlock,
    Shape::Text,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "count",
    type_hint: "integer (1..=30)",
    required: false,
    default: Some("10"),
    description: "Number of trending posts to display.",
}];

pub struct RedditTrendingFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    count: Option<u32>,
}

#[async_trait]
impl Fetcher for RedditTrendingFetcher {
    fn name(&self) -> &str {
        "reddit_trending"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Trending posts across all subreddits via Reddit's public `/r/popular.json` listing — what Reddit picks as hot site-wide right now. Distinct from `reddit_subreddit_posts` (a specific sub) and `reddit_user_posts` (one account's submissions)."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 30
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
        sample_trending_body(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let count = normalized_count(opts.count);
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        match fetch_listing::<Post>(&listing_path(count)).await {
            Ok(posts) => Ok(payload(render_for_shape(&posts, shape).await)),
            Err(err) => Ok(payload(network_unavailable_body(shape, &format!("{err}")))),
        }
    }
}

fn listing_path(count: usize) -> String {
    format!("/r/popular.json?limit={count}&raw_json=1")
}

/// `ImageLinkedList` is the one shape that needs an async detour to resolve each post's
/// thumbnail URL into a local cache path; every other shape is rendered synchronously.
async fn render_for_shape(posts: &[Post], shape: Shape) -> Body {
    if matches!(shape, Shape::ImageLinkedList) {
        let urls: Vec<Option<String>> = posts
            .iter()
            .map(|p| p.thumbnail_url().map(str::to_string))
            .collect();
        let paths = thumbnails::download_many(&urls).await;
        return render_posts_image_linked(posts, &paths);
    }
    render_trending_sync(posts, shape)
}

fn render_trending_sync(posts: &[Post], shape: Shape) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: trending_headline(posts),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: markdown_lines(posts),
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: posts.iter().map(score_bar).collect(),
        }),
        _ => render_posts(posts, shape),
    }
}

fn trending_headline(posts: &[Post]) -> String {
    posts
        .first()
        .map(|p| format!("{}  {}", post_meta(p), post_title(p)))
        .unwrap_or_else(|| "(no trending posts)".into())
}

fn markdown_lines(posts: &[Post]) -> String {
    posts
        .iter()
        .map(|p| format!("- **{}** — {}", post_title(p), post_meta(p)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Reddit scores can be negative; clamp to 0 because `Bar.value` is `u64`. Downvoted
/// front-page posts are vanishingly rare, but the trait contract has to hold.
fn score_bar(post: &Post) -> Bar {
    Bar {
        label: post_title(post),
        value: post.score.unwrap_or(0).max(0) as u64,
    }
}

fn sample_trending_body(shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::LinkedTextBlock => samples::linked_text_block(&[
            (
                "r/news · 25420↑ 1820c  Major breaking story headline",
                Some("https://www.reddit.com/r/news/comments/abc/"),
            ),
            (
                "r/AskReddit · 18230↑ 4210c  What's your biggest red flag?",
                Some("https://www.reddit.com/r/AskReddit/comments/def/"),
            ),
        ]),
        Shape::ImageLinkedList => samples::image_linked_list(&[
            (
                "Major breaking story headline",
                Some("https://www.reddit.com/r/news/comments/abc/"),
                None,
                Some("r/news · 25420↑ 1820c"),
            ),
            (
                "What's your biggest red flag?",
                Some("https://www.reddit.com/r/AskReddit/comments/def/"),
                None,
                Some("r/AskReddit · 18230↑ 4210c"),
            ),
        ]),
        Shape::TextBlock => samples::text_block(&[
            "r/news · 25420↑ 1820c  Major breaking story headline",
            "r/AskReddit · 18230↑ 4210c  What's your biggest red flag?",
        ]),
        Shape::Text => samples::text("r/news · 25420↑ 1820c  Major breaking story headline"),
        Shape::MarkdownTextBlock => samples::markdown(
            "- **Major breaking story headline** — r/news · 25420↑ 1820c\n- **What's your biggest red flag?** — r/AskReddit · 18230↑ 4210c",
        ),
        Shape::Entries => samples::entries(&[
            ("Major breaking story headline", "r/news · 25420↑ 1820c"),
            (
                "What's your biggest red flag?",
                "r/AskReddit · 18230↑ 4210c",
            ),
        ]),
        Shape::Bars => samples::bars(&[
            ("Major breaking story headline", 25420),
            ("What's your biggest red flag?", 18230),
        ]),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn ctx(shape: Option<Shape>, options: Option<toml::Value>) -> FetchContext {
        FetchContext {
            widget_id: "reddit-trending".into(),
            timeout: Duration::from_secs(1),
            shape,
            options,
            ..Default::default()
        }
    }

    fn post(title: &str, score: i64, sub: &str) -> Post {
        Post {
            title: Some(title.into()),
            score: Some(score),
            num_comments: Some(0),
            subreddit: Some(sub.into()),
            permalink: None,
            url: Some(format!("https://example.com/{}", title.replace(' ', "_"))),
            thumbnail: None,
        }
    }

    #[test]
    fn listing_path_targets_r_popular_with_limit() {
        assert_eq!(listing_path(15), "/r/popular.json?limit=15&raw_json=1");
    }

    #[test]
    fn options_default_to_none() {
        let opts = Options::default();
        assert!(opts.count.is_none());
    }

    #[test]
    fn options_deserialize_count() {
        let raw: toml::Value = toml::from_str("count = 7").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.count, Some(7));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("bogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn fetcher_exposes_catalog_metadata_and_samples() {
        let fetcher = RedditTrendingFetcher;
        assert_eq!(fetcher.name(), "reddit_trending");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(
            fetcher
                .option_schemas()
                .iter()
                .map(|schema| schema.name)
                .collect::<Vec<_>>(),
            vec!["count"]
        );
        for shape in SHAPES {
            assert!(
                fetcher.sample_body(*shape).is_some(),
                "expected sample for {shape:?}"
            );
        }
        assert!(fetcher.sample_body(Shape::Ratio).is_none());
    }

    #[test]
    fn render_trending_sync_text_uses_first_post() {
        let posts = vec![post("first", 100, "rust"), post("second", 50, "go")];
        let body = render_trending_sync(&posts, Shape::Text);
        let Body::Text(t) = body else {
            panic!("expected text");
        };
        assert!(t.value.contains("first"));
        assert!(t.value.contains("r/rust"));
    }

    #[test]
    fn render_trending_sync_text_handles_empty() {
        let body = render_trending_sync(&[], Shape::Text);
        let Body::Text(t) = body else {
            panic!("expected text");
        };
        assert_eq!(t.value, "(no trending posts)");
    }

    #[test]
    fn render_trending_sync_markdown_lists_bullets() {
        let posts = vec![post("first", 100, "rust"), post("second", 50, "go")];
        let body = render_trending_sync(&posts, Shape::MarkdownTextBlock);
        let Body::MarkdownTextBlock(m) = body else {
            panic!("expected markdown");
        };
        assert!(m.value.contains("- **first**"));
        assert!(m.value.contains("- **second**"));
        assert!(m.value.contains("r/rust"));
    }

    #[test]
    fn render_trending_sync_bars_ranks_by_score_and_clamps_negative() {
        let posts = vec![post("popular", 9000, "rust"), post("downvoted", -3, "rust")];
        let body = render_trending_sync(&posts, Shape::Bars);
        let Body::Bars(b) = body else {
            panic!("expected bars");
        };
        assert_eq!(b.bars[0].label, "popular");
        assert_eq!(b.bars[0].value, 9000);
        assert_eq!(b.bars[1].value, 0);
    }

    #[test]
    fn render_trending_sync_delegates_unknown_shapes_to_render_posts() {
        let posts = vec![post("first", 1, "rust")];
        let body = render_trending_sync(&posts, Shape::LinkedTextBlock);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked_text_block");
        };
        assert_eq!(b.items.len(), 1);
        assert!(b.items[0].text.contains("first"));
    }

    #[test]
    fn cache_key_changes_with_shape_and_count() {
        let fetcher = RedditTrendingFetcher;
        let linked = fetcher.cache_key(&ctx(Some(Shape::LinkedTextBlock), None));
        let bars = fetcher.cache_key(&ctx(Some(Shape::Bars), None));
        let custom = fetcher.cache_key(&ctx(
            Some(Shape::LinkedTextBlock),
            Some(toml::from_str("count = 30").unwrap()),
        ));
        assert_ne!(linked, bars);
        assert_ne!(linked, custom);
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_options_before_network() {
        let err = RedditTrendingFetcher
            .fetch(&ctx(
                Some(Shape::TextBlock),
                Some(toml::from_str("bogus = true").unwrap()),
            ))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("unknown field"));
    }

    #[tokio::test]
    async fn render_for_shape_image_linked_returns_empty_list_for_no_posts() {
        let body = render_for_shape(&[], Shape::ImageLinkedList).await;
        match body {
            Body::ImageLinkedList(data) => assert!(data.items.is_empty()),
            other => panic!("expected ImageLinkedList, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn render_for_shape_non_image_dispatches_to_sync_renderer() {
        let body = render_for_shape(&[], Shape::Bars).await;
        match body {
            Body::Bars(data) => assert!(data.bars.is_empty()),
            other => panic!("expected Bars, got {other:?}"),
        }
    }
}
