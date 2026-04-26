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
}
