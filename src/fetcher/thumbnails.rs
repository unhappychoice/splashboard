//! Shared helper for the `ImageLinkedList` family of fetchers (`rss`,
//! `reddit_subreddit_posts`, `wikipedia_featured`, …): take a remote image URL, download it
//! once into `$SPLASHBOARD_HOME/cache/thumbnails/`, and return the local path so
//! `ratatui-image` can read it on render.
//!
//! Filename derives from a sha256 of the URL so two widgets sharing a thumbnail dedupe to one
//! file. Magic-byte sniffing picks the on-disk extension (`.png` / `.jpg` / `.webp` / `.gif`)
//! because servers happily send any format under any URL suffix. Size capped to keep a hostile
//! feed from filling the disk.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Client;
use sha2::{Digest, Sha256};

use crate::fetcher::FetchError;
use crate::paths;

const MAX_BYTES: usize = 4 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));

/// Download `url` once, returning the local file path. Cached on disk forever (callers rely on
/// the cache being content-addressed by URL hash so cache-busting requires a new URL). Returns
/// `Ok(None)` when `url` is empty or unsupported.
pub async fn download_to_cache(url: &str) -> Result<Option<PathBuf>, FetchError> {
    let url = url.trim();
    if url.is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
        return Ok(None);
    }
    let dir = thumbnail_dir()
        .ok_or_else(|| FetchError::Failed("$HOME not available for thumbnail cache".into()))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| FetchError::Failed(format!("create thumbnail cache dir: {e}")))?;
    let hash = hex(&Sha256::digest(url.as_bytes()));
    // Reuse the existing file regardless of extension; the on-disk magic byte detection picks
    // the canonical suffix, so once a URL is cached its extension never drifts.
    if let Some(existing) = existing_cached(&dir, &hash) {
        return Ok(Some(existing));
    }
    let bytes = fetch_bytes(url).await?;
    let ext = image_extension(&bytes);
    let path = dir.join(format!("{hash}.{ext}"));
    std::fs::write(&path, &bytes)
        .map_err(|e| FetchError::Failed(format!("write thumbnail: {e}")))?;
    Ok(Some(path))
}

/// Download every URL sequentially, in input order. Failures collapse to `None` so a single
/// broken image never turns a feed into an error — the widget shows the bad row with an empty
/// thumbnail cell next to the title. Sequential rather than parallel because feed widgets cap
/// at 20 entries and the cache absorbs repeat-runs; keeping the implementation dependency-free
/// (no `futures` crate) is the better tradeoff.
pub async fn download_many(urls: &[Option<String>]) -> Vec<Option<PathBuf>> {
    let mut out = Vec::with_capacity(urls.len());
    for maybe in urls {
        let path = match maybe.as_deref() {
            Some(u) => download_to_cache(u).await.ok().flatten(),
            None => None,
        };
        out.push(path);
    }
    out
}

fn thumbnail_dir() -> Option<PathBuf> {
    paths::cache_dir().map(|d| d.join("thumbnails"))
}

fn existing_cached(dir: &std::path::Path, hash: &str) -> Option<PathBuf> {
    ["png", "jpg", "webp", "gif"]
        .iter()
        .map(|ext| dir.join(format!("{hash}.{ext}")))
        .find(|p| p.is_file())
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

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, FetchError> {
    let res = http()
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("thumbnail request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("thumbnail {status}")));
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("thumbnail body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "thumbnail response too large ({} bytes, cap {MAX_BYTES})",
            bytes.len()
        )));
    }
    Ok(bytes.to_vec())
}

fn image_extension(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "jpg"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "webp"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "gif"
    } else {
        // Unknown signatures fall back to `.png` — same heuristic `github_avatar` uses. The
        // `image` crate will surface a decode error at render time if the bytes are genuinely
        // not an image.
        "png"
    }
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_extension_detects_known_signatures() {
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\nrest"), "png");
        assert_eq!(image_extension(&[0xff, 0xd8, 0xff, 0xdb, 0]), "jpg");
        assert_eq!(image_extension(b"GIF89a..."), "gif");
        let mut webp = Vec::from(*b"RIFF");
        webp.extend_from_slice(&[0, 0, 0, 0]);
        webp.extend_from_slice(b"WEBPVP8 ");
        assert_eq!(image_extension(&webp), "webp");
    }

    #[test]
    fn image_extension_falls_back_to_png() {
        assert_eq!(image_extension(&[0, 0, 0, 0]), "png");
        assert_eq!(image_extension(&[]), "png");
    }

    #[tokio::test]
    async fn empty_url_returns_none() {
        assert!(download_to_cache("").await.unwrap().is_none());
        assert!(download_to_cache("   ").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn non_http_url_returns_none() {
        assert!(
            download_to_cache("file:///etc/passwd")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            download_to_cache("ftp://example.com/x.png")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn hex_round_trips() {
        assert_eq!(hex(&[0x12, 0xab, 0xff, 0x00]), "12abff00");
    }
}
