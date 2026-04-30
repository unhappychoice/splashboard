//! `reddit_user_posts` — recent submissions for a Reddit user.

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::github::common::{parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;

use super::client::fetch_listing;
use super::common::{
    Post, SHAPES, cache_key_for, network_unavailable_body, normalize_user, normalized_count,
    render_posts, sample_post_body,
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
            Ok(posts) => Ok(payload(render_posts(&posts, shape))),
            Err(err) => Ok(payload(network_unavailable_body(shape, &format!("{err}")))),
        }
    }
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
}
