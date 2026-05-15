//! Shared feed pipeline for `rss` and the `news_*` family.
//!
//! All three concerns of fetch-and-render — HTTP retrieval, feed-rs parsing, and entry-to-body
//! mapping — live here. Both callers re-use the same parser limits (5 MB body cap, 10 s timeout)
//! and the same row formatting (`MMM DD  Title`) so widgets stay visually consistent regardless
//! of which fetcher emits them. The `label` argument that threads through fetch / parse errors
//! is the caller's display name (`"rss"` / `"news_bbc"`), so the operator sees which feed broke.
//!
//! The only thing each caller owns separately is option parsing (rss takes `url`; news_* takes
//! `feed` as a key into a hardcoded table) and the Fetcher trait wiring.
//!
//! Constants are pub(crate) so test code in `rss.rs` can still assert against `MAX_BYTES` /
//! `DEFAULT_COUNT` etc. without re-declaring them.
use std::sync::OnceLock;
use std::time::Duration;

use chrono::{DateTime, Utc};
use feed_rs::model::{Entry, Feed};
use feed_rs::parser;
use regex::Regex;
use reqwest::Client;
use url::Url;

use crate::fetcher::thumbnails;
use crate::fetcher::{FetchContext, FetchError};
use crate::payload::{
    Body, EntriesData, Entry as PayloadEntry, ImageData, ImageLinkedItem, ImageLinkedListData,
    LinkedLine, LinkedTextBlockData, MarkdownTextBlockData, TextBlockData, TextData, TimelineData,
    TimelineEvent,
};
use crate::render::Shape;
use crate::time as t;

pub(crate) const DEFAULT_COUNT: u32 = 5;
pub(crate) const MIN_COUNT: u32 = 1;
pub(crate) const MAX_COUNT: u32 = 20;

/// Cap raw response bytes so a hostile / runaway feed can't OOM the daemon.
pub(crate) const MAX_BYTES: usize = 5 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));

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

pub(crate) async fn fetch_bytes(url: &Url, label: &str) -> Result<Vec<u8>, FetchError> {
    let res = http()
        .get(url.as_str())
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("{label} request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("{label} {status}")));
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("{label} read body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "{label} response too large ({} bytes, cap {MAX_BYTES})",
            bytes.len()
        )));
    }
    Ok(bytes.to_vec())
}

pub(crate) fn parse_feed(bytes: &[u8], label: &str) -> Result<Feed, FetchError> {
    parser::parse(bytes).map_err(|e| FetchError::Failed(format!("{label} parse: {e}")))
}

pub(crate) fn render_body(
    feed: &Feed,
    count: usize,
    shape: Shape,
    timezone: Option<&str>,
    locale: Option<&str>,
) -> Body {
    let entries: Vec<&Entry> = feed.entries.iter().take(count).collect();
    match shape {
        Shape::Text => Body::Text(TextData {
            value: entries
                .first()
                .map(|e| title_or_placeholder(e))
                .unwrap_or_default(),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: entries
                .iter()
                .map(|e| line_text(e, timezone, locale))
                .collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: entries
                .iter()
                .map(|e| markdown_line(e, timezone, locale))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: entries
                .iter()
                .map(|e| PayloadEntry {
                    key: title_or_placeholder(e),
                    value: Some(date_label(e, timezone, locale)).filter(|s| !s.is_empty()),
                    status: None,
                })
                .collect(),
        }),
        Shape::Timeline => Body::Timeline(TimelineData {
            events: entries
                .iter()
                .map(|e| TimelineEvent {
                    timestamp: e
                        .published
                        .or(e.updated)
                        .map(|d| d.timestamp())
                        .unwrap_or(0),
                    title: title_or_placeholder(e),
                    detail: link_host(e),
                    status: None,
                })
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

/// Async-only because it downloads the latest entry's thumbnail. Returns an empty `Body::Image`
/// (path "") when the feed has no entries or no resolvable thumbnail — `is_empty_body` treats
/// that as a placeholder.
pub(crate) async fn render_image_body(feed: &Feed) -> Body {
    let path = match feed.entries.first().and_then(thumbnail_url_for) {
        Some(url) => thumbnails::download_to_cache(&url)
            .await
            .ok()
            .flatten()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        None => String::new(),
    };
    Body::Image(ImageData { path })
}

fn markdown_line(entry: &Entry, timezone: Option<&str>, locale: Option<&str>) -> String {
    let date = date_label(entry, timezone, locale);
    let title = title_or_placeholder(entry);
    let labeled = if date.is_empty() {
        title
    } else {
        format!("{date}  {title}")
    };
    match link_for(entry) {
        Some(url) => format!("- [{labeled}]({url})"),
        None => format!("- {labeled}"),
    }
}

fn link_host(entry: &Entry) -> Option<String> {
    link_for(entry)
        .as_deref()
        .and_then(|u| Url::parse(u).ok())
        .and_then(|u| u.host_str().map(str::to_string))
}

pub(crate) async fn render_image_linked(feed: &Feed, count: usize, ctx: &FetchContext) -> Body {
    let entries: Vec<&Entry> = feed.entries.iter().take(count).collect();
    let thumbnail_urls: Vec<Option<String>> =
        entries.iter().map(|e| thumbnail_url_for(e)).collect();
    let thumbnail_paths = thumbnails::download_many(&thumbnail_urls).await;
    Body::ImageLinkedList(ImageLinkedListData {
        items: entries
            .iter()
            .zip(thumbnail_paths)
            .map(|(e, path)| ImageLinkedItem {
                title: title_or_placeholder(e),
                url: link_for(e),
                thumbnail_path: path.map(|p| p.to_string_lossy().into_owned()),
                subtitle: subtitle_for(e, ctx.timezone.as_deref(), ctx.locale.as_deref()),
            })
            .collect(),
    })
}

/// Best-effort image URL for the entry, in order of preference:
///
/// 1. `media:thumbnail` (RSS Media spec) — explicit thumbnail.
/// 2. `media:content` URL — full media, usually an image.
/// 3. First `<img src="http(s)://...">` in the entry's HTML `<content>` or `<summary>` —
///    covers Atom feeds without media extensions (Rust blog, most personal blogs) where the
///    cover image is inlined in the body markup.
pub(crate) fn thumbnail_url_for(entry: &Entry) -> Option<String> {
    entry
        .media
        .iter()
        .flat_map(|m| m.thumbnails.iter().map(|t| t.image.uri.clone()))
        .find(|s| !s.is_empty())
        .or_else(|| {
            entry
                .media
                .iter()
                .flat_map(|m| m.content.iter().filter_map(|c| c.url.as_ref()))
                .map(|u| u.to_string())
                .find(|s| !s.is_empty())
        })
        .or_else(|| {
            entry
                .content
                .as_ref()
                .and_then(|c| c.body.as_deref())
                .and_then(first_inline_image_src)
        })
        .or_else(|| {
            entry
                .summary
                .as_ref()
                .and_then(|s| first_inline_image_src(&s.content))
        })
}

/// Extracts the `src` attribute of the first HTTP(S) `<img>` tag in `html`. Used to recover a
/// thumbnail when the feed doesn't expose `media:thumbnail` — most Atom blogs only ship the
/// cover image inline inside the HTML body. Encoded entities (`&amp;`) are decoded back to `&`
/// so the URL works as-is.
pub(crate) fn first_inline_image_src(html: &str) -> Option<String> {
    static IMG_SRC_RE: OnceLock<Regex> = OnceLock::new();
    let re = IMG_SRC_RE
        .get_or_init(|| Regex::new(r#"(?is)<img\b[^>]*?\bsrc\s*=\s*["']([^"']+)["']"#).unwrap());
    re.captures_iter(html)
        .map(|cap| decode_html_entities(&cap[1]))
        .find(|src| src.starts_with("http://") || src.starts_with("https://"))
}

fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&#x2F;", "/")
        .replace("&#x3D;", "=")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Subtitle for the card layout: `"<date> · <source-host>"`, or just one half when the other is
/// missing.
pub(crate) fn subtitle_for(
    entry: &Entry,
    timezone: Option<&str>,
    locale: Option<&str>,
) -> Option<String> {
    let date = date_label(entry, timezone, locale);
    let host = link_for(entry)
        .as_deref()
        .and_then(|u| Url::parse(u).ok())
        .and_then(|u| u.host_str().map(str::to_string));
    match (date.is_empty(), host) {
        (true, None) => None,
        (false, None) => Some(date),
        (true, Some(h)) => Some(h),
        (false, Some(h)) => Some(format!("{date} · {h}")),
    }
}

pub(crate) fn line_text(entry: &Entry, timezone: Option<&str>, locale: Option<&str>) -> String {
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

pub(crate) fn format_short(
    dt: DateTime<Utc>,
    timezone: Option<&str>,
    locale: Option<&str>,
) -> String {
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

pub(crate) fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Atom entries can carry multiple `<link>` with `rel` discriminators (`alternate`, `self`,
/// `enclosure`, `via`, …). Per RFC 4287 the article URL is `rel="alternate"`, which is also
/// the default when `rel` is absent — matching RSS 2.0's single-link case. Picking the first
/// non-empty href would otherwise grab `rel="self"` (a self-pointer back into the feed) on
/// common Atom shapes.
pub(crate) fn link_for(entry: &Entry) -> Option<String> {
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
    use feed_rs::model::{
        Content, Entry as FeedEntry, Feed as FeedDoc, FeedType, Image, Link, MediaContent,
        MediaObject, MediaThumbnail, Text,
    };

    fn empty_feed() -> FeedDoc {
        FeedDoc {
            feed_type: FeedType::RSS2,
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
        }
    }

    fn entry_with_title_and_link(title: &str, href: &str) -> FeedEntry {
        FeedEntry {
            title: Some(Text {
                content_type: "text/plain".parse().unwrap(),
                src: None,
                content: title.into(),
            }),
            published: Some(Utc.with_ymd_and_hms(2026, 4, 26, 12, 0, 0).unwrap()),
            links: vec![Link {
                href: href.into(),
                rel: None,
                media_type: None,
                href_lang: None,
                title: None,
                length: None,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn render_body_text_uses_first_entry_title_or_empty() {
        let mut feed = empty_feed();
        feed.entries
            .push(entry_with_title_and_link("First", "https://example.com/1"));
        feed.entries
            .push(entry_with_title_and_link("Second", "https://example.com/2"));
        let body = render_body(&feed, 5, Shape::Text, Some("UTC"), None);
        let Body::Text(t) = body else {
            panic!("expected text");
        };
        assert_eq!(t.value, "First");

        let body_empty = render_body(&empty_feed(), 5, Shape::Text, None, None);
        let Body::Text(t) = body_empty else {
            panic!("expected text");
        };
        assert!(t.value.is_empty());
    }

    #[test]
    fn render_body_markdown_text_block_wraps_each_entry_with_link_syntax() {
        let mut feed = empty_feed();
        feed.entries
            .push(entry_with_title_and_link("First", "https://example.com/1"));
        feed.entries
            .push(entry_with_title_and_link("Second", "https://example.com/2"));
        let body = render_body(&feed, 5, Shape::MarkdownTextBlock, Some("UTC"), None);
        let Body::MarkdownTextBlock(md) = body else {
            panic!("expected markdown text block");
        };
        let lines: Vec<&str> = md.value.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("- ["), "got: {}", lines[0]);
        assert!(lines[0].contains("First"));
        assert!(lines[0].contains("https://example.com/1"));
        assert!(lines[0].contains("Apr 26"));
    }

    #[test]
    fn render_body_markdown_drops_link_syntax_when_no_url_and_no_date() {
        let entry = FeedEntry {
            title: Some(Text {
                content_type: "text/plain".parse().unwrap(),
                src: None,
                content: "Bare".into(),
            }),
            ..Default::default()
        };
        let mut feed = empty_feed();
        feed.entries.push(entry);
        let body = render_body(&feed, 5, Shape::MarkdownTextBlock, None, None);
        let Body::MarkdownTextBlock(md) = body else {
            panic!("expected markdown text block");
        };
        // No link wrapper, no date prefix — just the bullet + title.
        assert_eq!(md.value, "- Bare");
    }

    #[test]
    fn render_body_entries_emits_title_value_pairs_with_optional_date() {
        let mut feed = empty_feed();
        feed.entries
            .push(entry_with_title_and_link("First", "https://example.com/1"));
        let body = render_body(&feed, 5, Shape::Entries, Some("UTC"), None);
        let Body::Entries(e) = body else {
            panic!("expected entries");
        };
        assert_eq!(e.items.len(), 1);
        assert_eq!(e.items[0].key, "First");
        assert_eq!(e.items[0].value.as_deref(), Some("Apr 26"));
    }

    #[test]
    fn render_body_entries_omits_value_when_date_missing() {
        let mut feed = empty_feed();
        let mut e = entry_with_title_and_link("First", "https://example.com/1");
        e.published = None;
        e.updated = None;
        feed.entries.push(e);
        let body = render_body(&feed, 5, Shape::Entries, Some("UTC"), None);
        let Body::Entries(e) = body else {
            panic!("expected entries");
        };
        assert!(e.items[0].value.is_none());
    }

    #[test]
    fn render_body_timeline_carries_link_host_and_timestamp() {
        let mut feed = empty_feed();
        feed.entries.push(entry_with_title_and_link(
            "First",
            "https://blog.example.com/post",
        ));
        let body = render_body(&feed, 5, Shape::Timeline, None, None);
        let Body::Timeline(t) = body else {
            panic!("expected timeline");
        };
        assert_eq!(t.events.len(), 1);
        assert_eq!(t.events[0].title, "First");
        assert_eq!(t.events[0].detail.as_deref(), Some("blog.example.com"));
        // Apr 26 2026 12:00:00 UTC.
        assert_eq!(
            t.events[0].timestamp,
            Utc.with_ymd_and_hms(2026, 4, 26, 12, 0, 0)
                .unwrap()
                .timestamp(),
        );
    }

    #[test]
    fn render_body_timeline_falls_back_to_zero_timestamp_and_none_detail() {
        let entry = FeedEntry {
            title: Some(Text {
                content_type: "text/plain".parse().unwrap(),
                src: None,
                content: "Bare".into(),
            }),
            ..Default::default()
        };
        let mut feed = empty_feed();
        feed.entries.push(entry);
        let body = render_body(&feed, 5, Shape::Timeline, None, None);
        let Body::Timeline(t) = body else {
            panic!("expected timeline");
        };
        assert_eq!(t.events[0].timestamp, 0);
        assert!(t.events[0].detail.is_none());
    }

    #[test]
    fn link_for_prefers_alternate_over_self() {
        let entry = FeedEntry {
            links: vec![
                Link {
                    href: "https://example.com/feed.xml".into(),
                    rel: Some("self".into()),
                    media_type: None,
                    href_lang: None,
                    title: None,
                    length: None,
                },
                Link {
                    href: "https://example.com/post".into(),
                    rel: Some("alternate".into()),
                    media_type: None,
                    href_lang: None,
                    title: None,
                    length: None,
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            link_for(&entry).as_deref(),
            Some("https://example.com/post")
        );
    }

    #[test]
    fn link_for_falls_back_to_any_link_when_no_alternate_or_default_rel() {
        let entry = FeedEntry {
            links: vec![Link {
                href: "https://example.com/enclosure.mp3".into(),
                rel: Some("enclosure".into()),
                media_type: None,
                href_lang: None,
                title: None,
                length: None,
            }],
            ..Default::default()
        };
        assert_eq!(
            link_for(&entry).as_deref(),
            Some("https://example.com/enclosure.mp3"),
        );
    }

    #[test]
    fn link_for_returns_none_when_links_empty() {
        assert!(link_for(&FeedEntry::default()).is_none());
    }

    #[test]
    fn thumbnail_url_falls_back_to_media_content_when_no_thumbnail_set() {
        let media = MediaObject {
            title: None,
            content: vec![MediaContent {
                url: Some(Url::parse("https://example.com/full.jpg").unwrap()),
                content_type: None,
                height: None,
                width: None,
                duration: None,
                size: None,
                rating: None,
            }],
            duration: None,
            thumbnails: vec![],
            texts: vec![],
            description: None,
            community: None,
            credits: vec![],
        };
        let entry = FeedEntry {
            media: vec![media],
            ..Default::default()
        };
        assert_eq!(
            thumbnail_url_for(&entry).as_deref(),
            Some("https://example.com/full.jpg"),
        );
    }

    #[test]
    fn thumbnail_url_falls_back_to_summary_inline_img_when_content_absent() {
        let entry = FeedEntry {
            summary: Some(Text {
                content_type: "text/html".parse().unwrap(),
                src: None,
                content: r#"<img src="https://example.com/cover.png"/>"#.into(),
            }),
            ..Default::default()
        };
        assert_eq!(
            thumbnail_url_for(&entry).as_deref(),
            Some("https://example.com/cover.png"),
        );
    }

    #[test]
    fn thumbnail_url_skips_empty_media_thumbnail_uri() {
        // An entry that carries a Media block whose thumbnail URI is empty falls through to the
        // next source (in this case, content body inline img).
        let media = MediaObject {
            title: None,
            content: vec![],
            duration: None,
            thumbnails: vec![MediaThumbnail {
                image: Image {
                    uri: String::new(),
                    title: None,
                    link: None,
                    width: None,
                    height: None,
                    description: None,
                },
                time: None,
            }],
            texts: vec![],
            description: None,
            community: None,
            credits: vec![],
        };
        let entry = FeedEntry {
            media: vec![media],
            content: Some(Content {
                body: Some(r#"<img src="https://example.com/hero.png">"#.into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            thumbnail_url_for(&entry).as_deref(),
            Some("https://example.com/hero.png"),
        );
    }

    #[test]
    fn format_short_falls_back_to_local_when_timezone_is_unknown() {
        // An unparseable timezone string drops back to the local tz; we only assert the format
        // doesn't blow up and returns a non-empty short month-day label.
        let dt = Utc.with_ymd_and_hms(2026, 4, 30, 23, 30, 0).unwrap();
        let label = format_short(dt, Some("Not/A_Real_Timezone"), None);
        assert!(!label.is_empty());
    }

    #[tokio::test]
    async fn render_image_body_returns_empty_path_for_empty_feed() {
        let feed = empty_feed();
        let body = render_image_body(&feed).await;
        let Body::Image(img) = body else {
            panic!("expected image body");
        };
        assert!(img.path.is_empty());
    }

    #[tokio::test]
    async fn render_image_body_returns_empty_path_when_entry_has_no_thumbnail() {
        let mut feed = empty_feed();
        feed.entries
            .push(entry_with_title_and_link("Bare", "https://example.com/1"));
        let body = render_image_body(&feed).await;
        let Body::Image(img) = body else {
            panic!("expected image body");
        };
        assert!(img.path.is_empty());
    }

    #[tokio::test]
    async fn render_image_linked_returns_items_with_missing_thumbnails() {
        let mut feed = empty_feed();
        feed.entries
            .push(entry_with_title_and_link("First", "https://example.com/1"));
        let ctx = FetchContext {
            timezone: Some("UTC".into()),
            ..Default::default()
        };
        let body = render_image_linked(&feed, 5, &ctx).await;
        let Body::ImageLinkedList(d) = body else {
            panic!("expected image linked list");
        };
        assert_eq!(d.items.len(), 1);
        assert_eq!(d.items[0].title, "First");
        assert_eq!(d.items[0].url.as_deref(), Some("https://example.com/1"));
        // No thumbnail source on the entry → no thumbnail path on the row.
        assert!(d.items[0].thumbnail_path.is_none());
    }

    /// Helper for building a `MediaObject` whose only thumbnail is a single image URI. The
    /// surrounding fields are required by `feed_rs` but unused by `thumbnail_url_for`.
    fn entry_with_thumbnail(title: &str, href: &str, thumbnail_uri: &str) -> FeedEntry {
        let mut entry = entry_with_title_and_link(title, href);
        entry.media = vec![MediaObject {
            title: None,
            content: vec![],
            duration: None,
            thumbnails: vec![MediaThumbnail {
                image: Image {
                    uri: thumbnail_uri.into(),
                    title: None,
                    link: None,
                    width: None,
                    height: None,
                    description: None,
                },
                time: None,
            }],
            texts: vec![],
            description: None,
            community: None,
            credits: vec![],
        }];
        entry
    }

    /// SHA-256-hex of the URL bytes; matches `thumbnails::download_to_cache`'s on-disk cache key
    /// scheme so pre-seeding a cached file under `cache/thumbnails/<hash>.png` lets the helper
    /// short-circuit to the existing path without any network call.
    fn url_hash(url: &str) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(url.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn restore_splashboard_home(previous: Option<String>) {
        unsafe {
            match previous {
                Some(value) => std::env::set_var("SPLASHBOARD_HOME", value),
                None => std::env::remove_var("SPLASHBOARD_HOME"),
            }
        }
    }

    /// Pre-seeding the thumbnail cache lets `download_to_cache` short-circuit to
    /// `existing_cached`, so `render_image_body`'s `Some(url) => …` arm fires without touching
    /// the network. The `.ok()`/`.flatten()`/`.map(...)`/`.unwrap_or_default()` chain on the
    /// `Ok(Some(path))` returns the file's path string.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn render_image_body_returns_cached_thumbnail_path() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let previous = std::env::var("SPLASHBOARD_HOME").ok();
        unsafe { std::env::set_var("SPLASHBOARD_HOME", tmp.path()) };
        let url = "https://example.com/feed-thumb.png";
        let dir = tmp.path().join("cache").join("thumbnails");
        std::fs::create_dir_all(&dir).unwrap();
        let cached = dir.join(format!("{}.png", url_hash(url)));
        std::fs::write(&cached, b"pretend-png").unwrap();

        let mut feed = empty_feed();
        feed.entries
            .push(entry_with_thumbnail("Story", "https://example.com/1", url));
        let body = render_image_body(&feed).await;

        restore_splashboard_home(previous);
        assert!(matches!(
            body,
            Body::Image(img) if img.path == cached.to_string_lossy().into_owned()
        ));
    }

    /// Same cache-pre-seed trick exercises `render_image_linked`'s `path.map(|p| …)` arm —
    /// `download_many` returns `Some(cached_path)` for the seeded entry and `None` for the
    /// thumbnail-less one, pinning both row shapes in a single test.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn render_image_linked_carries_thumbnail_path_when_cached() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let previous = std::env::var("SPLASHBOARD_HOME").ok();
        unsafe { std::env::set_var("SPLASHBOARD_HOME", tmp.path()) };
        let url = "https://example.com/linked-thumb.png";
        let dir = tmp.path().join("cache").join("thumbnails");
        std::fs::create_dir_all(&dir).unwrap();
        let cached = dir.join(format!("{}.png", url_hash(url)));
        std::fs::write(&cached, b"pretend-png").unwrap();

        let mut feed = empty_feed();
        feed.entries.push(entry_with_thumbnail(
            "Hero",
            "https://example.com/hero",
            url,
        ));
        feed.entries.push(entry_with_title_and_link(
            "Bare",
            "https://example.com/bare",
        ));
        let ctx = FetchContext {
            timezone: Some("UTC".into()),
            ..Default::default()
        };
        let body = render_image_linked(&feed, 5, &ctx).await;

        restore_splashboard_home(previous);
        let cached_str = cached.to_string_lossy().into_owned();
        assert!(matches!(
            &body,
            Body::ImageLinkedList(d)
                if d.items.len() == 2
                    && d.items[0].thumbnail_path.as_deref() == Some(cached_str.as_str())
                    && d.items[1].thumbnail_path.is_none()
        ));
    }

    #[test]
    fn parse_feed_surfaces_label_in_error() {
        let err = parse_feed(b"not a feed", "rss").unwrap_err();
        match err {
            FetchError::Failed(msg) => assert!(msg.contains("rss"), "msg: {msg}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}
