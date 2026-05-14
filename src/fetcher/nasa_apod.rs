//! `nasa_apod` — NASA's Astronomy Picture of the Day, exposed as an image plus its metadata.
//!
//! Safety::Safe: the API host (`api.nasa.gov`) is hardcoded and the optional `api_key` only
//! ever leaves to that known host — the `github_*` "fixed-host authenticated fetcher stays
//! Safe" rule. The JSON response carries an arbitrary image URL string; like `random_dog` we
//! re-validate its host against an allowlist so a future API change can't redirect the image
//! download off-host.
//!
//! Multi-shape: `Image` (default) downloads the picture; `Text` / `TextBlock` /
//! `MarkdownTextBlock` / `LinkedTextBlock` / `Entries` reshape the same single API read into
//! the title / explanation / date metadata. On video days (APOD posts a video roughly weekly)
//! the `Image` shape errors out while the text shapes still render.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload, text_block_body};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::paths;
use crate::payload::{
    Body, EntriesData, Entry, ImageData, LinkedLine, LinkedTextBlockData, MarkdownTextBlockData,
    Payload, TextData,
};
use crate::render::Shape;
use crate::samples;

const API_BASE: &str = "https://api.nasa.gov/planetary/apod";
const ALLOWED_IMAGE_HOSTS: &[&str] = &["apod.nasa.gov", "www.nasa.gov", "science.nasa.gov"];
const DEFAULT_API_KEY: &str = "DEMO_KEY";
const API_KEY_ENV: &str = "NASA_API_KEY";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
/// Cap response bytes so a hostile / runaway response can't OOM the daemon. APOD's standard-res
/// `url` images run a few hundred KB to a couple MB; 10 MB is generous headroom while bounded.
const MAX_BYTES: usize = 10 * 1024 * 1024;

const SHAPES: &[Shape] = &[
    Shape::Image,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::LinkedTextBlock,
    Shape::Entries,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "api_key",
    type_hint: "string",
    required: false,
    default: Some("\"DEMO_KEY\""),
    description: "NASA API key. Falls back to the `NASA_API_KEY` env var, then to the shared `DEMO_KEY` (rate-limited but fine for a once-daily splash). Get a free key at api.nasa.gov.",
}];

pub struct NasaApodFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub api_key: Option<String>,
}

#[async_trait]
impl Fetcher for NasaApodFetcher {
    fn name(&self) -> &str {
        "nasa_apod"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "NASA's Astronomy Picture of the Day — a fresh curated space photograph each day. The default `Image` shape renders the picture; text shapes surface the title, explanation, and date. On the occasional video day the image shape errors while text shapes still render."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60
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
            Shape::Image => return None,
            Shape::Text => samples::text("Pillars of Creation"),
            Shape::TextBlock => samples::text_block(&[
                "Pillars of Creation",
                "These columns of cool interstellar gas and dust in the Eagle Nebula are incubators for new stars.",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "# Pillars of Creation\n\nThese columns of cool interstellar gas and dust in the Eagle Nebula are incubators for new stars.",
            ),
            Shape::LinkedTextBlock => samples::linked_text_block(&[(
                "Pillars of Creation",
                Some("https://apod.nasa.gov/apod/ap240115.html"),
            )]),
            Shape::Entries => samples::entries(&[
                ("date", "2024-01-15"),
                ("title", "Pillars of Creation"),
                ("media", "image"),
                ("copyright", "NASA, ESA, CSA, STScI"),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let api_key = resolve_api_key(opts.api_key.as_deref());
        let apod = fetch_apod(&build_api_url(&api_key)).await?;
        let shape = ctx.shape.unwrap_or(Shape::Image);
        let body = match shape {
            Shape::Image => image_body(&apod, &self.cache_key(ctx)).await?,
            other => text_body(&apod, other).ok_or_else(|| {
                FetchError::Failed(format!("nasa_apod can't emit {}", other.as_str()))
            })?,
        };
        Ok(payload(body))
    }
}

/// Reshape a fetched APOD into one of the non-image shapes. `None` for `Image` (handled
/// separately, since it needs an async download) or any shape outside `SHAPES`. Pure and sync
/// so every text-shape branch is testable without a network round-trip.
fn text_body(apod: &ApiResponse, shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::Text => Body::Text(TextData {
            value: apod.title.clone(),
        }),
        Shape::TextBlock => text_block_body(vec![apod.title.clone(), apod.explanation.clone()]),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: format!("# {}\n\n{}", apod.title, apod.explanation),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: vec![LinkedLine {
                text: apod.title.clone(),
                url: apod_page_url(&apod.date),
            }],
        }),
        Shape::Entries => entries_body(apod),
        _ => return None,
    })
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    date: String,
    title: String,
    explanation: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    media_type: String,
    #[serde(default)]
    copyright: Option<String>,
}

fn resolve_api_key(explicit: Option<&str>) -> String {
    let from_opt = explicit.map(str::trim).filter(|s| !s.is_empty());
    let from_env = || {
        std::env::var(API_KEY_ENV)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    from_opt
        .map(String::from)
        .or_else(from_env)
        .unwrap_or_else(|| DEFAULT_API_KEY.to_string())
}

fn build_api_url(api_key: &str) -> String {
    format!("{API_BASE}?api_key={}", encode_segment(api_key))
}

fn encode_segment(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// GET + parse the APOD JSON. Takes the fully-built URL (rather than the api key) so tests can
/// point it at a local server — mirrors `random_dog`'s `fetch_image_url`.
async fn fetch_apod(url: &str) -> Result<ApiResponse, FetchError> {
    let res = http()
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("apod API request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("apod API {status}")));
    }
    res.json::<ApiResponse>()
        .await
        .map_err(|e| FetchError::Failed(format!("apod API body: {e}")))
}

async fn image_body(apod: &ApiResponse, key: &str) -> Result<Body, FetchError> {
    if apod.media_type != "image" {
        return Err(FetchError::Failed(format!(
            "APOD is a {} today: {}",
            apod.media_type, apod.title
        )));
    }
    let path = download_image(&apod.url, key).await?;
    Ok(Body::Image(ImageData {
        path: path.to_string_lossy().into_owned(),
    }))
}

async fn download_image(image_url: &str, key: &str) -> Result<PathBuf, FetchError> {
    let out_dir = apod_dir()
        .ok_or_else(|| FetchError::Failed("$HOME not available for apod cache".into()))?;
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| FetchError::Failed(format!("create apod cache dir: {e}")))?;
    enforce_allowed_host(image_url)?;
    let bytes = fetch_bytes(image_url).await?;
    let ext = image_extension(&bytes)
        .ok_or_else(|| FetchError::Failed("unrecognized image format from apod".into()))?;
    remove_stale(&out_dir, key);
    let path = out_dir.join(format!("{key}.{ext}"));
    std::fs::write(&path, &bytes).map_err(|e| FetchError::Failed(format!("write apod: {e}")))?;
    Ok(path)
}

/// Defense in depth: the API hands us an arbitrary URL string in `url`. If `api.nasa.gov` ever
/// changes infrastructure or is compromised, we still refuse to follow off-host links rather
/// than handing the API a free SSRF primitive into our image downloader.
fn enforce_allowed_host(url: &str) -> Result<(), FetchError> {
    let host = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("");
    if ALLOWED_IMAGE_HOSTS.contains(&host) {
        Ok(())
    } else {
        Err(FetchError::Failed(format!(
            "apod API returned off-host image URL: {url}"
        )))
    }
}

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, FetchError> {
    let res = http()
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("apod image request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("apod image {status}")));
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("apod image body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "apod image too large: {} bytes",
            bytes.len()
        )));
    }
    Ok(bytes.to_vec())
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

/// APOD archive page for a `YYYY-MM-DD` date: `https://apod.nasa.gov/apod/apYYMMDD.html`.
/// Returns `None` for any date that isn't the expected all-digit `YYYY-MM-DD` shape.
fn apod_page_url(date: &str) -> Option<String> {
    let parts: Vec<&str> = date.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return None;
    };
    let well_formed = year.len() == 4
        && month.len() == 2
        && day.len() == 2
        && [year, month, day]
            .iter()
            .all(|p| p.bytes().all(|b| b.is_ascii_digit()));
    if !well_formed {
        return None;
    }
    Some(format!(
        "https://apod.nasa.gov/apod/ap{}{month}{day}.html",
        &year[2..]
    ))
}

fn entries_body(apod: &ApiResponse) -> Body {
    let mut items = vec![
        entry("date", &apod.date),
        entry("title", &apod.title),
        entry("media", &apod.media_type),
    ];
    if let Some(copyright) = apod
        .copyright
        .as_deref()
        .map(clean_copyright)
        .filter(|s| !s.is_empty())
    {
        items.push(entry("copyright", &copyright));
    }
    Body::Entries(EntriesData { items })
}

fn entry(key: &str, value: &str) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value.into()),
        status: None,
    }
}

/// APOD's `copyright` field is notoriously full of stray newlines and runs of spaces. Collapse
/// any whitespace run to a single space so it renders as one tidy line.
fn clean_copyright(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

fn apod_dir() -> Option<PathBuf> {
    paths::cache_dir().map(|d| d.join("apod"))
}

/// Remove any prior `<key>.<ext>` for this cache key so a format-changing refresh
/// (JPG → PNG) doesn't leave a misleading stale file with a different extension.
fn remove_stale(dir: &std::path::Path, key: &str) {
    for ext in ["png", "jpg", "gif", "webp"] {
        let _ = std::fs::remove_file(dir.join(format!("{key}.{ext}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn sample_apod(media_type: &str) -> ApiResponse {
        ApiResponse {
            date: "2024-01-15".into(),
            title: "Pillars of Creation".into(),
            explanation: "Towers of gas and dust in the Eagle Nebula.".into(),
            url: "https://apod.nasa.gov/apod/image/2401/pillars.jpg".into(),
            media_type: media_type.into(),
            copyright: Some("  NASA,\n  ESA  ".into()),
        }
    }

    fn restore_env(key: &str, previous: Option<String>) {
        unsafe {
            match previous {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    fn serve_once(status: &str, body: &[u8]) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let status = status.to_owned();
        let body = body.to_vec();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request);
            let header = format!(
                "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
            stream.flush().unwrap();
        });
        (format!("http://{addr}"), handle)
    }

    fn run_async<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn options_default_when_absent() {
        let opts: Options = parse_options(None).unwrap();
        assert!(opts.api_key.is_none());
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("bogus = 1").unwrap();
        assert!(parse_options::<Options>(Some(&raw)).is_err());
    }

    #[test]
    fn options_accept_api_key() {
        let raw: toml::Value = toml::from_str("api_key = \"abc123\"").unwrap();
        let opts: Options = parse_options(Some(&raw)).unwrap();
        assert_eq!(opts.api_key.as_deref(), Some("abc123"));
    }

    #[test]
    fn fetcher_exposes_catalog_metadata() {
        let fetcher = NasaApodFetcher;
        assert_eq!(fetcher.name(), "nasa_apod");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.default_shape(), Shape::Image);
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(
            fetcher
                .option_schemas()
                .iter()
                .map(|schema| schema.name)
                .collect::<Vec<_>>(),
            vec!["api_key"]
        );
        assert!(
            fetcher
                .description()
                .contains("Astronomy Picture of the Day")
        );
        assert!(fetcher.sample_body(Shape::Image).is_none());
        assert!(matches!(
            fetcher.sample_body(Shape::Text),
            Some(Body::Text(_))
        ));
        assert!(matches!(
            fetcher.sample_body(Shape::TextBlock),
            Some(Body::TextBlock(_))
        ));
        assert!(matches!(
            fetcher.sample_body(Shape::MarkdownTextBlock),
            Some(Body::MarkdownTextBlock(_))
        ));
        assert!(matches!(
            fetcher.sample_body(Shape::LinkedTextBlock),
            Some(Body::LinkedTextBlock(_))
        ));
        assert!(matches!(
            fetcher.sample_body(Shape::Entries),
            Some(Body::Entries(_))
        ));
    }

    #[test]
    fn resolve_api_key_prefers_explicit_option() {
        let _lock = paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var(API_KEY_ENV).ok();
        unsafe { std::env::set_var(API_KEY_ENV, "from-env") };
        assert_eq!(resolve_api_key(Some("from-option")), "from-option");
        restore_env(API_KEY_ENV, previous);
    }

    #[test]
    fn resolve_api_key_falls_back_to_env_then_demo_key() {
        let _lock = paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var(API_KEY_ENV).ok();
        unsafe { std::env::set_var(API_KEY_ENV, "from-env") };
        assert_eq!(resolve_api_key(None), "from-env");
        assert_eq!(resolve_api_key(Some("   ")), "from-env");
        unsafe { std::env::remove_var(API_KEY_ENV) };
        assert_eq!(resolve_api_key(None), DEFAULT_API_KEY);
        restore_env(API_KEY_ENV, previous);
    }

    #[test]
    fn build_api_url_appends_encoded_key() {
        assert_eq!(
            build_api_url("DEMO_KEY"),
            "https://api.nasa.gov/planetary/apod?api_key=DEMO_KEY"
        );
        assert_eq!(
            build_api_url("a b"),
            "https://api.nasa.gov/planetary/apod?api_key=a%20b"
        );
    }

    #[test]
    fn enforce_allowed_host_accepts_apod_host() {
        assert!(enforce_allowed_host("https://apod.nasa.gov/apod/image/x.jpg").is_ok());
        assert!(enforce_allowed_host("http://www.nasa.gov/x.png").is_ok());
    }

    #[test]
    fn enforce_allowed_host_rejects_off_host() {
        assert!(enforce_allowed_host("https://evil.example.com/x.jpg").is_err());
        assert!(enforce_allowed_host("https://apod.nasa.gov.evil.com/x.jpg").is_err());
        assert!(enforce_allowed_host("").is_err());
    }

    #[test]
    fn apod_page_url_builds_archive_link() {
        assert_eq!(
            apod_page_url("2024-01-15").as_deref(),
            Some("https://apod.nasa.gov/apod/ap240115.html")
        );
    }

    #[test]
    fn apod_page_url_rejects_malformed_dates() {
        assert!(apod_page_url("2024-1-5").is_none());
        assert!(apod_page_url("not-a-date").is_none());
        assert!(apod_page_url("2024-01").is_none());
        assert!(apod_page_url("20x4-01-15").is_none());
    }

    #[test]
    fn clean_copyright_collapses_whitespace() {
        assert_eq!(clean_copyright("  NASA,\n  ESA  "), "NASA, ESA");
        assert_eq!(clean_copyright("   "), "");
    }

    #[test]
    fn entries_body_includes_copyright_when_present() {
        let Body::Entries(data) = entries_body(&sample_apod("image")) else {
            panic!("expected Entries");
        };
        let keys: Vec<_> = data.items.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, vec!["date", "title", "media", "copyright"]);
        assert_eq!(data.items[3].value.as_deref(), Some("NASA, ESA"));
    }

    #[test]
    fn entries_body_omits_copyright_when_absent_or_blank() {
        let mut apod = sample_apod("image");
        apod.copyright = None;
        let Body::Entries(data) = entries_body(&apod) else {
            panic!("expected Entries");
        };
        assert_eq!(data.items.len(), 3);
        apod.copyright = Some("   \n  ".into());
        let Body::Entries(data) = entries_body(&apod) else {
            panic!("expected Entries");
        };
        assert_eq!(data.items.len(), 3);
    }

    #[test]
    fn image_extension_detects_known_signatures() {
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
        assert_eq!(
            image_extension(&[0xff, 0xd8, 0xff, 0xdb, 0x00]),
            Some("jpg")
        );
        assert_eq!(image_extension(b"GIF89a..."), Some("gif"));
        let mut webp = Vec::from(*b"RIFF");
        webp.extend_from_slice(&[0u8; 4]);
        webp.extend_from_slice(b"WEBPVP8 ");
        assert_eq!(image_extension(&webp), Some("webp"));
        assert!(image_extension(&[0, 0, 0, 0]).is_none());
    }

    #[test]
    fn image_body_rejects_video_media_type() {
        let err = run_async(image_body(&sample_apod("video"), "test-key")).unwrap_err();
        assert_eq!(
            format!("{err}"),
            "fetch failed: APOD is a video today: Pillars of Creation"
        );
    }

    #[test]
    fn cache_key_changes_with_shape_and_options() {
        let fetcher = NasaApodFetcher;
        let image_key = fetcher.cache_key(&ctx(Some(Shape::Image), None));
        let text_key = fetcher.cache_key(&ctx(Some(Shape::Text), None));
        let keyed = fetcher.cache_key(&ctx(
            Some(Shape::Image),
            Some(toml::from_str("api_key = \"abc\"").unwrap()),
        ));
        assert_ne!(image_key, text_key);
        assert_ne!(image_key, keyed);
    }

    #[test]
    fn remove_stale_removes_other_extensions() {
        let tmp = tempfile::tempdir().unwrap();
        let key = "abc";
        for ext in ["png", "jpg", "gif", "webp"] {
            std::fs::write(tmp.path().join(format!("{key}.{ext}")), b"x").unwrap();
        }
        remove_stale(tmp.path(), key);
        for ext in ["png", "jpg", "gif", "webp"] {
            assert!(!tmp.path().join(format!("{key}.{ext}")).exists());
        }
    }

    #[test]
    fn apod_dir_uses_cache_layout_under_env_override() {
        let _lock = paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let previous = std::env::var("SPLASHBOARD_HOME").ok();
        unsafe { std::env::set_var("SPLASHBOARD_HOME", tmp.path()) };
        assert_eq!(apod_dir(), Some(tmp.path().join("cache").join("apod")));
        restore_env("SPLASHBOARD_HOME", previous);
    }

    #[test]
    fn fetch_rejects_unknown_options_before_network() {
        let err = run_async(NasaApodFetcher.fetch(&ctx(
            Some(Shape::Image),
            Some(toml::from_str("bogus = true").unwrap()),
        )))
        .unwrap_err();
        assert!(format!("{err}").contains("unknown field"));
    }

    #[test]
    fn text_body_builds_each_text_shape() {
        let apod = sample_apod("image");
        let Some(Body::Text(t)) = text_body(&apod, Shape::Text) else {
            panic!("expected Text");
        };
        assert_eq!(t.value, "Pillars of Creation");

        let Some(Body::TextBlock(tb)) = text_body(&apod, Shape::TextBlock) else {
            panic!("expected TextBlock");
        };
        assert_eq!(tb.lines, vec![apod.title.clone(), apod.explanation.clone()]);

        let Some(Body::MarkdownTextBlock(md)) = text_body(&apod, Shape::MarkdownTextBlock) else {
            panic!("expected MarkdownTextBlock");
        };
        assert!(md.value.starts_with("# Pillars of Creation\n\n"));
        assert!(md.value.ends_with(&apod.explanation));

        let Some(Body::LinkedTextBlock(lt)) = text_body(&apod, Shape::LinkedTextBlock) else {
            panic!("expected LinkedTextBlock");
        };
        assert_eq!(lt.items.len(), 1);
        assert_eq!(lt.items[0].text, "Pillars of Creation");
        assert_eq!(
            lt.items[0].url.as_deref(),
            Some("https://apod.nasa.gov/apod/ap240115.html")
        );

        assert!(matches!(
            text_body(&apod, Shape::Entries),
            Some(Body::Entries(_))
        ));
    }

    #[test]
    fn text_body_returns_none_for_image_and_unsupported_shapes() {
        let apod = sample_apod("image");
        assert!(text_body(&apod, Shape::Image).is_none());
        assert!(text_body(&apod, Shape::Bars).is_none());
        assert!(text_body(&apod, Shape::Heatmap).is_none());
    }

    #[test]
    fn text_body_linked_block_drops_url_for_malformed_date() {
        let mut apod = sample_apod("image");
        apod.date = "not-a-date".into();
        let Some(Body::LinkedTextBlock(lt)) = text_body(&apod, Shape::LinkedTextBlock) else {
            panic!("expected LinkedTextBlock");
        };
        assert!(lt.items[0].url.is_none());
    }

    #[test]
    fn fetch_apod_reads_success_payload() {
        let body = br#"{"date":"2024-01-15","title":"Pillars of Creation","explanation":"Towers of gas.","url":"https://apod.nasa.gov/apod/image/2401/pillars.jpg","media_type":"image","copyright":"NASA"}"#;
        let (url, server) = serve_once("200 OK", body);
        let apod = run_async(fetch_apod(&url)).unwrap();
        server.join().unwrap();
        assert_eq!(apod.title, "Pillars of Creation");
        assert_eq!(apod.media_type, "image");
        assert_eq!(apod.date, "2024-01-15");
    }

    #[test]
    fn fetch_apod_rejects_http_status() {
        let (url, server) = serve_once("403 Forbidden", br#"{"error":"over rate limit"}"#);
        let err = run_async(fetch_apod(&url)).unwrap_err();
        server.join().unwrap();
        assert_eq!(format!("{err}"), "fetch failed: apod API 403 Forbidden");
    }

    #[test]
    fn fetch_apod_rejects_invalid_json() {
        let (url, server) = serve_once("200 OK", b"not-json");
        let err = run_async(fetch_apod(&url)).unwrap_err();
        server.join().unwrap();
        assert!(format!("{err}").contains("apod API body"));
    }

    #[test]
    fn fetch_apod_rejects_malformed_url() {
        let err = run_async(fetch_apod("https://bad host")).unwrap_err();
        assert!(format!("{err}").contains("apod API request failed"));
    }

    #[test]
    fn fetch_bytes_reads_small_body() {
        let bytes = b"\x89PNG\r\n\x1a\nrest";
        let (url, server) = serve_once("200 OK", bytes);
        let downloaded = run_async(fetch_bytes(&url)).unwrap();
        server.join().unwrap();
        assert_eq!(downloaded, bytes);
    }

    #[test]
    fn fetch_bytes_rejects_malformed_url() {
        let err = run_async(fetch_bytes("https://bad host")).unwrap_err();
        assert!(format!("{err}").contains("apod image request failed"));
    }

    #[test]
    fn fetch_bytes_rejects_oversized_body() {
        let (url, server) = serve_once("200 OK", &vec![b'x'; MAX_BYTES + 1]);
        let err = run_async(fetch_bytes(&url)).unwrap_err();
        server.join().unwrap();
        assert_eq!(
            format!("{err}"),
            format!(
                "fetch failed: apod image too large: {} bytes",
                MAX_BYTES + 1
            )
        );
    }

    #[test]
    fn fetch_bytes_rejects_http_status() {
        let (url, server) = serve_once("404 Not Found", b"missing");
        let err = run_async(fetch_bytes(&url)).unwrap_err();
        server.join().unwrap();
        assert_eq!(format!("{err}"), "fetch failed: apod image 404 Not Found");
    }

    #[test]
    fn download_image_rejects_off_host_url_before_network() {
        let _lock = paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let previous = std::env::var("SPLASHBOARD_HOME").ok();
        unsafe { std::env::set_var("SPLASHBOARD_HOME", tmp.path()) };
        let err = run_async(download_image(
            "https://evil.example.com/x.jpg",
            "test-apod",
        ))
        .unwrap_err();
        restore_env("SPLASHBOARD_HOME", previous);
        assert!(format!("{err}").contains("off-host image URL"));
    }

    #[test]
    fn download_image_surfaces_cache_dir_creation_failure() {
        let _lock = paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("not-a-dir");
        std::fs::write(&file, b"x").unwrap();
        let previous = std::env::var("SPLASHBOARD_HOME").ok();
        unsafe { std::env::set_var("SPLASHBOARD_HOME", &file) };
        let err = run_async(download_image(
            "https://apod.nasa.gov/apod/image/x.jpg",
            "test-apod",
        ))
        .unwrap_err();
        restore_env("SPLASHBOARD_HOME", previous);
        assert!(format!("{err}").contains("create apod cache dir"));
    }

    /// Live smoke test — downloads today's APOD and verifies the file is a real image. `#[ignore]`
    /// keeps CI offline-safe; run with
    /// `cargo test -- --ignored fetcher::nasa_apod::tests::live --nocapture`.
    #[tokio::test]
    #[ignore]
    #[allow(clippy::await_holding_lock)]
    async fn live_downloads_todays_apod() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("SPLASHBOARD_HOME", tmp.path()) };
        let apod = fetch_apod(&build_api_url(DEFAULT_API_KEY)).await.unwrap();
        eprintln!("APOD {}: {} ({})", apod.date, apod.title, apod.media_type);
        if apod.media_type == "image" {
            let path = download_image(&apod.url, "test-apod").await.unwrap();
            let bytes = std::fs::read(&path).unwrap();
            assert!(image_extension(&bytes).is_some(), "unrecognized format");
        }
        unsafe { std::env::remove_var("SPLASHBOARD_HOME") };
    }
}
