//! `wikipedia_featured` — today's Featured Article (TFA) via the Wikipedia REST API
//! `feed/featured/{YYYY}/{MM}/{DD}` endpoint.
//!
//! Safety::Safe: host is `*.wikipedia.org`. The TFA slot is best populated for `lang = "en"`;
//! other languages may return an empty `tfa` block, which surfaces as a fetch error so the
//! splash falls back to its `error_placeholder`.

use async_trait::async_trait;
use chrono::{Datelike, Utc};
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;
use crate::samples;

use super::client::{DEFAULT_LANG, PageSummary, get, render_page_summary, rest_api_base};

const SHAPES: &[Shape] = &[Shape::LinkedTextBlock, Shape::TextBlock, Shape::Text];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "lang",
    type_hint: "string (Wikipedia language code)",
    required: false,
    default: Some("\"en\""),
    description: "Wikipedia language edition. The TFA endpoint is best populated for `\"en\"`.",
}];

pub struct WikipediaFeaturedFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub lang: Option<String>,
}

#[async_trait]
impl Fetcher for WikipediaFeaturedFetcher {
    fn name(&self) -> &str {
        "wikipedia_featured"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Today's English Wikipedia \"Today's Featured Article\" — the daily curated front-page pick, with title, summary, and link. Use `wikipedia_on_this_day` for historical events on this date or `wikipedia_random` for an arbitrary article."
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
            Shape::LinkedTextBlock => samples::linked_text_block(&[(
                "Hyperion (poem)",
                Some("https://en.wikipedia.org/wiki/Hyperion_(poem)"),
            )]),
            Shape::TextBlock => samples::text_block(&[
                "Hyperion (poem)",
                "Hyperion is an unfinished epic poem by John Keats, recounting the despair of the Titans after their defeat by the Olympians.",
            ]),
            Shape::Text => {
                samples::text("Hyperion (poem): Hyperion is an unfinished epic poem by John Keats.")
            }
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let lang = opts.lang.as_deref().unwrap_or(DEFAULT_LANG);
        let summary = fetch_tfa(lang).await?;
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        Ok(payload(render_page_summary(&summary, shape)))
    }
}

async fn fetch_tfa(lang: &str) -> Result<PageSummary, FetchError> {
    let now = Utc::now();
    let url = format!(
        "{}/feed/featured/{:04}/{:02}/{:02}",
        rest_api_base(lang),
        now.year(),
        now.month(),
        now.day()
    );
    let response: FeaturedResponse = get(&url).await?;
    response
        .tfa
        .ok_or_else(|| FetchError::Failed("wikipedia featured: no `tfa` in response".into()))
}

#[derive(Debug, Deserialize)]
struct FeaturedResponse {
    #[serde(default)]
    tfa: Option<PageSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetcher::FetchContext;

    fn ctx(options: Option<&str>, shape: Option<Shape>) -> FetchContext {
        FetchContext {
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            ..FetchContext::default()
        }
    }

    #[test]
    fn options_default_lang_to_none() {
        let opts = Options::default();
        assert!(opts.lang.is_none());
    }

    #[test]
    fn options_deserialize_lang() {
        let raw: toml::Value = toml::from_str("lang = \"ja\"").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.lang.as_deref(), Some("ja"));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("lang = \"en\"\nbogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn featured_response_deserializes_with_tfa() {
        let raw = r#"{"tfa":{"title":"Hyperion","extract":"x.","content_urls":{"desktop":{"page":"https://en.wikipedia.org/wiki/Hyperion"}}}}"#;
        let r: FeaturedResponse = serde_json::from_str(raw).unwrap();
        let tfa = r.tfa.unwrap();
        assert_eq!(tfa.title, "Hyperion");
    }

    #[test]
    fn featured_response_deserializes_without_tfa() {
        let raw = r#"{"news":[]}"#;
        let r: FeaturedResponse = serde_json::from_str(raw).unwrap();
        assert!(r.tfa.is_none());
    }

    #[test]
    fn fetcher_contract_and_samples_cover_catalog_surface() {
        let fetcher = WikipediaFeaturedFetcher;

        assert_eq!(fetcher.name(), "wikipedia_featured");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("Featured Article"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(fetcher.option_schemas().len(), 1);
        assert_eq!(fetcher.option_schemas()[0].name, OPTION_SCHEMAS[0].name);
        assert_eq!(
            fetcher.option_schemas()[0].default,
            OPTION_SCHEMAS[0].default
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
            fetcher.sample_body(Shape::Text),
            Some(Body::Text(_))
        ));
        assert!(fetcher.sample_body(Shape::Entries).is_none());
    }

    #[test]
    fn cache_key_changes_with_shape_and_lang() {
        let fetcher = WikipediaFeaturedFetcher;
        let default = fetcher.cache_key(&ctx(None, None));
        let ja = fetcher.cache_key(&ctx(Some("lang = \"ja\""), None));
        let ja_text = fetcher.cache_key(&ctx(Some("lang = \"ja\""), Some(Shape::Text)));

        assert_ne!(default, ja);
        assert_ne!(ja, ja_text);
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_options_before_request() {
        let err = WikipediaFeaturedFetcher
            .fetch(&ctx(Some("bogus = true"), None))
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            FetchError::Failed(message) if message.contains("unknown field")
        ));
    }

    /// Live smoke test — hits Wikipedia REST API.
    #[tokio::test]
    #[ignore]
    async fn live_returns_a_featured_article() {
        let s = fetch_tfa("en").await.unwrap();
        eprintln!("featured: {}", s.title);
        assert!(!s.title.is_empty());
    }
}
