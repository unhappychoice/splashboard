//! `news_*` — curated single-source feed family.
//!
//! Each `NewsSource` in [`sources::SOURCES`] is exposed as a separate `news_<slug>` fetcher with
//! a hardcoded feed URL set. Compared to the generic [`crate::fetcher::rss::RssFetcher`]:
//!
//! - URLs are baked at compile time (no `url` option), so the family is `Safety::Safe` —
//!   trust-gate friction drops, and a per-dir `.splashboard/dashboard.toml` shipped with a repo
//!   renders for cloners without `splashboard trust`.
//! - Each source can carry multiple sub-feeds (`feed = "tech"`); config picks among them via a
//!   closed enum, but cannot inject new destinations.
//!
//! Parsing / fetching / row-formatting live in [`crate::fetcher::feed`], shared with `rss`.

mod sources;

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use url::Url;

pub use sources::{NewsCategory, NewsFeed, NewsSource, SOURCES};

use crate::fetcher::feed;
use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;
use crate::samples;

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::ImageLinkedList,
    Shape::TextBlock,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "feed",
        type_hint: "string (source-specific key)",
        required: false,
        default: Some("first sub-feed of the source"),
        description: "Sub-feed key to select (e.g. `\"world\"` / `\"tech\"`). Run `splashboard catalog fetcher news_<source>` to list available keys.",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=20)",
        required: false,
        default: Some("5"),
        description: "Number of feed entries to display.",
    },
];

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    SOURCES
        .iter()
        .map(|source| -> Arc<dyn Fetcher> { Arc::new(NewsFeedFetcher { source }) })
        .collect()
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    feed: Option<String>,
    #[serde(default)]
    count: Option<u32>,
}

pub struct NewsFeedFetcher {
    source: &'static NewsSource,
}

#[async_trait]
impl Fetcher for NewsFeedFetcher {
    fn name(&self) -> &str {
        self.source.name
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        self.source.description
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
        cache_key(self.source.name, ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        let display = self.source.display;
        Some(match shape {
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    &format!("Apr 26  {display}: top story"),
                    Some("https://example.com/article-1"),
                ),
                (
                    &format!("Apr 25  {display}: feature piece"),
                    Some("https://example.com/article-2"),
                ),
                (
                    &format!("Apr 24  {display}: shorter update"),
                    Some("https://example.com/article-3"),
                ),
            ]),
            Shape::ImageLinkedList => samples::image_linked_list(&[
                (
                    "Top story",
                    Some("https://example.com/article-1"),
                    None,
                    Some("Apr 26"),
                ),
                (
                    "Feature piece",
                    Some("https://example.com/article-2"),
                    None,
                    Some("Apr 25"),
                ),
                (
                    "Shorter update",
                    Some("https://example.com/article-3"),
                    None,
                    Some("Apr 24"),
                ),
            ]),
            Shape::TextBlock => samples::text_block(&[
                &format!("Apr 26  {display}: top story"),
                &format!("Apr 25  {display}: feature piece"),
                &format!("Apr 24  {display}: shorter update"),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let selected = resolve_feed(self.source, opts.feed.as_deref())?;
        let url = Url::parse(selected.url).map_err(|e| {
            FetchError::Failed(format!(
                "{}: bundled feed url for `{}` is malformed: {e}",
                self.source.name, selected.key
            ))
        })?;
        let count = opts
            .count
            .unwrap_or(feed::DEFAULT_COUNT)
            .clamp(feed::MIN_COUNT, feed::MAX_COUNT) as usize;
        let bytes = feed::fetch_bytes(&url, self.source.name).await?;
        let parsed = feed::parse_feed(&bytes, self.source.name)?;
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let body = match shape {
            Shape::ImageLinkedList => feed::render_image_linked(&parsed, count, ctx).await,
            other => feed::render_body(
                &parsed,
                count,
                other,
                ctx.timezone.as_deref(),
                ctx.locale.as_deref(),
            ),
        };
        Ok(payload(body))
    }
}

fn resolve_feed(
    source: &'static NewsSource,
    requested: Option<&str>,
) -> Result<&'static NewsFeed, FetchError> {
    let Some(key) = requested.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(source.default_feed());
    };
    source.find_feed(key).ok_or_else(|| {
        let available = source
            .feeds
            .iter()
            .map(|f| f.key)
            .collect::<Vec<_>>()
            .join(", ");
        FetchError::Failed(format!(
            "{}: unknown feed key `{key}` (available: {available})",
            source.name
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ctx(shape: Option<Shape>, options: Option<toml::Value>) -> FetchContext {
        FetchContext {
            widget_id: "w".into(),
            timeout: Duration::from_secs(1),
            shape,
            options,
            ..Default::default()
        }
    }

    fn parse_opts(raw: &str) -> toml::Value {
        toml::from_str(raw).expect("test toml must parse")
    }

    fn bbc() -> &'static NewsSource {
        SOURCES
            .iter()
            .find(|s| s.name == "news_bbc")
            .expect("bbc source must exist")
    }

    fn aljazeera() -> &'static NewsSource {
        SOURCES
            .iter()
            .find(|s| s.name == "news_aljazeera")
            .expect("aljazeera source must exist")
    }

    #[test]
    fn all_sources_are_news_prefixed_and_have_at_least_one_feed() {
        for source in SOURCES {
            assert!(
                source.name.starts_with("news_"),
                "source name must use `news_` prefix: {}",
                source.name
            );
            assert!(!source.feeds.is_empty(), "{} has no feeds", source.name);
            for feed in source.feeds {
                assert!(!feed.key.is_empty(), "{} has empty feed key", source.name);
                assert!(
                    feed.url.starts_with("https://"),
                    "{}: feed `{}` must use https:// (got {})",
                    source.name,
                    feed.key,
                    feed.url
                );
            }
        }
    }

    #[test]
    fn source_names_are_unique() {
        let mut names: Vec<_> = SOURCES.iter().map(|s| s.name).collect();
        names.sort();
        let count = names.len();
        names.dedup();
        assert_eq!(count, names.len(), "duplicate source names in SOURCES");
    }

    #[test]
    fn feed_keys_are_unique_within_each_source() {
        for source in SOURCES {
            let mut keys: Vec<_> = source.feeds.iter().map(|f| f.key).collect();
            keys.sort();
            let count = keys.len();
            keys.dedup();
            assert_eq!(
                count,
                keys.len(),
                "duplicate feed keys in source {}",
                source.name
            );
        }
    }

    #[test]
    fn resolve_feed_defaults_to_first_entry_when_unset() {
        let resolved = resolve_feed(bbc(), None).unwrap();
        assert_eq!(resolved.key, bbc().feeds[0].key);
    }

    #[test]
    fn resolve_feed_defaults_when_key_is_whitespace() {
        let resolved = resolve_feed(bbc(), Some("  ")).unwrap();
        assert_eq!(resolved.key, bbc().feeds[0].key);
    }

    #[test]
    fn resolve_feed_picks_matching_sub_feed() {
        let resolved = resolve_feed(bbc(), Some("tech")).unwrap();
        assert_eq!(resolved.key, "tech");
    }

    #[test]
    fn resolve_feed_errors_on_unknown_key_and_lists_options() {
        let err = resolve_feed(bbc(), Some("zzz")).unwrap_err();
        match err {
            FetchError::Failed(m) => {
                assert!(m.contains("unknown feed key"), "msg: {m}");
                assert!(m.contains("zzz"), "msg: {m}");
                assert!(m.contains("world"), "msg should list available keys: {m}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn options_parse_feed_and_count() {
        let raw = parse_opts("feed = \"tech\"\ncount = 7");
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.feed.as_deref(), Some("tech"));
        assert_eq!(opts.count, Some(7));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw = parse_opts("feed = \"tech\"\nbogus = 1");
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn fetcher_exposes_safety_safe_and_name_from_source() {
        let f = NewsFeedFetcher { source: bbc() };
        assert_eq!(f.name(), "news_bbc");
        assert_eq!(f.safety(), Safety::Safe);
        assert_eq!(f.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(f.shapes(), SHAPES);
        assert_eq!(
            f.option_schemas()
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>(),
            vec!["feed", "count"]
        );
        assert!(f.description().contains("BBC"));
    }

    #[test]
    fn sample_body_covers_every_declared_shape() {
        let f = NewsFeedFetcher {
            source: aljazeera(),
        };
        assert!(matches!(
            f.sample_body(Shape::LinkedTextBlock),
            Some(Body::LinkedTextBlock(_))
        ));
        assert!(matches!(
            f.sample_body(Shape::ImageLinkedList),
            Some(Body::ImageLinkedList(_))
        ));
        assert!(matches!(
            f.sample_body(Shape::TextBlock),
            Some(Body::TextBlock(_))
        ));
        assert!(f.sample_body(Shape::Text).is_none());
    }

    #[test]
    fn cache_key_differs_between_sub_feeds() {
        let f = NewsFeedFetcher { source: bbc() };
        let mut a = ctx(None, None);
        let mut b = ctx(None, None);
        a.options = Some(parse_opts("feed = \"world\""));
        b.options = Some(parse_opts("feed = \"tech\""));
        assert_ne!(f.cache_key(&a), f.cache_key(&b));
    }

    #[test]
    fn fetchers_entry_point_registers_every_source() {
        let registered: Vec<_> = fetchers().iter().map(|f| f.name().to_string()).collect();
        assert_eq!(registered.len(), SOURCES.len());
        for source in SOURCES {
            assert!(
                registered.iter().any(|n| n == source.name),
                "{} missing from fetchers()",
                source.name
            );
        }
    }

    /// Live smoke test for every bundled `news_*` feed. Hits each URL in parallel, parses with
    /// feed-rs, and asserts at least one entry comes back. `#[ignore]` keeps CI offline-safe;
    /// run with `cargo test -- --ignored fetcher::news::tests::live_every_bundled_feed_parses`
    /// to surface dead feeds (404, redirect to login, schema change). The report includes per-
    /// feed `(source, key, entries)` rows on success and the failing URL + reason otherwise.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    #[ignore]
    async fn live_every_bundled_feed_parses() {
        let targets: Vec<(&'static str, &'static str, &'static str)> = SOURCES
            .iter()
            .flat_map(|s| s.feeds.iter().map(move |f| (s.name, f.key, f.url)))
            .collect();
        let mut tasks = tokio::task::JoinSet::new();
        for (source_name, key, raw_url) in targets {
            tasks.spawn(async move {
                let url = match Url::parse(raw_url) {
                    Ok(u) => u,
                    Err(e) => return (source_name, key, raw_url, Err(format!("parse url: {e}"))),
                };
                let bytes = match feed::fetch_bytes(&url, source_name).await {
                    Ok(b) => b,
                    Err(e) => return (source_name, key, raw_url, Err(format!("fetch: {e}"))),
                };
                let parsed = match feed::parse_feed(&bytes, source_name) {
                    Ok(p) => p,
                    Err(e) => return (source_name, key, raw_url, Err(format!("parse: {e}"))),
                };
                (source_name, key, raw_url, Ok(parsed.entries.len()))
            });
        }
        let mut failures = Vec::new();
        let mut successes = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            let (source, key, url, outcome) = joined.expect("join must succeed");
            match outcome {
                Ok(0) => failures.push(format!("{source} [{key}]  {url}  EMPTY")),
                Ok(n) => successes.push(format!("{source} [{key}]  {n} entries")),
                Err(e) => failures.push(format!("{source} [{key}]  {url}  {e}")),
            }
        }
        successes.sort();
        failures.sort();
        for line in &successes {
            eprintln!("OK {line}");
        }
        for line in &failures {
            eprintln!("FAIL {line}");
        }
        assert!(
            failures.is_empty(),
            "{} of {} bundled news feeds failed to parse — see eprintln output above",
            failures.len(),
            successes.len() + failures.len()
        );
    }
}
