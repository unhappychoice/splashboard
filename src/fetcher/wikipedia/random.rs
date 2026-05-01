//! `wikipedia_random` — a random page summary via `/page/random/summary`.
//!
//! Safety::Safe: host is `*.wikipedia.org`. Same `{title, extract, url}` triplet as
//! `wikipedia_featured`, just with a non-curated picker on the server side. Cache TTL
//! controls how often a new article is drawn — at the default `refresh_interval` the page
//! sticks for the configured window rather than rotating on every splash open.

use async_trait::async_trait;
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
    description: "Wikipedia language edition.",
}];

pub struct WikipediaRandomFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub lang: Option<String>,
}

#[async_trait]
impl Fetcher for WikipediaRandomFetcher {
    fn name(&self) -> &str {
        "wikipedia_random"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "An arbitrary Wikipedia page summary (title, extract, link), drawn from the language edition's random endpoint. Refresh interval controls how often a new article is picked. Use `wikipedia_featured` for the curated daily article or `wikipedia_on_this_day` for date-anchored history."
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
                "Quokka",
                Some("https://en.wikipedia.org/wiki/Quokka"),
            )]),
            Shape::TextBlock => samples::text_block(&[
                "Quokka",
                "The quokka is a small macropod about the size of a domestic cat, native to small islands and a small mainland area of southwestern Australia.",
            ]),
            Shape::Text => {
                samples::text("Quokka: A small macropod native to southwestern Australia.")
            }
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let lang = opts.lang.as_deref().unwrap_or(DEFAULT_LANG);
        let summary = fetch_random(lang).await?;
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        Ok(payload(render_page_summary(&summary, shape)))
    }
}

async fn fetch_random(lang: &str) -> Result<PageSummary, FetchError> {
    let url = format!("{}/page/random/summary", rest_api_base(lang));
    get::<PageSummary>(&url).await
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn ctx(options: Option<&str>, shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "wiki-random".into(),
            format: Some("compact".into()),
            timeout: Duration::from_secs(1),
            file_format: None,
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            timezone: None,
            locale: None,
        }
    }

    #[test]
    fn options_default_lang_to_none() {
        assert!(Options::default().lang.is_none());
    }

    #[test]
    fn options_deserialize_lang() {
        let raw: toml::Value = toml::from_str("lang = \"ja\"").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.lang.as_deref(), Some("ja"));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("lang = \"en\"\nbogus = 1").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn fetcher_metadata_samples_and_cache_key_cover_supported_shapes() {
        let fetcher = WikipediaRandomFetcher;
        assert_eq!(fetcher.name(), "wikipedia_random");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("random endpoint"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(fetcher.option_schemas().len(), 1);
        assert_eq!(fetcher.option_schemas()[0].name, "lang");

        let linked = fetcher.sample_body(Shape::LinkedTextBlock);
        assert!(matches!(linked, Some(Body::LinkedTextBlock(_))));
        if let Some(Body::LinkedTextBlock(linked)) = linked {
            assert_eq!(linked.items[0].text, "Quokka");
            assert_eq!(
                linked.items[0].url.as_deref(),
                Some("https://en.wikipedia.org/wiki/Quokka")
            );
        }

        let text_block = fetcher.sample_body(Shape::TextBlock);
        assert!(matches!(text_block, Some(Body::TextBlock(_))));
        if let Some(Body::TextBlock(text_block)) = text_block {
            assert_eq!(text_block.lines[0], "Quokka");
            assert!(text_block.lines[1].contains("small macropod"));
        }

        let text = fetcher.sample_body(Shape::Text);
        assert!(matches!(text, Some(Body::Text(_))));
        if let Some(Body::Text(text)) = text {
            assert!(text.value.starts_with("Quokka:"));
        }
        assert!(fetcher.sample_body(Shape::Entries).is_none());

        let a = fetcher.cache_key(&ctx(Some("lang = \"en\""), Some(Shape::TextBlock)));
        let b = fetcher.cache_key(&ctx(Some("lang = \"ja\""), Some(Shape::TextBlock)));
        let c = fetcher.cache_key(&ctx(Some("lang = \"en\""), Some(Shape::Text)));
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_options_before_network() {
        let fetcher = WikipediaRandomFetcher;
        let err = fetcher
            .fetch(&ctx(Some("bogus = true"), Some(Shape::Text)))
            .await
            .unwrap_err();
        assert!(matches!(&err, FetchError::Failed(message) if message.contains("unknown field")));
    }

    #[tokio::test]
    async fn fetch_surfaces_request_error_for_invalid_lang() {
        let fetcher = WikipediaRandomFetcher;
        let err = fetcher
            .fetch(&ctx(Some("lang = \"bad lang\""), Some(Shape::Text)))
            .await
            .unwrap_err();
        assert!(matches!(
            &err,
            FetchError::Failed(message) if message.contains("wikipedia request failed")
        ));
    }
}
