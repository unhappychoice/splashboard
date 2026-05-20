//! `wikipedia_trending` — most-viewed Wikipedia articles for the previous UTC day via the
//! Wikimedia Pageviews API.
//!
//! Safety::Safe: requests target two fixed hosts — `wikimedia.org` for the pageviews list and
//! `<lang>.wikipedia.org` for per-article summary lookups when the `ImageLinkedList` shape
//! needs thumbnails. The user-supplied `lang` only swaps the language subdomain; it can't
//! redirect traffic off the wiki ecosystem.
//!
//! The pageviews response is filtered to drop `Main_Page`, search pages, and meta-namespace
//! entries (`Wikipedia:`, `Portal:`, `File:`, `Category:`, …) so the trending list shows real
//! articles a reader would actually click. `date` defaults to "yesterday in UTC" because the
//! API serves complete days only and today's snapshot is not yet finalised.

use std::path::PathBuf;

use async_trait::async_trait;
use chrono::{Datelike, Duration, Utc};
use serde::Deserialize;
use url::Url;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::thumbnails;
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, ImageLinkedItem, ImageLinkedListData, LinkedLine,
    LinkedTextBlockData, MarkdownTextBlockData, Payload, TextBlockData, TextData,
};
use crate::render::Shape;
use crate::samples;

use super::client::{DEFAULT_LANG, PageSummary, get, rest_api_base};

const DEFAULT_COUNT: u32 = 10;
const MIN_COUNT: u32 = 1;
const MAX_COUNT: u32 = 20;

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::ImageLinkedList,
    Shape::TextBlock,
    Shape::Text,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "lang",
        type_hint: "string (Wikipedia language code)",
        required: false,
        default: Some("\"en\""),
        description: "Wikipedia language edition.",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=20)",
        required: false,
        default: Some("10"),
        description: "Number of trending articles to display.",
    },
    OptionSchema {
        name: "access",
        type_hint: "\"all-access\" | \"desktop\" | \"mobile-web\" | \"mobile-app\"",
        required: false,
        default: Some("\"all-access\""),
        description: "Which pageview channel to rank by.",
    },
];

pub struct WikipediaTrendingFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    count: Option<u32>,
    #[serde(default)]
    access: Option<Access>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Access {
    /// Default; named `All` instead of `AllAccess` to satisfy clippy's
    /// `enum_variant_names` lint. Serde rename keeps the user-facing TOML value
    /// `"all-access"` matching the Wikimedia API path segment.
    #[default]
    #[serde(rename = "all-access")]
    All,
    Desktop,
    MobileWeb,
    MobileApp,
}

impl Access {
    fn as_path(self) -> &'static str {
        match self {
            Self::All => "all-access",
            Self::Desktop => "desktop",
            Self::MobileWeb => "mobile-web",
            Self::MobileApp => "mobile-app",
        }
    }
}

#[async_trait]
impl Fetcher for WikipediaTrendingFetcher {
    fn name(&self) -> &str {
        "wikipedia_trending"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Most-viewed Wikipedia articles for the previous UTC day via the public Wikimedia pageviews API. Filters out `Main_Page`, search, and meta-namespace pages so the list is real articles. Companion to the shipped `wikipedia_featured` (today's curated pick) and `wikipedia_random`."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60 * 6
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
        sample_trending_body(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let lang = opts.lang.as_deref().unwrap_or(DEFAULT_LANG);
        let count = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_COUNT) as usize;
        let access = opts.access.unwrap_or_default();
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);

        let articles = fetch_trending_articles(lang, access, count).await?;
        let body = render_for_shape(&articles, shape, lang).await;
        Ok(payload(body))
    }
}

/// Trending article as the renderer wants it — title display form, source page URL, view
/// count. Thumbnail URL is resolved later (only when the shape needs it) via a per-article
/// `/page/summary` call.
#[derive(Debug, Clone)]
struct Article {
    title_display: String,
    title_raw: String,
    url: String,
    views: u64,
}

async fn fetch_trending_articles(
    lang: &str,
    access: Access,
    limit: usize,
) -> Result<Vec<Article>, FetchError> {
    let (year, month, day) = previous_utc_day();
    let url = format!(
        "https://wikimedia.org/api/rest_v1/metrics/pageviews/top/{lang}.wikipedia/{}/{year}/{month:02}/{day:02}",
        access.as_path(),
    );
    let response: TopResponse = get(&url).await?;
    Ok(response
        .items
        .into_iter()
        .flat_map(|item| item.articles)
        .filter(is_real_article)
        .take(limit)
        .map(|raw| article_from_raw(lang, raw))
        .collect())
}

fn previous_utc_day() -> (i32, u32, u32) {
    let yesterday = Utc::now() - Duration::days(1);
    (yesterday.year(), yesterday.month(), yesterday.day())
}

fn is_real_article(raw: &TopArticle) -> bool {
    let title = raw.article.as_str();
    if title == "Main_Page" || title == "-" {
        return false;
    }
    !title.starts_with("Special:")
        && !title.starts_with("Wikipedia:")
        && !title.starts_with("Portal:")
        && !title.starts_with("File:")
        && !title.starts_with("Category:")
        && !title.starts_with("Help:")
        && !title.starts_with("Template:")
        && !title.starts_with("Talk:")
}

fn article_from_raw(lang: &str, raw: TopArticle) -> Article {
    let title_display = raw.article.replace('_', " ");
    let url = build_article_url(lang, &raw.article);
    Article {
        title_display,
        title_raw: raw.article,
        url,
        views: raw.views,
    }
}

/// Build a `https://{lang}.wikipedia.org/wiki/{title}` URL. Goes through `url::Url` so reserved
/// path characters (`#`, `?`, …) and non-ASCII bytes are percent-encoded correctly; underscores
/// (Wikipedia's canonical title separator) pass through unchanged.
fn build_article_url(lang: &str, title_raw: &str) -> String {
    let mut url = Url::parse(&format!("https://{lang}.wikipedia.org/wiki/"))
        .expect("wiki article base parses");
    url.path_segments_mut()
        .expect("wikipedia URL is path-segments-capable")
        .pop_if_empty()
        .push(title_raw);
    url.into()
}

async fn render_for_shape(articles: &[Article], shape: Shape, lang: &str) -> Body {
    if matches!(shape, Shape::ImageLinkedList) {
        let paths = resolve_thumbnails(articles, lang).await;
        return image_linked_body(articles, &paths);
    }
    render_sync(articles, shape)
}

async fn resolve_thumbnails(articles: &[Article], lang: &str) -> Vec<Option<PathBuf>> {
    let urls = collect_thumbnail_urls(articles, lang).await;
    thumbnails::download_many(&urls).await
}

async fn collect_thumbnail_urls(articles: &[Article], lang: &str) -> Vec<Option<String>> {
    let base = rest_api_base(lang);
    let mut out = Vec::with_capacity(articles.len());
    for article in articles {
        out.push(fetch_thumbnail_url(&base, &article.title_raw).await);
    }
    out
}

async fn fetch_thumbnail_url(rest_base: &str, title_raw: &str) -> Option<String> {
    let url = build_summary_url(rest_base, title_raw);
    let summary: PageSummary = get(&url).await.ok()?;
    summary.thumbnail_url().map(str::to_string)
}

fn build_summary_url(rest_base: &str, title_raw: &str) -> String {
    let mut url =
        Url::parse(&format!("{rest_base}/page/summary/")).expect("wiki summary base parses");
    url.path_segments_mut()
        .expect("wikipedia URL is path-segments-capable")
        .pop_if_empty()
        .push(title_raw);
    url.into()
}

fn render_sync(articles: &[Article], shape: Shape) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: headline(articles),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: articles.iter().map(article_line).collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: markdown_body(articles),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: articles
                .iter()
                .map(|a| Entry {
                    key: a.title_display.clone(),
                    value: Some(format_views(a.views)),
                    status: None,
                })
                .collect(),
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: articles
                .iter()
                .map(|a| Bar {
                    label: a.title_display.clone(),
                    value: a.views,
                    value_label: None,
                })
                .collect(),
        }),
        _ => Body::LinkedTextBlock(LinkedTextBlockData {
            items: articles
                .iter()
                .map(|a| LinkedLine {
                    text: format!("{}  {}", format_views(a.views), a.title_display),
                    url: Some(a.url.clone()),
                })
                .collect(),
        }),
    }
}

fn image_linked_body(articles: &[Article], paths: &[Option<PathBuf>]) -> Body {
    Body::ImageLinkedList(ImageLinkedListData {
        items: articles
            .iter()
            .enumerate()
            .map(|(i, a)| ImageLinkedItem {
                title: a.title_display.clone(),
                url: Some(a.url.clone()),
                thumbnail_path: paths
                    .get(i)
                    .and_then(|p| p.as_ref())
                    .map(|p| p.to_string_lossy().into_owned()),
                subtitle: Some(format_views(a.views)),
            })
            .collect(),
    })
}

fn headline(articles: &[Article]) -> String {
    articles
        .first()
        .map(|a| format!("{}  {}", format_views(a.views), a.title_display))
        .unwrap_or_else(|| "(no trending articles)".into())
}

fn article_line(article: &Article) -> String {
    format!("{}  {}", format_views(article.views), article.title_display)
}

fn markdown_body(articles: &[Article]) -> String {
    articles
        .iter()
        .map(|a| format!("- **{}** — {}", a.title_display, format_views(a.views)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_views(views: u64) -> String {
    if views >= 1_000_000 {
        format!("{:.1}M views", views as f64 / 1_000_000.0)
    } else if views >= 1_000 {
        format!("{:.1}k views", views as f64 / 1_000.0)
    } else {
        format!("{views} views")
    }
}

fn sample_trending_body(shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::LinkedTextBlock => samples::linked_text_block(&[
            (
                "412.0k views  Quokka",
                Some("https://en.wikipedia.org/wiki/Quokka"),
            ),
            (
                "287.0k views  Apollo 11",
                Some("https://en.wikipedia.org/wiki/Apollo_11"),
            ),
        ]),
        Shape::ImageLinkedList => samples::image_linked_list(&[
            (
                "Quokka",
                Some("https://en.wikipedia.org/wiki/Quokka"),
                None,
                Some("412.0k views"),
            ),
            (
                "Apollo 11",
                Some("https://en.wikipedia.org/wiki/Apollo_11"),
                None,
                Some("287.0k views"),
            ),
        ]),
        Shape::TextBlock => {
            samples::text_block(&["412.0k views  Quokka", "287.0k views  Apollo 11"])
        }
        Shape::Text => samples::text("412.0k views  Quokka"),
        Shape::MarkdownTextBlock => {
            samples::markdown("- **Quokka** — 412.0k views\n- **Apollo 11** — 287.0k views")
        }
        Shape::Entries => {
            samples::entries(&[("Quokka", "412.0k views"), ("Apollo 11", "287.0k views")])
        }
        Shape::Bars => samples::bars(&[("Quokka", 412_000), ("Apollo 11", 287_000)]),
        _ => return None,
    })
}

#[derive(Debug, Deserialize)]
struct TopResponse {
    items: Vec<TopItem>,
}

#[derive(Debug, Deserialize)]
struct TopItem {
    articles: Vec<TopArticle>,
}

#[derive(Debug, Deserialize)]
struct TopArticle {
    article: String,
    views: u64,
}

#[cfg(test)]
mod tests {
    use std::time::Duration as StdDuration;

    use super::*;

    fn ctx(options: Option<&str>, shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "wiki-trending".into(),
            timeout: StdDuration::from_secs(1),
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            ..Default::default()
        }
    }

    fn article(title: &str, views: u64) -> Article {
        Article {
            title_display: title.replace('_', " "),
            title_raw: title.into(),
            url: format!("https://en.wikipedia.org/wiki/{title}"),
            views,
        }
    }

    #[test]
    fn access_as_path_covers_all_variants() {
        assert_eq!(Access::All.as_path(), "all-access");
        assert_eq!(Access::Desktop.as_path(), "desktop");
        assert_eq!(Access::MobileWeb.as_path(), "mobile-web");
        assert_eq!(Access::MobileApp.as_path(), "mobile-app");
    }

    #[test]
    fn options_default_to_none() {
        let opts = Options::default();
        assert!(opts.lang.is_none());
        assert!(opts.count.is_none());
        assert!(opts.access.is_none());
    }

    #[test]
    fn options_deserialize_full() {
        let raw: toml::Value =
            toml::from_str("lang = \"ja\"\ncount = 5\naccess = \"mobile-web\"").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.lang.as_deref(), Some("ja"));
        assert_eq!(opts.count, Some(5));
        assert!(matches!(opts.access, Some(Access::MobileWeb)));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("bogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn is_real_article_filters_meta_and_placeholders() {
        let cases = [
            ("Main_Page", false),
            ("-", false),
            ("Special:Search", false),
            ("Wikipedia:About", false),
            ("Portal:Current_events", false),
            ("File:Logo.svg", false),
            ("Category:Cats", false),
            ("Help:Contents", false),
            ("Template:Stub", false),
            ("Talk:Quokka", false),
            ("Quokka", true),
            ("Apollo_11", true),
            ("List_of_films", true),
        ];
        for (title, expected) in cases {
            let raw = TopArticle {
                article: title.into(),
                views: 1,
            };
            assert_eq!(is_real_article(&raw), expected, "title: {title}");
        }
    }

    #[test]
    fn article_from_raw_swaps_underscores_and_percent_encodes_url() {
        let a = article_from_raw(
            "en",
            TopArticle {
                article: "Apollo_11".into(),
                views: 100,
            },
        );
        assert_eq!(a.title_display, "Apollo 11");
        assert_eq!(a.url, "https://en.wikipedia.org/wiki/Apollo_11");
    }

    #[test]
    fn article_from_raw_percent_encodes_unsafe_chars() {
        let a = article_from_raw(
            "en",
            TopArticle {
                article: "Hello#world".into(),
                views: 1,
            },
        );
        assert!(a.url.ends_with("/wiki/Hello%23world"), "url: {}", a.url);
    }

    #[test]
    fn format_views_picks_unit_by_magnitude() {
        assert_eq!(format_views(412), "412 views");
        assert_eq!(format_views(1_500), "1.5k views");
        assert_eq!(format_views(412_000), "412.0k views");
        assert_eq!(format_views(2_500_000), "2.5M views");
    }

    #[test]
    fn render_sync_text_uses_first_article() {
        let body = render_sync(&[article("Quokka", 412_000)], Shape::Text);
        let Body::Text(t) = body else {
            panic!("expected text");
        };
        assert!(t.value.contains("Quokka"));
        assert!(t.value.contains("412.0k"));
    }

    #[test]
    fn render_sync_text_handles_empty_articles() {
        let body = render_sync(&[], Shape::Text);
        let Body::Text(t) = body else {
            panic!("expected text");
        };
        assert_eq!(t.value, "(no trending articles)");
    }

    #[test]
    fn render_sync_bars_carry_view_count_as_value() {
        let body = render_sync(
            &[article("Quokka", 412_000), article("Apollo_11", 287_000)],
            Shape::Bars,
        );
        let Body::Bars(b) = body else {
            panic!("expected bars");
        };
        assert_eq!(b.bars[0].label, "Quokka");
        assert_eq!(b.bars[0].value, 412_000);
        assert_eq!(b.bars[1].label, "Apollo 11");
        assert_eq!(b.bars[1].value, 287_000);
    }

    #[test]
    fn render_sync_entries_pair_title_with_view_count() {
        let body = render_sync(&[article("Quokka", 412_000)], Shape::Entries);
        let Body::Entries(e) = body else {
            panic!("expected entries");
        };
        assert_eq!(e.items[0].key, "Quokka");
        assert_eq!(e.items[0].value.as_deref(), Some("412.0k views"));
    }

    #[test]
    fn render_sync_markdown_emits_bullet_per_article() {
        let body = render_sync(
            &[article("Quokka", 412_000), article("Apollo_11", 287_000)],
            Shape::MarkdownTextBlock,
        );
        let Body::MarkdownTextBlock(m) = body else {
            panic!("expected markdown");
        };
        assert!(m.value.contains("- **Quokka**"));
        assert!(m.value.contains("- **Apollo 11**"));
    }

    #[test]
    fn render_sync_default_to_linked_text_block() {
        let body = render_sync(&[article("Quokka", 412_000)], Shape::LinkedTextBlock);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked_text_block");
        };
        assert_eq!(b.items[0].text, "412.0k views  Quokka");
        assert_eq!(
            b.items[0].url.as_deref(),
            Some("https://en.wikipedia.org/wiki/Quokka")
        );
    }

    #[test]
    fn image_linked_body_pins_paths_and_subtitle() {
        let articles = vec![article("Quokka", 412_000), article("Apollo_11", 287_000)];
        let paths = vec![Some(PathBuf::from("/tmp/q.jpg")), None];
        let body = image_linked_body(&articles, &paths);
        let Body::ImageLinkedList(data) = body else {
            panic!("expected image_linked_list");
        };
        assert_eq!(data.items.len(), 2);
        assert_eq!(data.items[0].title, "Quokka");
        assert_eq!(data.items[0].thumbnail_path.as_deref(), Some("/tmp/q.jpg"));
        assert_eq!(data.items[0].subtitle.as_deref(), Some("412.0k views"));
        assert_eq!(data.items[1].thumbnail_path, None);
    }

    #[test]
    fn previous_utc_day_returns_a_valid_date() {
        let (year, month, day) = previous_utc_day();
        assert!((1..=12).contains(&month));
        assert!((1..=31).contains(&day));
        assert!(year >= 2024);
    }

    #[test]
    fn fetcher_exposes_catalog_metadata_and_samples() {
        let fetcher = WikipediaTrendingFetcher;
        assert_eq!(fetcher.name(), "wikipedia_trending");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(
            fetcher
                .option_schemas()
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>(),
            vec!["lang", "count", "access"]
        );
        for shape in SHAPES {
            assert!(
                fetcher.sample_body(*shape).is_some(),
                "missing sample for {shape:?}",
            );
        }
        assert!(fetcher.sample_body(Shape::Ratio).is_none());
    }

    #[test]
    fn cache_key_partitions_by_shape_and_options() {
        let fetcher = WikipediaTrendingFetcher;
        let linked = fetcher.cache_key(&ctx(Some("lang = \"en\""), Some(Shape::LinkedTextBlock)));
        let bars = fetcher.cache_key(&ctx(Some("lang = \"en\""), Some(Shape::Bars)));
        let custom_lang =
            fetcher.cache_key(&ctx(Some("lang = \"ja\""), Some(Shape::LinkedTextBlock)));
        assert_ne!(linked, bars);
        assert_ne!(linked, custom_lang);
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_options_before_network() {
        let err = WikipediaTrendingFetcher
            .fetch(&ctx(Some("bogus = true"), Some(Shape::Text)))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("unknown field"));
    }

    #[test]
    fn render_sync_text_block_lists_one_article_line_each() {
        let body = render_sync(
            &[article("Quokka", 412_000), article("Apollo_11", 287_000)],
            Shape::TextBlock,
        );
        let Body::TextBlock(t) = body else {
            panic!("expected text_block");
        };
        assert_eq!(t.lines[0], "412.0k views  Quokka");
        assert_eq!(t.lines[1], "287.0k views  Apollo 11");
    }

    #[test]
    fn build_summary_url_appends_title_to_rest_base() {
        let base = rest_api_base("en");
        let url = build_summary_url(&base, "Apollo_11");
        assert_eq!(
            url,
            "https://en.wikipedia.org/api/rest_v1/page/summary/Apollo_11"
        );
    }

    #[test]
    fn build_summary_url_percent_encodes_reserved_chars() {
        let base = rest_api_base("ja");
        let url = build_summary_url(&base, "Hello#world");
        assert!(url.ends_with("/page/summary/Hello%23world"), "url: {url}");
        assert!(url.starts_with("https://ja.wikipedia.org/"), "url: {url}");
    }

    #[tokio::test]
    async fn render_for_shape_non_image_shape_renders_synchronously() {
        let body = render_for_shape(&[article("Quokka", 412_000)], Shape::Text, "en").await;
        let Body::Text(t) = body else {
            panic!("expected text");
        };
        assert!(t.value.contains("Quokka"));
        assert!(t.value.contains("412.0k"));
    }
}
