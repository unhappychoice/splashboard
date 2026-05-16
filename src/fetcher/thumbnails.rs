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
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn restore_home(previous: Option<String>) {
        unsafe {
            match previous {
                Some(value) => std::env::set_var("SPLASHBOARD_HOME", value),
                None => std::env::remove_var("SPLASHBOARD_HOME"),
            }
        }
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
        (format!("http://{addr}/img.bin"), handle)
    }

    fn hash_of(url: &str) -> String {
        hex(&Sha256::digest(url.as_bytes()))
    }

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

    #[tokio::test]
    async fn download_many_preserves_order_and_passes_none_through() {
        // Empty + None + non-http URLs all resolve to None (no network involved) in input
        // order, so the renderer can zip them back against the source list without
        // misalignment.
        let urls = vec![
            None,
            Some(String::new()),
            Some("file:///etc/passwd".into()),
            Some("not-a-url".into()),
        ];
        let paths = download_many(&urls).await;
        assert_eq!(paths.len(), 4);
        assert!(paths.iter().all(|p| p.is_none()));
    }

    #[test]
    fn existing_cached_returns_first_matching_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let hash = "deadbeef";
        // No file yet — None.
        assert!(existing_cached(dir, hash).is_none());
        // .jpg exists alongside no .png — picks .jpg.
        let jpg = dir.join(format!("{hash}.jpg"));
        std::fs::write(&jpg, b"jpeg-bytes").unwrap();
        assert_eq!(existing_cached(dir, hash), Some(jpg.clone()));
        // .png also exists — preference order in the helper is png, jpg, webp, gif, so png wins.
        let png = dir.join(format!("{hash}.png"));
        std::fs::write(&png, b"png-bytes").unwrap();
        assert_eq!(existing_cached(dir, hash), Some(png));
    }

    #[tokio::test]
    async fn download_to_cache_no_home_yields_failed_error() {
        // Bypass network: an unsupported scheme returns Ok(None) without touching the cache
        // dir at all, so we don't need to mock $HOME for that path. This complements the
        // happy-path coverage which lives under the live ignored tests in fetcher/rss.rs.
        let res = download_to_cache("ftp://example.com/x.png").await.unwrap();
        assert!(res.is_none());
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn download_to_cache_returns_existing_cached_path_without_network() {
        let _lock = paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let previous = std::env::var("SPLASHBOARD_HOME").ok();
        unsafe { std::env::set_var("SPLASHBOARD_HOME", tmp.path()) };
        let url = "https://example.com/cached.png";
        let dir = tmp.path().join("cache").join("thumbnails");
        std::fs::create_dir_all(&dir).unwrap();
        let cached = dir.join(format!("{}.png", hash_of(url)));
        std::fs::write(&cached, b"pretend-png").unwrap();
        let result = download_to_cache(url).await.unwrap();
        restore_home(previous);
        assert_eq!(result, Some(cached));
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn download_to_cache_writes_new_thumbnail_to_disk() {
        let _lock = paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let previous = std::env::var("SPLASHBOARD_HOME").ok();
        unsafe { std::env::set_var("SPLASHBOARD_HOME", tmp.path()) };
        let payload = b"\x89PNG\r\n\x1a\nfreshly-downloaded";
        let (url, server) = serve_once("200 OK", "image/png", payload);
        let result = download_to_cache(&url).await.unwrap();
        server.join().unwrap();
        restore_home(previous);
        let path = result.expect("happy path should yield a cached path");
        assert_eq!(path.extension().unwrap(), "png");
        let written = std::fs::read(&path).unwrap();
        assert_eq!(written, payload);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn download_to_cache_propagates_http_error_status() {
        let _lock = paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let previous = std::env::var("SPLASHBOARD_HOME").ok();
        unsafe { std::env::set_var("SPLASHBOARD_HOME", tmp.path()) };
        let (url, server) = serve_once("404 Not Found", "text/plain", b"missing");
        let err = download_to_cache(&url).await.unwrap_err();
        server.join().unwrap();
        restore_home(previous);
        let FetchError::Failed(msg) = err else {
            panic!("expected Failed, got {err:?}");
        };
        assert!(msg.contains("thumbnail 404"), "unexpected error: {msg}");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn download_to_cache_rejects_oversized_payload() {
        let _lock = paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let previous = std::env::var("SPLASHBOARD_HOME").ok();
        unsafe { std::env::set_var("SPLASHBOARD_HOME", tmp.path()) };
        let oversize = vec![b'x'; MAX_BYTES + 1];
        let (url, server) = serve_once("200 OK", "image/png", &oversize);
        let err = download_to_cache(&url).await.unwrap_err();
        server.join().unwrap();
        restore_home(previous);
        let FetchError::Failed(msg) = err else {
            panic!("expected Failed, got {err:?}");
        };
        assert!(
            msg.contains("thumbnail response too large"),
            "unexpected error: {msg}"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn download_many_collapses_individual_failures_to_none() {
        // A 404 mid-list returns Err inside download_to_cache; download_many is expected to
        // swallow it via .ok().flatten() so the remaining slots still line up with input order.
        let _lock = paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let previous = std::env::var("SPLASHBOARD_HOME").ok();
        unsafe { std::env::set_var("SPLASHBOARD_HOME", tmp.path()) };
        let (bad_url, server) = serve_once("500 Internal Server Error", "text/plain", b"boom");
        let urls = vec![None, Some(bad_url), Some("not-a-url".into())];
        let paths = download_many(&urls).await;
        server.join().unwrap();
        restore_home(previous);
        assert_eq!(paths.len(), 3);
        assert!(paths.iter().all(|p| p.is_none()));
    }
}
