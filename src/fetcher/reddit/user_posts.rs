//! `reddit_user_posts` — recent submissions for a Reddit user.

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::github::common::{parse_options, payload};
use crate::fetcher::thumbnails;
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;

use super::client::fetch_listing;
use super::common::{
    Post, SHAPES, cache_key_for, network_unavailable_body, normalize_user, normalized_count,
    render_posts, render_posts_image_linked, sample_post_body,
};

const DEFAULT_USER: &str = "spez";

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
        description: "Number of submissions to display.",
    },
];

pub struct RedditUserPostsFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    count: Option<u32>,
}

#[async_trait]
impl Fetcher for RedditUserPostsFetcher {
    fn name(&self) -> &str {
        "reddit_user_posts"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Recent submissions by a single Reddit user. Use `reddit_user_comments` for that user's comments instead, or `reddit_subreddit_posts` to follow a subreddit rather than a person."
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
        sample_post_body(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let count = normalized_count(opts.count);
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let user = normalize_user(opts.user.as_deref().unwrap_or(DEFAULT_USER))?;
        match fetch_user_posts(&user, count).await {
            Ok(posts) => Ok(payload(render_for_shape(&posts, shape).await)),
            Err(err) => Ok(payload(network_unavailable_body(shape, &format!("{err}")))),
        }
    }
}

/// Sibling of [`reddit_subreddit_posts::render_for_shape`]: pre-downloads thumbnails for the
/// `ImageLinkedList` shape so the renderer can show them, and falls through to the sync
/// `render_posts` for every other shape.
async fn render_for_shape(posts: &[Post], shape: Shape) -> Body {
    if !matches!(shape, Shape::ImageLinkedList) {
        return render_posts(posts, shape);
    }
    let urls: Vec<Option<String>> = posts
        .iter()
        .map(|p| p.thumbnail_url().map(str::to_string))
        .collect();
    let paths = thumbnails::download_many(&urls).await;
    render_posts_image_linked(posts, &paths)
}

async fn fetch_user_posts(user: &str, count: usize) -> Result<Vec<Post>, FetchError> {
    let path = format!("/user/{user}/submitted.json?limit={count}&raw_json=1");
    fetch_listing(&path).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ctx(shape: Option<Shape>, options: Option<toml::Value>) -> FetchContext {
        FetchContext {
            widget_id: "reddit-user-posts".into(),
            timeout: Duration::from_secs(1),
            shape,
            options,
            ..Default::default()
        }
    }

    #[test]
    fn options_parse_user_and_count() {
        let raw: toml::Value = toml::from_str("user = \"spez\"\ncount = 5").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.user.as_deref(), Some("spez"));
        assert_eq!(opts.count, Some(5));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("user = \"spez\"\nbogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn options_default_to_none() {
        let opts = Options::default();
        assert!(opts.user.is_none());
        assert!(opts.count.is_none());
    }

    #[test]
    fn fetcher_exposes_catalog_metadata_and_samples() {
        let fetcher = RedditUserPostsFetcher;
        assert_eq!(fetcher.name(), "reddit_user_posts");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(
            fetcher
                .option_schemas()
                .iter()
                .map(|schema| schema.name)
                .collect::<Vec<_>>(),
            vec!["user", "count"]
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
        let fetcher = RedditUserPostsFetcher;
        let linked = fetcher.cache_key(&ctx(Some(Shape::LinkedTextBlock), None));
        let entries = fetcher.cache_key(&ctx(Some(Shape::Entries), None));
        let configured = fetcher.cache_key(&ctx(
            Some(Shape::LinkedTextBlock),
            Some(toml::from_str("user = \"spez\"\ncount = 5").unwrap()),
        ));
        assert_ne!(linked, entries);
        assert_ne!(linked, configured);
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_options_before_network() {
        let err = RedditUserPostsFetcher
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
        let err = RedditUserPostsFetcher
            .fetch(&ctx(
                None,
                Some(toml::from_str("user = \"/u/\"\ncount = 0").unwrap()),
            ))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("user must not be empty"));
    }

    #[test]
    fn description_mentions_sibling_widgets() {
        let desc = RedditUserPostsFetcher.description();
        assert!(desc.contains("submissions"), "desc: {desc}");
        assert!(desc.contains("reddit_user_comments"), "desc: {desc}");
        assert!(desc.contains("reddit_subreddit_posts"), "desc: {desc}");
    }

    #[tokio::test]
    async fn render_for_shape_image_linked_returns_empty_image_list_for_no_posts() {
        let body = render_for_shape(&[], Shape::ImageLinkedList).await;
        assert!(matches!(body, Body::ImageLinkedList(data) if data.items.is_empty()));
    }

    #[tokio::test]
    async fn render_for_shape_non_image_dispatches_to_sync_renderer() {
        let body = render_for_shape(&[], Shape::TextBlock).await;
        assert!(matches!(body, Body::TextBlock(data) if data.lines.is_empty()));
    }

    /// Exercises the iterator+collect closure body in `render_for_shape`'s ImageLinkedList arm
    /// with non-empty input. Both posts use Reddit's non-URL thumbnail placeholders (`"self"`,
    /// empty string), so `Post::thumbnail_url()` filters them to `None` and `download_many`
    /// short-circuits without hitting the network — covering the closure path that the
    /// empty-posts test bypasses.
    #[tokio::test]
    async fn render_for_shape_image_linked_with_posts_runs_thumbnail_resolution_closure() {
        let posts = vec![
            Post {
                title: Some("first".into()),
                score: Some(10),
                num_comments: Some(2),
                subreddit: Some("rust".into()),
                permalink: Some("/r/rust/comments/abc/first/".into()),
                url: Some("https://example.com/first".into()),
                thumbnail: Some("self".into()),
            },
            Post {
                title: Some("second".into()),
                score: Some(5),
                num_comments: Some(0),
                subreddit: Some("rust".into()),
                permalink: Some("/r/rust/comments/def/second/".into()),
                url: Some("https://example.com/second".into()),
                thumbnail: Some("".into()),
            },
        ];
        let body = render_for_shape(&posts, Shape::ImageLinkedList).await;
        assert!(matches!(body, Body::ImageLinkedList(data)
            if data.items.len() == 2
                && data.items.iter().all(|i| i.thumbnail_path.is_none())
                && data.items[0].title == "first"
                && data.items[1].title == "second"));
    }
}
