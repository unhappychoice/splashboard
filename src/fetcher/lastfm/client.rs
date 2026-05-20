//! Shared HTTP client + auth for the `lastfm_*` fetcher family.
//!
//! Every request targets `ws.audioscrobbler.com` — the host is hardcoded, so the API key
//! never leaves to an attacker-controlled origin. Same `Safety::Safe` model as `github_*`:
//! config-provided fields (`user`, `period`, …) only change which resource within the fixed
//! host is queried.
//!
//! Auth: `LASTFM_API_KEY` (free, https://www.last.fm/api/account/create).
//! Last.fm signals errors with HTTP 200 + `{"error": <code>, "message": <msg>}` rather than a
//! non-2xx status, so [`get_json`] inspects the JSON envelope before deserializing into the
//! caller's target type.

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Client;
use serde::de::DeserializeOwned;

use crate::fetcher::FetchError;

const API_BASE: &str = "https://ws.audioscrobbler.com/2.0/";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BYTES: usize = 5 * 1024 * 1024;

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

pub fn resolve_api_key() -> Result<String, FetchError> {
    std::env::var("LASTFM_API_KEY").map_err(|_| FetchError::Failed("LASTFM_API_KEY not set".into()))
}

/// `GET https://ws.audioscrobbler.com/2.0/?method=<method>&api_key=<key>&format=json&<params>`
/// then deserialize into `T`. Caller passes additional query pairs as `(key, value)` slices;
/// `method`, `api_key`, and `format` are appended unconditionally.
pub async fn get_json<T: DeserializeOwned>(
    method: &str,
    params: &[(&str, &str)],
) -> Result<T, FetchError> {
    let key = resolve_api_key()?;
    let url = build_url(method, &key, params);
    let res = http()
        .get(&url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("lastfm request failed: {e}")))?;
    parse_json(res).await
}

fn build_url(method: &str, api_key: &str, params: &[(&str, &str)]) -> String {
    let mut url = url::Url::parse(API_BASE).expect("static API_BASE parses");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("method", method);
        q.append_pair("api_key", api_key);
        q.append_pair("format", "json");
        for (k, v) in params {
            q.append_pair(k, v);
        }
    }
    url.into()
}

async fn parse_json<T: DeserializeOwned>(res: reqwest::Response) -> Result<T, FetchError> {
    let status = res.status();
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("lastfm read body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "lastfm response too large ({} bytes, cap {MAX_BYTES})",
            bytes.len()
        )));
    }
    if !status.is_success() {
        return Err(FetchError::Failed(format!("lastfm {status}")));
    }
    // Last.fm tunnels errors through a 200 + `{"error":N,"message":...}` envelope, so we
    // detect that before handing bytes to the caller's deserializer.
    if let Some(err) = parse_api_error(&bytes) {
        return Err(err);
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("lastfm json parse: {e}")))
}

fn parse_api_error(bytes: &[u8]) -> Option<FetchError> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let code = value.get("error").and_then(|v| v.as_i64())?;
    let message = value
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("(no message)");
    Some(FetchError::Failed(format!(
        "lastfm error {code}: {message}"
    )))
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize)]
    struct TestPayload {
        ok: bool,
    }

    #[test]
    fn build_url_includes_method_api_key_and_format() {
        let url = build_url(
            "user.getRecentTracks",
            "secret",
            &[("user", "rj"), ("limit", "5")],
        );
        assert!(url.starts_with("https://ws.audioscrobbler.com/2.0/?"));
        assert!(url.contains("method=user.getRecentTracks"));
        assert!(url.contains("api_key=secret"));
        assert!(url.contains("format=json"));
        assert!(url.contains("user=rj"));
        assert!(url.contains("limit=5"));
    }

    #[test]
    fn build_url_percent_encodes_param_values() {
        // Last.fm usernames are ASCII but reserved characters in artist / track query params
        // (slashes, ampersands, plus, …) must encode so we don't smuggle extra query pairs.
        let url = build_url("artist.search", "k", &[("artist", "AC/DC & Friends")]);
        assert!(
            url.contains("artist=AC%2FDC+%26+Friends")
                || url.contains("artist=AC%2FDC%20%26%20Friends")
        );
    }

    #[test]
    fn resolve_api_key_fails_with_clear_message_when_unset() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("LASTFM_API_KEY").ok();
        unsafe { std::env::remove_var("LASTFM_API_KEY") };
        let err = resolve_api_key().unwrap_err();
        assert!(matches!(err, FetchError::Failed(m) if m == "LASTFM_API_KEY not set"));
        if let Some(v) = prev {
            unsafe { std::env::set_var("LASTFM_API_KEY", v) };
        }
    }

    #[test]
    fn resolve_api_key_reads_env() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("LASTFM_API_KEY").ok();
        unsafe { std::env::set_var("LASTFM_API_KEY", "abc123") };
        assert_eq!(resolve_api_key().unwrap(), "abc123");
        match prev {
            Some(v) => unsafe { std::env::set_var("LASTFM_API_KEY", v) },
            None => unsafe { std::env::remove_var("LASTFM_API_KEY") },
        }
    }

    #[test]
    fn parse_api_error_surfaces_lastfm_error_envelope() {
        let bytes = br#"{"error":6,"message":"User not found"}"#;
        let err = parse_api_error(bytes).expect("error envelope must be detected");
        let FetchError::Failed(msg) = err else {
            panic!("expected FetchError::Failed");
        };
        assert!(msg.contains("lastfm error 6"));
        assert!(msg.contains("User not found"));
    }

    #[test]
    fn parse_api_error_returns_none_on_success_bodies() {
        let bytes = br#"{"recenttracks":{"track":[]}}"#;
        assert!(parse_api_error(bytes).is_none());
    }

    #[test]
    fn parse_api_error_returns_none_on_unparseable_bytes() {
        assert!(parse_api_error(b"not-json").is_none());
    }

    #[test]
    fn http_reuses_the_same_client() {
        assert!(std::ptr::eq(http(), http()));
    }

    #[test]
    fn parse_json_deserializes_success_body() {
        let payload: TestPayload = parse_local("200 OK", r#"{"ok":true}"#).unwrap();
        assert!(payload.ok);
    }

    #[test]
    fn parse_json_surfaces_non_success_status() {
        let err = parse_local::<TestPayload>("503 Service Unavailable", "").unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(msg) if msg == "lastfm 503 Service Unavailable"
        ));
    }

    #[test]
    fn parse_json_surfaces_json_parse_errors() {
        let err = parse_local::<TestPayload>("200 OK", "not-json").unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(msg) if msg.starts_with("lastfm json parse:")
        ));
    }

    #[test]
    fn parse_json_rejects_oversized_body() {
        let body = "x".repeat(MAX_BYTES + 1);
        let err = parse_local::<TestPayload>("200 OK", &body).unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(msg) if msg.contains("lastfm response too large")
        ));
    }

    #[test]
    fn parse_json_surfaces_lastfm_error_envelope_on_a_200() {
        // Last.fm tunnels errors through HTTP 200 + `{"error":N,...}`; `parse_json` must catch
        // that envelope before the caller's deserializer ever sees the bytes.
        let err = parse_local::<TestPayload>("200 OK", r#"{"error":6,"message":"User not found"}"#)
            .unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(msg)
                if msg.contains("lastfm error 6") && msg.contains("User not found")
        ));
    }

    #[test]
    fn get_json_fails_fast_when_api_key_missing() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("LASTFM_API_KEY").ok();
        unsafe { std::env::remove_var("LASTFM_API_KEY") };
        let err = run_async(get_json::<TestPayload>("user.getInfo", &[])).unwrap_err();
        if let Some(v) = prev {
            unsafe { std::env::set_var("LASTFM_API_KEY", v) };
        }
        assert!(matches!(err, FetchError::Failed(msg) if msg == "LASTFM_API_KEY not set"));
    }

    /// Serves `body` from a one-shot local server, drives a real `reqwest` request through the
    /// shared `http()` client, and hands the resulting `Response` to `parse_json` — exercising
    /// the body-size, status, error-envelope, and deserialization branches without network.
    fn parse_local<T: DeserializeOwned>(status: &str, body: &str) -> Result<T, FetchError> {
        let (url, server) = serve_once(status, body);
        let result = run_async(async {
            let res = http().get(&url).send().await.unwrap();
            parse_json::<T>(res).await
        });
        server.join().unwrap();
        result
    }

    fn serve_once(status: &str, body: &str) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let status = status.to_owned();
        let body = body.to_owned();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
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
}
