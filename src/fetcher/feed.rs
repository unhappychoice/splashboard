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
    Body, ImageLinkedItem, ImageLinkedListData, LinkedLine, LinkedTextBlockData, TextBlockData,
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
