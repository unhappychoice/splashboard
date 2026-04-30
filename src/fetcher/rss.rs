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
use crate::time as t;

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
    fn description(&self) -> &'static str {
        "Generic feed reader for any RSS 2.0 / RSS 1.0 / Atom / JSON Feed URL, parsed via `feed-rs`. Each row shows the entry's published date and title, optionally linked. Trust-gated because the URL is config-controlled."
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
            ctx.timezone.as_deref(),
            ctx.locale.as_deref(),
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

fn render_body(
    feed: &Feed,
    count: usize,
    shape: Shape,
    timezone: Option<&str>,
    locale: Option<&str>,
) -> Body {
    let entries: Vec<&Entry> = feed.entries.iter().take(count).collect();
    match shape {
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: entries
                .iter()
                .map(|e| line_text(e, timezone, locale))
                .collect(),
        }),
        _ => Body::LinkedTextBlock(LinkedTextBlockData {
            items: entries
                .iter()
                .map(|e| LinkedLine {
                    text: line_text(e, timezone, locale),
                    url: link_for(e),
                })
                .collect(),
        }),
    }
}

fn line_text(entry: &Entry, timezone: Option<&str>, locale: Option<&str>) -> String {
    let date = date_label(entry, timezone, locale);
    let title = title_or_placeholder(entry);
    if date.is_empty() {
        title
    } else {
        format!("{date}  {title}")
    }
}

fn date_label(entry: &Entry, timezone: Option<&str>, locale: Option<&str>) -> String {
    entry
        .published
        .or(entry.updated)
        .map(|dt| format_short(dt, timezone, locale))
        .unwrap_or_default()
}

fn format_short(dt: DateTime<Utc>, timezone: Option<&str>, locale: Option<&str>) -> String {
    let local = match t::parse_tz(timezone) {
        Some(tz) => dt.with_timezone(&tz).fixed_offset(),
        None => dt.with_timezone(&chrono::Local).fixed_offset(),
    };
    t::format_local(&local, "%b %d", locale)
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
    // Atom entries can carry multiple <link> with `rel` discriminators (`alternate`, `self`,
    // `enclosure`, `via`, …). Per RFC 4287 the article URL is `rel="alternate"`, which is also
    // the default when `rel` is absent — matching RSS 2.0's single-link case. Picking the first
    // non-empty href would otherwise grab `rel="self"` (a self-pointer back into the feed) on
    // common Atom shapes.
    let alternate = entry
        .links
        .iter()
        .find(|l| !l.href.is_empty() && matches!(l.rel.as_deref(), None | Some("alternate")));
    alternate
        .or_else(|| entry.links.iter().find(|l| !l.href.is_empty()))
        .map(|l| l.href.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::future::Future;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn ctx(shape: Option<Shape>, options: Option<toml::Value>) -> FetchContext {
        FetchContext {
            widget_id: "w".into(),
            timeout: Duration::from_secs(1),
            shape,
            options,
            ..Default::default()
        }
    }

    fn run_async<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn serve_once(
        status: &str,
        content_type: &str,
        body: &[u8],
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let status = status.to_owned();
        let content_type = content_type.to_owned();
        let body = body.to_vec();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request);
            let header = format!(
                "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
            stream.flush().unwrap();
        });
        (format!("http://{addr}"), handle)
    }

    fn assert_failed_contains(err: FetchError, needle: &str) {
        match err {
            FetchError::Failed(message) => assert!(message.contains(needle), "msg: {message}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

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

    #[test]
    fn invalid_url_syntax_is_error() {
        let err = validated_url(Some("http://[::1")).unwrap_err();
        assert_failed_contains(err, "invalid url");
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
        let body = render_body(&feed, 5, Shape::LinkedTextBlock, None, None);
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
        let body = render_body(&feed, 5, Shape::TextBlock, None, None);
        let Body::TextBlock(t) = body else {
            panic!("expected text block");
        };
        assert_eq!(t.lines.len(), 2);
        assert!(t.lines[0].contains("First post"));
    }

    #[test]
    fn atom_parses_with_link_and_date() {
        let feed = parse(ATOM_FIXTURE);
        let body = render_body(&feed, 5, Shape::LinkedTextBlock, None, None);
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
    fn line_text_without_date_uses_title_only() {
        let mut feed = parse(RSS_FIXTURE);
        feed.entries[0].published = None;
        feed.entries[0].updated = None;
        let text = line_text(&feed.entries[0], None, None);
        assert_eq!(text, "First post");
    }

    #[test]
    fn format_short_uses_explicit_timezone() {
        let dt = Utc.with_ymd_and_hms(2026, 4, 30, 23, 30, 0).unwrap();
        assert_eq!(format_short(dt, Some("UTC"), None), "Apr 30");
        assert_eq!(format_short(dt, Some("Asia/Tokyo"), None), "May 01");
    }

    #[test]
    fn count_caps_entries() {
        let feed = parse(RSS_FIXTURE);
        let body = render_body(&feed, 1, Shape::LinkedTextBlock, None, None);
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
        let body = render_body(&empty, 5, Shape::LinkedTextBlock, None, None);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked text block");
        };
        assert!(b.items.is_empty());
    }

    #[test]
    fn missing_title_falls_back_to_placeholder() {
        let mut feed = parse(RSS_FIXTURE);
        feed.entries[0].title = None;
        let body = render_body(&feed, 1, Shape::TextBlock, None, None);
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
    fn fetcher_exposes_catalog_contract() {
        let fetcher = RssFetcher;
        assert_eq!(fetcher.name(), "rss");
        assert_eq!(fetcher.safety(), Safety::Network);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(
            fetcher
                .option_schemas()
                .iter()
                .map(|schema| schema.name)
                .collect::<Vec<_>>(),
            vec!["url", "count"]
        );
        assert!(fetcher.description().contains("feed-rs"));
    }

    #[test]
    fn whitespace_only_titles_fall_back_to_placeholder() {
        let mut feed = parse(RSS_FIXTURE);
        feed.entries[0].title.as_mut().unwrap().content = " \n\t ".into();
        let body = render_body(&feed, 1, Shape::TextBlock, None, None);
        let Body::TextBlock(t) = body else {
            panic!("expected text block");
        };
        assert!(t.lines[0].contains("(no title)"));
    }

    #[test]
    fn fetch_rejects_unknown_options_before_network() {
        let raw: toml::Value =
            toml::from_str("url = \"https://example.com\"\nbogus = true").unwrap();
        let err = run_async(RssFetcher.fetch(&ctx(None, Some(raw)))).unwrap_err();
        assert_failed_contains(err, "unknown field");
    }

    #[test]
    fn fetch_rejects_invalid_url_before_network() {
        let raw: toml::Value = toml::from_str("url = \"ftp://example.com/feed.xml\"").unwrap();
        let err = run_async(RssFetcher.fetch(&ctx(None, Some(raw)))).unwrap_err();
        assert_failed_contains(err, "unsupported url scheme");
    }

    #[test]
    fn fetch_parses_remote_feed_and_uses_default_linked_shape() {
        let (url, server) = serve_once("200 OK", "application/rss+xml", RSS_FIXTURE.as_bytes());
        let raw: toml::Value = toml::from_str(&format!("url = \"{url}\"")).unwrap();
        let payload = run_async(RssFetcher.fetch(&ctx(None, Some(raw)))).unwrap();
        server.join().unwrap();
        let Body::LinkedTextBlock(body) = payload.body else {
            panic!("expected linked text block");
        };
        assert_eq!(body.items.len(), 2);
        assert_eq!(body.items[1].url.as_deref(), Some("https://example.com/2"));
    }

    #[test]
    fn fetch_surfaces_parse_errors_from_non_feed_body() {
        let (url, server) = serve_once("200 OK", "text/plain", b"not a feed");
        let raw: toml::Value = toml::from_str(&format!("url = \"{url}\"")).unwrap();
        let err = run_async(RssFetcher.fetch(&ctx(None, Some(raw)))).unwrap_err();
        server.join().unwrap();
        assert_failed_contains(err, "rss parse");
    }

    #[test]
    fn fetch_bytes_reports_http_status_and_size_cap() {
        let (status_url, status_server) =
            serve_once("500 Internal Server Error", "text/plain", b"oops");
        let status_err =
            run_async(fetch_bytes(&validated_url(Some(&status_url)).unwrap())).unwrap_err();
        status_server.join().unwrap();
        assert_failed_contains(status_err, "rss 500");

        let oversized = vec![b'x'; MAX_BYTES + 1];
        let (size_url, size_server) = serve_once("200 OK", "application/octet-stream", &oversized);
        let size_err =
            run_async(fetch_bytes(&validated_url(Some(&size_url)).unwrap())).unwrap_err();
        size_server.join().unwrap();
        assert_failed_contains(size_err, "too large");
    }

    /// Atom feeds frequently carry both `rel="self"` (feed-internal permalink) and
    /// `rel="alternate"` (the article URL). Picking the first non-empty link would surface the
    /// self-pointer; readers want the alternate.
    const ATOM_MULTI_REL_FIXTURE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>multi rel</title>
  <id>urn:atom:m</id>
  <updated>2026-04-26T10:00:00Z</updated>
  <entry>
    <title>Pick the article URL</title>
    <id>urn:atom:m:1</id>
    <link rel="self" href="https://feed.example.com/entries/1"/>
    <link rel="alternate" href="https://example.com/articles/1"/>
    <updated>2026-04-26T10:00:00Z</updated>
  </entry>
</feed>"#;

    #[test]
    fn link_prefers_alternate_over_self_in_atom() {
        let feed = parse(ATOM_MULTI_REL_FIXTURE);
        let body = render_body(&feed, 5, Shape::LinkedTextBlock, None, None);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked text block");
        };
        assert_eq!(
            b.items[0].url.as_deref(),
            Some("https://example.com/articles/1")
        );
    }

    #[test]
    fn link_falls_back_to_first_non_empty_href_when_no_alternate_exists() {
        let mut feed = parse(ATOM_FIXTURE);
        feed.entries[0].links[0].rel = Some("self".into());
        assert_eq!(
            link_for(&feed.entries[0]).as_deref(),
            Some("https://example.com/atom-1")
        );
    }

    #[test]
    fn fetch_bytes_reports_request_failure_for_unreachable_host() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/feed.xml", listener.local_addr().unwrap());
        drop(listener);

        let err = run_async(fetch_bytes(&validated_url(Some(&url)).unwrap())).unwrap_err();
        assert_failed_contains(err, "rss request failed");
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
        let body = render_body(&feed, 3, Shape::LinkedTextBlock, None, None);
        if let Body::LinkedTextBlock(b) = body {
            for it in &b.items {
                eprintln!("{}  -> {:?}", it.text, it.url);
            }
        }
    }
}
