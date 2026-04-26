//! `rss` — generic feed fetcher (RSS 2.0 / RSS 1.0 / Atom / JSON Feed via `feed-rs`).
//!
//! Safety::Network — the URL is config-provided, so the destination is whatever the user (or a
//! local-config they trust) types. Local configs must be `splashboard trust`-ed before this
//! widget runs; global config is implicitly trusted.
//!
//! Output shapes: `LinkedTextBlock` (default — clickable rows for `list_links`) and `TextBlock`
//! (titles only). Lines are `MMM DD  Article title`, mirroring `hackernews_top`'s
//! `234pt 56c  Title` rhythm so the same renderer slot stays visually consistent.

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use feed_rs::model::{Entry, Feed};
use feed_rs::parser;
use reqwest::Client;
use serde::Deserialize;
use url::Url;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, LinkedLine, LinkedTextBlockData, Payload, TextBlockData};
use crate::render::Shape;
use crate::samples;

const DEFAULT_COUNT: u32 = 5;
const MIN_COUNT: u32 = 1;
const MAX_COUNT: u32 = 20;

/// Cap raw response bytes so a hostile / runaway feed can't OOM the daemon.
const MAX_BYTES: usize = 5 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));

const SHAPES: &[Shape] = &[Shape::LinkedTextBlock, Shape::TextBlock];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "url",
        type_hint: "string (http / https)",
        required: true,
        default: None,
        description: "Feed URL — RSS 2.0, RSS 1.0, Atom, or JSON Feed.",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=20)",
        required: false,
        default: Some("5"),
        description: "Number of feed entries to display.",
    },
];

pub struct RssFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub count: Option<u32>,
}

#[async_trait]
impl Fetcher for RssFetcher {
    fn name(&self) -> &str {
        "rss"
    }
    fn safety(&self) -> Safety {
        Safety::Network
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
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                ("Apr 26  Why X over Y", Some("https://example.com/post-1")),
                (
                    "Apr 24  Release notes 0.42",
                    Some("https://example.com/post-2"),
                ),
                ("Apr 22  A short note", Some("https://example.com/post-3")),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "Apr 26  Why X over Y",
                "Apr 24  Release notes 0.42",
                "Apr 22  A short note",
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let url = validated_url(opts.url.as_deref())?;
        let count = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_COUNT) as usize;
        let bytes = fetch_bytes(&url).await?;
        let feed = parser::parse(bytes.as_slice())
            .map_err(|e| FetchError::Failed(format!("rss parse: {e}")))?;
        Ok(payload(render_body(
            &feed,
            count,
            ctx.shape.unwrap_or(Shape::LinkedTextBlock),
        )))
    }
}

fn validated_url(raw: Option<&str>) -> Result<Url, FetchError> {
    let raw = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| FetchError::Failed("rss: option `url` is required".into()))?;
    let parsed =
        Url::parse(raw).map_err(|e| FetchError::Failed(format!("rss: invalid url: {e}")))?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed),
        other => Err(FetchError::Failed(format!(
            "rss: unsupported url scheme `{other}` (only http/https allowed)"
        ))),
    }
}

fn http() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .gzip(true)
            .build()
            .expect("reqwest client should build with default config")
    })
}

async fn fetch_bytes(url: &Url) -> Result<Vec<u8>, FetchError> {
    let res = http()
        .get(url.as_str())
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("rss request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("rss {status}")));
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("rss read body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "rss response too large ({} bytes, cap {MAX_BYTES})",
            bytes.len()
        )));
    }
    Ok(bytes.to_vec())
}

fn render_body(feed: &Feed, count: usize, shape: Shape) -> Body {
    let entries: Vec<&Entry> = feed.entries.iter().take(count).collect();
    match shape {
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: entries.iter().map(|e| line_text(e)).collect(),
        }),
        _ => Body::LinkedTextBlock(LinkedTextBlockData {
            items: entries
                .iter()
                .map(|e| LinkedLine {
                    text: line_text(e),
                    url: link_for(e),
                })
                .collect(),
        }),
    }
}

fn line_text(entry: &Entry) -> String {
    let date = date_label(entry);
    let title = title_or_placeholder(entry);
    if date.is_empty() {
        title
    } else {
        format!("{date}  {title}")
    }
}

fn date_label(entry: &Entry) -> String {
    entry
        .published
        .or(entry.updated)
        .map(format_short)
        .unwrap_or_default()
}

fn format_short(dt: DateTime<Utc>) -> String {
    dt.format("%b %d").to_string()
}

fn title_or_placeholder(entry: &Entry) -> String {
    entry
        .title
        .as_ref()
        .map(|t| collapse_whitespace(&t.content))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(no title)".into())
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn link_for(entry: &Entry) -> Option<String> {
    entry
        .links
        .iter()
        .find(|l| !l.href.is_empty())
        .map(|l| l.href.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_default_to_none() {
        let opts = Options::default();
        assert!(opts.url.is_none());
        assert!(opts.count.is_none());
    }

    #[test]
    fn options_deserialize_url_and_count() {
        let raw: toml::Value =
            toml::from_str("url = \"https://example.com/feed.xml\"\ncount = 7").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.url.as_deref(), Some("https://example.com/feed.xml"));
        assert_eq!(opts.count, Some(7));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("url = \"https://e.com\"\nbogus = 1").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn missing_url_is_error() {
        let err = validated_url(None).unwrap_err();
        match err {
            FetchError::Failed(m) => assert!(m.contains("required"), "msg: {m}"),
            _ => panic!("expected Failed"),
        }
    }

    #[test]
    fn empty_url_is_error() {
        assert!(validated_url(Some("   ")).is_err());
    }

    #[test]
    fn rejects_non_http_schemes() {
        for bad in ["file:///etc/passwd", "ftp://x", "data:,abc", "javascript:0"] {
            let err = validated_url(Some(bad)).unwrap_err();
            match err {
                FetchError::Failed(m) => assert!(
                    m.contains("scheme") || m.contains("invalid url"),
                    "expected scheme rejection for {bad}: {m}"
                ),
                _ => panic!("expected Failed for {bad}"),
            }
        }
    }

    #[test]
    fn accepts_http_and_https() {
        assert!(validated_url(Some("http://example.com/feed")).is_ok());
        assert!(validated_url(Some("https://example.com/feed")).is_ok());
    }

    const RSS_FIXTURE: &str = r#"<?xml version="1.0"?>
<rss version="2.0">
  <channel>
    <title>Example</title>
    <link>https://example.com</link>
    <description>x</description>
    <item>
      <title>First post</title>
      <link>https://example.com/1</link>
      <pubDate>Sun, 26 Apr 2026 09:00:00 +0000</pubDate>
    </item>
    <item>
      <title>Second post</title>
      <link>https://example.com/2</link>
      <pubDate>Fri, 24 Apr 2026 11:30:00 +0000</pubDate>
    </item>
  </channel>
</rss>"#;

    const ATOM_FIXTURE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Atom Example</title>
  <id>urn:atom:1</id>
  <updated>2026-04-26T10:00:00Z</updated>
  <entry>
    <title>Atom one</title>
    <id>urn:atom:1:1</id>
    <link href="https://example.com/atom-1"/>
    <updated>2026-04-26T10:00:00Z</updated>
  </entry>
</feed>"#;

    fn parse(fixture: &str) -> Feed {
        parser::parse(fixture.as_bytes()).expect("fixture must parse")
    }

    #[test]
    fn rss_parses_into_linked_text_block_with_dates_and_urls() {
        let feed = parse(RSS_FIXTURE);
        let body = render_body(&feed, 5, Shape::LinkedTextBlock);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked text block");
        };
        assert_eq!(b.items.len(), 2);
        assert!(b.items[0].text.contains("First post"));
        assert!(b.items[0].text.starts_with("Apr 26"));
        assert_eq!(b.items[0].url.as_deref(), Some("https://example.com/1"));
    }

    #[test]
    fn rss_parses_into_text_block_without_urls() {
        let feed = parse(RSS_FIXTURE);
        let body = render_body(&feed, 5, Shape::TextBlock);
        let Body::TextBlock(t) = body else {
            panic!("expected text block");
        };
        assert_eq!(t.lines.len(), 2);
        assert!(t.lines[0].contains("First post"));
    }

    #[test]
    fn atom_parses_with_link_and_date() {
        let feed = parse(ATOM_FIXTURE);
        let body = render_body(&feed, 5, Shape::LinkedTextBlock);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked text block");
        };
        assert_eq!(b.items.len(), 1);
        assert_eq!(
            b.items[0].url.as_deref(),
            Some("https://example.com/atom-1")
        );
        assert!(b.items[0].text.contains("Atom one"));
    }

    #[test]
    fn count_caps_entries() {
        let feed = parse(RSS_FIXTURE);
        let body = render_body(&feed, 1, Shape::LinkedTextBlock);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked text block");
        };
        assert_eq!(b.items.len(), 1);
    }

    #[test]
    fn empty_feed_renders_empty_body() {
        let empty = Feed {
            feed_type: feed_rs::model::FeedType::RSS2,
            id: String::new(),
            updated: None,
            authors: vec![],
            title: None,
            description: None,
            links: vec![],
            categories: vec![],
            contributors: vec![],
            generator: None,
            icon: None,
            language: None,
            logo: None,
            published: None,
            rating: None,
            rights: None,
            ttl: None,
            entries: vec![],
        };
        let body = render_body(&empty, 5, Shape::LinkedTextBlock);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked text block");
        };
        assert!(b.items.is_empty());
    }

    #[test]
    fn missing_title_falls_back_to_placeholder() {
        let mut feed = parse(RSS_FIXTURE);
        feed.entries[0].title = None;
        let body = render_body(&feed, 1, Shape::TextBlock);
        let Body::TextBlock(t) = body else {
            panic!("expected text block");
        };
        assert!(t.lines[0].contains("(no title)"));
    }

    #[test]
    fn collapses_whitespace_in_titles() {
        // RSS titles sometimes have embedded newlines / runs of spaces — render on one line.
        assert_eq!(
            collapse_whitespace("hello\n  world\t!"),
            "hello world !".to_string()
        );
    }

    #[test]
    fn count_clamps_extremes() {
        assert_eq!(0u32.clamp(MIN_COUNT, MAX_COUNT), MIN_COUNT);
        assert_eq!(999u32.clamp(MIN_COUNT, MAX_COUNT), MAX_COUNT);
        assert_eq!(DEFAULT_COUNT.clamp(MIN_COUNT, MAX_COUNT), DEFAULT_COUNT);
    }

    #[test]
    fn cache_key_varies_with_url() {
        let f = RssFetcher;
        let parse_opts =
            |raw: &str| -> toml::Value { toml::from_str(raw).expect("test toml must parse") };
        let mut a = FetchContext {
            widget_id: "w".into(),
            ..Default::default()
        };
        let mut b = a.clone();
        a.options = Some(parse_opts("url = \"https://a.example/feed\""));
        b.options = Some(parse_opts("url = \"https://b.example/feed\""));
        assert_ne!(f.cache_key(&a), f.cache_key(&b));
    }

    #[test]
    fn sample_body_covers_both_shapes() {
        let f = RssFetcher;
        assert!(matches!(
            f.sample_body(Shape::LinkedTextBlock),
            Some(Body::LinkedTextBlock(_))
        ));
        assert!(matches!(
            f.sample_body(Shape::TextBlock),
            Some(Body::TextBlock(_))
        ));
        assert!(f.sample_body(Shape::Text).is_none());
    }

    #[test]
    fn announces_network_safety() {
        assert_eq!(RssFetcher.safety(), Safety::Network);
    }

    /// Live smoke test — hits the Rust blog's feed. `#[ignore]` keeps CI offline-safe; run with
    /// `cargo test -- --ignored fetcher::rss::tests::live` to verify the real fetch path.
    #[tokio::test]
    #[ignore]
    async fn live_rust_blog_feed_returns_at_least_one_entry() {
        let url = validated_url(Some("https://blog.rust-lang.org/feed.xml")).unwrap();
        let bytes = fetch_bytes(&url).await.unwrap();
        let feed = parser::parse(bytes.as_slice()).unwrap();
        assert!(!feed.entries.is_empty());
        let body = render_body(&feed, 3, Shape::LinkedTextBlock);
        if let Body::LinkedTextBlock(b) = body {
            for it in &b.items {
                eprintln!("{}  -> {:?}", it.text, it.url);
            }
        }
    }
}
