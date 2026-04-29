//! `reddit_subreddit_posts` — public listing for a subreddit (`top|hot|new|rising`).

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::github::common::{parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;

use super::client::fetch_listing;
use super::common::{
    Post, SHAPES, cache_key_for, network_unavailable_body, normalize_subreddit, normalized_count,
    render_posts, sample_post_body,
};

const DEFAULT_SUBREDDIT: &str = "programming";

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "subreddit",
        type_hint: "string",
        required: false,
        default: Some("\"programming\""),
        description: "Subreddit name without `/r/` prefix.",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Number of posts to display.",
    },
    OptionSchema {
        name: "type",
        type_hint: "\"top\" | \"hot\" | \"new\" | \"rising\"",
        required: false,
        default: Some("\"top\""),
        description: "Subreddit listing type.",
    },
    OptionSchema {
        name: "period",
        type_hint: "\"hour\" | \"day\" | \"week\" | \"month\" | \"year\" | \"all\"",
        required: false,
        default: Some("\"day\""),
        description: "Ranking window used when `type = \"top\"`.",
    },
];

pub struct RedditSubredditPostsFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    subreddit: Option<String>,
    #[serde(default)]
    count: Option<u32>,
    #[serde(default, rename = "type")]
    r#type: Option<ListingType>,
    #[serde(default)]
    period: Option<Period>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ListingType {
    #[default]
    Top,
    Hot,
    New,
    Rising,
}

impl ListingType {
    fn path(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Hot => "hot",
            Self::New => "new",
            Self::Rising => "rising",
        }
    }

    fn needs_period(self) -> bool {
        matches!(self, Self::Top)
    }
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Period {
    Hour,
    #[default]
    Day,
    Week,
    Month,
    Year,
    All,
}

impl Period {
    fn as_query(self) -> &'static str {
        match self {
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
            Self::All => "all",
        }
    }
}

#[async_trait]
impl Fetcher for RedditSubredditPostsFetcher {
    fn name(&self) -> &str {
        "reddit_subreddit_posts"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Posts from a single subreddit's public listing — `top` / `hot` / `new` / `rising`, with a `period` window for `top`. Pair with `reddit_user_posts` for one user's submissions or `reddit_user_comments` for their comments."
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
        let listing_type = opts.r#type.unwrap_or_default();
        let period = opts.period.unwrap_or_default();
        let subreddit =
            normalize_subreddit(opts.subreddit.as_deref().unwrap_or(DEFAULT_SUBREDDIT))?;
        match fetch_subreddit_posts(&subreddit, count, listing_type, period).await {
            Ok(posts) => Ok(payload(render_posts(&posts, shape))),
            Err(err) => Ok(payload(network_unavailable_body(shape, &format!("{err}")))),
        }
    }
}

async fn fetch_subreddit_posts(
    subreddit: &str,
    count: usize,
    listing_type: ListingType,
    period: Period,
) -> Result<Vec<Post>, FetchError> {
    fetch_listing(&listing_path(subreddit, count, listing_type, period)).await
}

fn listing_path(
    subreddit: &str,
    count: usize,
    listing_type: ListingType,
    period: Period,
) -> String {
    let mut path = format!(
        "/r/{subreddit}/{}.json?limit={count}&raw_json=1",
        listing_type.path()
    );
    if listing_type.needs_period() {
        path.push_str("&t=");
        path.push_str(period.as_query());
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ctx(shape: Option<Shape>, options: Option<toml::Value>) -> FetchContext {
        FetchContext {
            widget_id: "reddit-subreddit-posts".into(),
            timeout: Duration::from_secs(1),
            shape,
            options,
            ..Default::default()
        }
    }

    #[test]
    fn options_parse_type_and_period() {
        let raw: toml::Value =
            toml::from_str("subreddit = \"rust\"\ncount = 7\ntype = \"hot\"\nperiod = \"week\"")
                .unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.subreddit.as_deref(), Some("rust"));
        assert_eq!(opts.count, Some(7));
        assert!(matches!(opts.r#type, Some(ListingType::Hot)));
        assert!(matches!(opts.period, Some(Period::Week)));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("subreddit = \"rust\"\nbogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn listing_type_controls_period_usage() {
        assert!(ListingType::Top.needs_period());
        assert!(!ListingType::Hot.needs_period());
        assert!(!ListingType::New.needs_period());
        assert!(!ListingType::Rising.needs_period());
    }

    #[test]
    fn listing_type_endpoint_paths() {
        assert_eq!(ListingType::Top.path(), "top");
        assert_eq!(ListingType::Hot.path(), "hot");
        assert_eq!(ListingType::New.path(), "new");
        assert_eq!(ListingType::Rising.path(), "rising");
    }

    #[test]
    fn period_query_covers_all_variants() {
        for (variant, expected) in [
            (Period::Hour, "hour"),
            (Period::Day, "day"),
            (Period::Week, "week"),
            (Period::Month, "month"),
            (Period::Year, "year"),
            (Period::All, "all"),
        ] {
            assert_eq!(variant.as_query(), expected);
        }
    }

    #[test]
    fn listing_path_appends_period_only_for_top() {
        assert_eq!(
            listing_path("rust", 10, ListingType::Top, Period::Week),
            "/r/rust/top.json?limit=10&raw_json=1&t=week"
        );
        assert_eq!(
            listing_path("rust", 10, ListingType::Hot, Period::Week),
            "/r/rust/hot.json?limit=10&raw_json=1"
        );
        assert_eq!(
            listing_path("rust", 5, ListingType::New, Period::Day),
            "/r/rust/new.json?limit=5&raw_json=1"
        );
        assert_eq!(
            listing_path("rust", 3, ListingType::Rising, Period::Day),
            "/r/rust/rising.json?limit=3&raw_json=1"
        );
    }

    #[test]
    fn options_default_to_none() {
        let opts = Options::default();
        assert!(opts.subreddit.is_none());
        assert!(opts.count.is_none());
        assert!(opts.r#type.is_none());
        assert!(opts.period.is_none());
    }

    #[test]
    fn fetcher_exposes_catalog_metadata_and_samples() {
        let fetcher = RedditSubredditPostsFetcher;
        assert_eq!(fetcher.name(), "reddit_subreddit_posts");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(
            fetcher
                .option_schemas()
                .iter()
                .map(|schema| schema.name)
                .collect::<Vec<_>>(),
            vec!["subreddit", "count", "type", "period"]
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
        let fetcher = RedditSubredditPostsFetcher;
        let linked = fetcher.cache_key(&ctx(Some(Shape::LinkedTextBlock), None));
        let entries = fetcher.cache_key(&ctx(Some(Shape::Entries), None));
        let configured = fetcher.cache_key(&ctx(
            Some(Shape::LinkedTextBlock),
            Some(
                toml::from_str(
                    "subreddit = \"rust\"\ncount = 30\ntype = \"new\"\nperiod = \"all\"",
                )
                .unwrap(),
            ),
        ));
        assert_ne!(linked, entries);
        assert_ne!(linked, configured);
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_options_before_network() {
        let err = RedditSubredditPostsFetcher
            .fetch(&ctx(
                Some(Shape::TextBlock),
                Some(toml::from_str("bogus = true").unwrap()),
            ))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("unknown field"));
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_subreddit_before_network() {
        let err = RedditSubredditPostsFetcher
            .fetch(&ctx(
                None,
                Some(
                    toml::from_str(
                        "subreddit = \"/r/\"\ncount = 0\ntype = \"rising\"\nperiod = \"year\"",
                    )
                    .unwrap(),
                ),
            ))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("subreddit must not be empty"));
    }
}
