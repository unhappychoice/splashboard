//! `random_cat` — download a random cat image from cataas.com and expose it as `Body::Image`.
//!
//! Safety::Safe: the host is hardcoded (`cataas.com`), so config can't redirect traffic to a
//! different origin. No auth is needed, so no token can leak. Mirrors `github_avatar`'s
//! "fixed-host fetcher stays Safe" stance — see `project_trust_model.md`.
//!
//! Caching: bytes are written to `$SPLASHBOARD_HOME/cache/cats/<cache_key>.<ext>`. Each
//! `refresh_interval` tick produces a new cat (the API returns a different image per request).
//! The framework gates re-downloads via the cache TTL.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::paths;
use crate::payload::{Body, ImageData, Payload};
use crate::render::Shape;

const CATAAS_BASE: &str = "https://cataas.com";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
/// Cap response bytes so a hostile / runaway response can't OOM the daemon. Cat photos
/// are typically 50–500 KB — 5 MB is plenty of headroom while still bounded.
const MAX_BYTES: usize = 5 * 1024 * 1024;

const SHAPES: &[Shape] = &[Shape::Image];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "tag",
    type_hint: "string",
    required: false,
    default: None,
    description: "cataas.com tag filter (e.g. `cute`, `orange`, `kitten`). Omit for any cat.",
}];

pub struct RandomCatFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub tag: Option<String>,
}

#[async_trait]
impl Fetcher for RandomCatFetcher {
    fn name(&self) -> &str {
        "random_cat"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "A random cat picture from cataas.com, downloaded each refresh cycle and rendered as an image. Optional `tag` filters by mood (`cute` / `orange` / `kitten` / …)."
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
    fn sample_body(&self, _shape: Shape) -> Option<Body> {
        None
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let key = self.cache_key(ctx);
        let path = download_cat(opts.tag.as_deref(), &key).await?;
        Ok(payload(Body::Image(ImageData {
            path: path.to_string_lossy().into_owned(),
        })))
    }
}

async fn download_cat(tag: Option<&str>, key: &str) -> Result<PathBuf, FetchError> {
    let out_dir =
        cat_dir().ok_or_else(|| FetchError::Failed("$HOME not available for cat cache".into()))?;
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| FetchError::Failed(format!("create cat cache dir: {e}")))?;
    let url = build_url(tag);
    let bytes = fetch_bytes(&url).await?;
    let ext = image_extension(&bytes)
        .ok_or_else(|| FetchError::Failed("unrecognized image format from cataas".into()))?;
    remove_stale(&out_dir, key);
    let path = out_dir.join(format!("{key}.{ext}"));
    std::fs::write(&path, &bytes).map_err(|e| FetchError::Failed(format!("write cat: {e}")))?;
    Ok(path)
}

fn build_url(tag: Option<&str>) -> String {
    match tag.map(str::trim).filter(|s| !s.is_empty()) {
        Some(t) => format!("{CATAAS_BASE}/cat/{}", encode_tag(t)),
        None => format!("{CATAAS_BASE}/cat"),
    }
}

/// Percent-encode the tag so spaces / commas / other URL-significant chars don't break the path.
/// cataas.com accepts comma-joined tags (`cute,fat`) but we treat the option as a single token —
/// the user can pass `"cute,fat"` and it'll be encoded transparently.
fn encode_tag(tag: &str) -> String {
    tag.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b',' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, FetchError> {
    let res = http()
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("cat request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("cat {status}")));
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("cat body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "cat body too large: {} bytes",
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

fn cat_dir() -> Option<PathBuf> {
    paths::cache_dir().map(|d| d.join("cats"))
}

/// Remove any prior `<key>.<ext>` for this cache key so format-changing refreshes
/// (PNG → JPG) don't leave a misleading stale file with a different extension.
fn remove_stale(dir: &std::path::Path, key: &str) {
    for ext in ["png", "jpg", "gif", "webp"] {
        let _ = std::fs::remove_file(dir.join(format!("{key}.{ext}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_default_when_absent() {
        let opts: Options = parse_options(None).unwrap();
        assert!(opts.tag.is_none());
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("bogus = 1").unwrap();
        assert!(parse_options::<Options>(Some(&raw)).is_err());
    }

    #[test]
    fn options_accept_tag() {
        let raw: toml::Value = toml::from_str("tag = \"cute\"").unwrap();
        let opts: Options = parse_options(Some(&raw)).unwrap();
        assert_eq!(opts.tag.as_deref(), Some("cute"));
    }

    #[test]
    fn build_url_without_tag() {
        assert_eq!(build_url(None), "https://cataas.com/cat");
    }

    #[test]
    fn build_url_with_tag() {
        assert_eq!(build_url(Some("cute")), "https://cataas.com/cat/cute");
    }

    #[test]
    fn build_url_treats_empty_tag_as_none() {
        assert_eq!(build_url(Some("")), "https://cataas.com/cat");
        assert_eq!(build_url(Some("   ")), "https://cataas.com/cat");
    }

    #[test]
    fn build_url_encodes_special_chars() {
        assert_eq!(
            build_url(Some("orange tabby")),
            "https://cataas.com/cat/orange%20tabby"
        );
    }

    #[test]
    fn build_url_keeps_comma_for_multi_tag() {
        // cataas.com supports `cute,fat` to AND tags; comma is path-safe so we leave it alone.
        assert_eq!(
            build_url(Some("cute,fat")),
            "https://cataas.com/cat/cute,fat"
        );
    }

    #[test]
    fn image_extension_detects_png() {
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
    }

    #[test]
    fn image_extension_detects_jpeg() {
        assert_eq!(
            image_extension(&[0xff, 0xd8, 0xff, 0xdb, 0x00]),
            Some("jpg")
        );
    }

    #[test]
    fn image_extension_detects_gif() {
        assert_eq!(image_extension(b"GIF89a..."), Some("gif"));
        assert_eq!(image_extension(b"GIF87a..."), Some("gif"));
    }

    #[test]
    fn image_extension_detects_webp() {
        let mut bytes = vec![];
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&[0u8; 4]);
        bytes.extend_from_slice(b"WEBPVP8 ");
        assert_eq!(image_extension(&bytes), Some("webp"));
    }

    #[test]
    fn image_extension_unknown_returns_none() {
        assert!(image_extension(&[0, 0, 0, 0]).is_none());
        assert!(image_extension(b"<html>").is_none());
    }

    #[test]
    fn cache_key_changes_with_tag() {
        let mut ctx = FetchContext {
            widget_id: "w".into(),
            timeout: Duration::from_secs(1),
            shape: Some(Shape::Image),
            ..Default::default()
        };
        let none_key = RandomCatFetcher.cache_key(&ctx);
        ctx.options = Some(toml::Value::Table({
            let mut t = toml::value::Table::new();
            t.insert("tag".into(), toml::Value::String("cute".into()));
            t
        }));
        let cute_key = RandomCatFetcher.cache_key(&ctx);
        assert_ne!(none_key, cute_key);
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
            assert!(
                !tmp.path().join(format!("{key}.{ext}")).exists(),
                "stale {ext} not removed"
            );
        }
    }

    /// Live smoke test — downloads a cat and verifies the file is a real image. `#[ignore]` keeps
    /// CI offline-safe; run with
    /// `cargo test -- --ignored fetcher::random_cat::tests::live --nocapture`.
    #[tokio::test]
    #[ignore]
    #[allow(clippy::await_holding_lock)]
    async fn live_downloads_a_real_cat() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("SPLASHBOARD_HOME", tmp.path()) };
        let path = download_cat(None, "test-cat").await.unwrap();
        eprintln!("wrote {}", path.display());
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            bytes.len() > 256,
            "cat suspiciously small: {} bytes",
            bytes.len()
        );
        assert!(
            image_extension(&bytes).is_some(),
            "unrecognized image format"
        );
        unsafe { std::env::remove_var("SPLASHBOARD_HOME") };
    }
}
