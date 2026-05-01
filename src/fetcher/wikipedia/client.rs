//! Shared HTTP client + URL helpers for the `wikipedia_*` family. Targets the public
//! Wikipedia REST API at `https://{lang}.wikipedia.org/api/rest_v1/...` — host stays inside
//! `*.wikipedia.org` regardless of which language code the config supplies, so every fetcher
//! in the family is Safety::Safe.

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::fetcher::FetchError;
use crate::payload::{Body, LinkedLine, LinkedTextBlockData, TextBlockData, TextData};
use crate::render::Shape;

pub const REST_API_PATH: &str = "/api/rest_v1";
pub const DEFAULT_LANG: &str = "en";
const USER_AGENT: &str = concat!(
    "splashboard/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/unhappychoice/splashboard-2)"
);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub fn http() -> &'static Client {
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

pub fn rest_api_base(lang: &str) -> String {
    format!("https://{lang}.wikipedia.org{REST_API_PATH}")
}

pub async fn get<T: DeserializeOwned>(url: &str) -> Result<T, FetchError> {
    let res = http()
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("wikipedia request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("wikipedia {status}")));
    }
    res.json()
        .await
        .map_err(|e| FetchError::Failed(format!("wikipedia json parse: {e}")))
}

/// Page summary returned by `/page/random/summary`, embedded inside `feed/featured`'s `tfa`
/// block, and inside each on-this-day event's `pages[]`. Featured / random share the rendering
/// helpers below; on-this-day uses only `url()` to wire up event links.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PageSummary {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub extract: Option<String>,
    #[serde(default)]
    pub content_urls: Option<ContentUrls>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ContentUrls {
    #[serde(default)]
    pub desktop: Option<UrlsByPlatform>,
    #[serde(default)]
    pub mobile: Option<UrlsByPlatform>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UrlsByPlatform {
    pub page: String,
}

impl PageSummary {
    pub fn url(&self) -> Option<String> {
        self.content_urls
            .as_ref()
            .and_then(|c| c.desktop.as_ref().or(c.mobile.as_ref()))
            .map(|p| p.page.clone())
    }

    /// First sentence of the extract — split at the first `". "` so the Text shape can carry a
    /// preview without the whole multi-paragraph body. Returns `None` when the extract is
    /// missing or empty.
    pub fn first_sentence(&self) -> Option<String> {
        let extract = self.extract.as_deref()?.trim();
        if extract.is_empty() {
            return None;
        }
        let cut = extract.find(". ").map(|i| i + 1).unwrap_or(extract.len());
        Some(extract[..cut].trim().to_string())
    }
}

/// Shape-aware rendering for a single page summary — shared by `wikipedia_featured` and
/// `wikipedia_random` since both fetchers expose the same `{title, extract, url}` triplet.
pub fn render_page_summary(summary: &PageSummary, shape: Shape) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: text_line(summary),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: text_block_lines(summary),
        }),
        _ => Body::LinkedTextBlock(LinkedTextBlockData {
            items: vec![LinkedLine {
                text: summary.title.clone(),
                url: summary.url(),
            }],
        }),
    }
}

fn text_line(summary: &PageSummary) -> String {
    match summary.first_sentence() {
        Some(sentence) => format!("{}: {sentence}", summary.title),
        None => summary.title.clone(),
    }
}

fn text_block_lines(summary: &PageSummary) -> Vec<String> {
    let mut lines = vec![summary.title.clone()];
    if let Some(extract) = summary.extract.as_deref().filter(|s| !s.is_empty()) {
        for line in extract.lines().filter(|l| !l.trim().is_empty()) {
            lines.push(line.to_string());
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    fn summary_with(extract: Option<&str>, page_url: Option<&str>) -> PageSummary {
        PageSummary {
            title: "Quokka".into(),
            extract: extract.map(String::from),
            content_urls: page_url.map(|p| ContentUrls {
                desktop: Some(UrlsByPlatform { page: p.into() }),
                mobile: None,
            }),
        }
    }

    fn run_async<T>(fut: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fut)
    }

    fn serve_once(status: &str, content_type: &str, body: &str) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let body = body.to_owned();
        let content_type = content_type.to_owned();
        let status = status.to_owned();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        format!("http://{addr}/")
    }

    #[test]
    fn rest_api_base_uses_lang_subdomain() {
        assert_eq!(rest_api_base("ja"), "https://ja.wikipedia.org/api/rest_v1");
    }

    #[test]
    fn url_prefers_desktop_over_mobile() {
        let s = summary_with(None, Some("https://en.wikipedia.org/wiki/Quokka"));
        assert_eq!(
            s.url().as_deref(),
            Some("https://en.wikipedia.org/wiki/Quokka")
        );
    }

    #[test]
    fn url_falls_back_to_mobile_when_desktop_absent() {
        let s = PageSummary {
            title: "x".into(),
            extract: None,
            content_urls: Some(ContentUrls {
                desktop: None,
                mobile: Some(UrlsByPlatform {
                    page: "https://en.m.wikipedia.org/wiki/x".into(),
                }),
            }),
        };
        assert_eq!(
            s.url().as_deref(),
            Some("https://en.m.wikipedia.org/wiki/x")
        );
    }

    #[test]
    fn url_is_none_when_content_urls_absent() {
        assert!(summary_with(None, None).url().is_none());
    }

    #[test]
    fn first_sentence_splits_on_period_space() {
        let s = summary_with(Some("Apollo 11 was a mission. It landed in 1969."), None);
        assert_eq!(
            s.first_sentence().as_deref(),
            Some("Apollo 11 was a mission.")
        );
    }

    #[test]
    fn first_sentence_returns_full_extract_when_no_period_space() {
        let s = summary_with(Some("Single fragment"), None);
        assert_eq!(s.first_sentence().as_deref(), Some("Single fragment"));
    }

    #[test]
    fn first_sentence_returns_none_when_extract_missing_or_blank() {
        assert!(summary_with(None, None).first_sentence().is_none());
        assert!(summary_with(Some("   "), None).first_sentence().is_none());
    }

    #[test]
    fn render_text_combines_title_and_first_sentence() {
        let s = summary_with(Some("It hops. It is fluffy."), None);
        let body = render_page_summary(&s, Shape::Text);
        assert!(matches!(body, Body::Text(_)));
        if let Body::Text(t) = body {
            assert_eq!(t.value, "Quokka: It hops.");
        }
    }

    #[test]
    fn render_text_falls_back_to_title_when_extract_missing() {
        let body = render_page_summary(&summary_with(None, None), Shape::Text);
        assert!(matches!(body, Body::Text(_)));
        if let Body::Text(t) = body {
            assert_eq!(t.value, "Quokka");
        }
    }

    #[test]
    fn render_text_block_emits_title_then_extract_lines() {
        let s = summary_with(Some("Para one.\n\nPara two."), None);
        let body = render_page_summary(&s, Shape::TextBlock);
        assert!(matches!(body, Body::TextBlock(_)));
        if let Body::TextBlock(t) = body {
            assert_eq!(t.lines, vec!["Quokka", "Para one.", "Para two."]);
        }
    }

    #[test]
    fn render_text_block_emits_title_only_when_extract_empty() {
        let s = summary_with(Some(""), None);
        let body = render_page_summary(&s, Shape::TextBlock);
        assert!(matches!(body, Body::TextBlock(_)));
        if let Body::TextBlock(t) = body {
            assert_eq!(t.lines, vec!["Quokka"]);
        }
    }

    #[test]
    fn render_linked_text_block_uses_summary_url() {
        let s = summary_with(None, Some("https://en.wikipedia.org/wiki/Quokka"));
        let body = render_page_summary(&s, Shape::LinkedTextBlock);
        assert!(matches!(body, Body::LinkedTextBlock(_)));
        if let Body::LinkedTextBlock(b) = body {
            assert_eq!(b.items.len(), 1);
            assert_eq!(b.items[0].text, "Quokka");
            assert_eq!(
                b.items[0].url.as_deref(),
                Some("https://en.wikipedia.org/wiki/Quokka")
            );
        }
    }

    #[test]
    fn render_linked_text_block_drops_url_when_absent() {
        let body = render_page_summary(&summary_with(None, None), Shape::LinkedTextBlock);
        assert!(matches!(body, Body::LinkedTextBlock(_)));
        if let Body::LinkedTextBlock(b) = body {
            assert!(b.items[0].url.is_none());
        }
    }

    #[test]
    fn page_summary_deserializes_real_api_payload() {
        let raw = r#"{
            "title": "Quokka",
            "extract": "The quokka is a small macropod.",
            "content_urls": {
                "desktop": { "page": "https://en.wikipedia.org/wiki/Quokka" }
            }
        }"#;
        let s: PageSummary = serde_json::from_str(raw).unwrap();
        assert_eq!(s.title, "Quokka");
        assert_eq!(
            s.extract.as_deref(),
            Some("The quokka is a small macropod.")
        );
        assert_eq!(
            s.url().as_deref(),
            Some("https://en.wikipedia.org/wiki/Quokka")
        );
    }

    #[test]
    fn page_summary_deserializes_with_only_title() {
        let raw = r#"{"title":"x"}"#;
        let s: PageSummary = serde_json::from_str(raw).unwrap();
        assert_eq!(s.title, "x");
        assert!(s.extract.is_none());
        assert!(s.url().is_none());
    }

    #[test]
    fn get_deserializes_successful_json_payloads() {
        let url = serve_once(
            "200 OK",
            "application/json",
            r#"{"title":"Quokka","extract":"The quokka is a small macropod."}"#,
        );

        let summary = run_async(get::<PageSummary>(&url)).unwrap();

        assert_eq!(summary.title, "Quokka");
        assert_eq!(
            summary.extract.as_deref(),
            Some("The quokka is a small macropod.")
        );
    }

    #[test]
    fn get_rejects_non_success_statuses() {
        let url = serve_once("503 Service Unavailable", "text/plain", "busy");

        let err = run_async(get::<PageSummary>(&url)).unwrap_err();

        assert!(matches!(
            err,
            FetchError::Failed(message) if message.contains("wikipedia 503")
        ));
    }

    #[test]
    fn get_rejects_invalid_json_payloads() {
        let url = serve_once("200 OK", "application/json", "{\"title\":");

        let err = run_async(get::<PageSummary>(&url)).unwrap_err();

        assert!(matches!(
            err,
            FetchError::Failed(message) if message.contains("wikipedia json parse")
        ));
    }
}
