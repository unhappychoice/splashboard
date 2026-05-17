//! Shared HTTP client + auth for the `gitlab_*` family. GitLab uses `PRIVATE-TOKEN` (Personal
//! Access Token) rather than bearer auth as its canonical PAT header; we honour that so users
//! can paste a token from the GitLab UI directly into `secrets.toml`.
//!
//! Every request flows through [`rest_get`], which composes `https://{host}/api/v4{path}` —
//! `host` is the caller-supplied (and pre-validated) GitLab instance, defaulting to
//! `gitlab.com`. The fetcher resolves and validates the host before reaching this module, so we
//! treat it as already-safe here.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::fetcher::FetchError;

const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_HOST: &str = "gitlab.com";

static AUTHENTICATED_USERNAME_CACHE: OnceLock<Mutex<Option<(String, String)>>> = OnceLock::new();

pub fn default_host() -> &'static str {
    DEFAULT_HOST
}

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

pub fn resolve_token() -> Result<String, FetchError> {
    std::env::var("GITLAB_TOKEN").map_err(|_| FetchError::Failed("GITLAB_TOKEN not set".into()))
}

/// Username of the token-authenticated user. Resolved lazily via `/api/v4/user` and memoised
/// per-host so the `gitlab_review_requests` widget doesn't pay a roundtrip on every refresh.
/// Keyed on `host` because two configs pointing at different GitLab instances must not bleed
/// usernames into each other.
pub async fn resolve_authenticated_username(host: &str) -> Result<String, FetchError> {
    let slot = AUTHENTICATED_USERNAME_CACHE.get_or_init(|| Mutex::new(None));
    if let Some(cached) = slot
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .filter(|(h, _)| h == host)
    {
        return Ok(cached.1);
    }
    #[derive(Deserialize)]
    struct Me {
        username: String,
    }
    let me: Me = rest_get(host, "/user").await?;
    if let Ok(mut g) = slot.lock() {
        *g = Some((host.into(), me.username.clone()));
    }
    Ok(me.username)
}

#[cfg(test)]
pub(crate) fn clear_authenticated_username_cache() {
    if let Ok(mut g) = AUTHENTICATED_USERNAME_CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
    {
        *g = None;
    }
}

/// REST GET → deserialize JSON. `path` is the part after `/api/v4` (e.g. `"/user"`,
/// `"/projects/group%2Fproj"`). Non-2xx responses surface the GitLab `message` field when
/// present so the error placeholder and the log line are both actionable.
pub async fn rest_get<T: DeserializeOwned>(host: &str, path: &str) -> Result<T, FetchError> {
    let token = resolve_token()?;
    let url = format!("https://{host}/api/v4{path}");
    let res = http()
        .get(&url)
        .header("PRIVATE-TOKEN", &token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("gitlab request failed: {e}")))?;
    parse_json(res).await
}

async fn parse_json<T: DeserializeOwned>(res: reqwest::Response) -> Result<T, FetchError> {
    let status = res.status();
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("gitlab response body: {e}")))?;
    if !status.is_success() {
        return Err(FetchError::Failed(error_message(status, &bytes)));
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("gitlab json parse: {e}")))
}

fn error_message(status: StatusCode, body: &[u8]) -> String {
    #[derive(Deserialize)]
    struct ApiError {
        message: Option<serde_json::Value>,
        error: Option<String>,
    }
    let reported = serde_json::from_slice::<ApiError>(body).ok().and_then(|e| {
        e.message
            .as_ref()
            .map(|m| match m {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .or(e.error)
    });
    match reported {
        Some(m) => format!("gitlab {status}: {m}"),
        None => format!("gitlab {status}"),
    }
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
        username: String,
    }

    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        restore: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn set(pairs: &[(&'static str, Option<&str>)]) -> Self {
            let lock = crate::paths::TEST_ENV_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let restore = pairs
                .iter()
                .map(|(k, v)| {
                    let prev = std::env::var(k).ok();
                    match v {
                        Some(value) => unsafe { std::env::set_var(k, value) },
                        None => unsafe { std::env::remove_var(k) },
                    }
                    (*k, prev)
                })
                .collect();
            Self {
                _lock: lock,
                restore,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.restore {
                match v {
                    Some(value) => unsafe { std::env::set_var(k, value) },
                    None => unsafe { std::env::remove_var(k) },
                }
            }
        }
    }

    fn run_async<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn default_host_is_gitlab_dot_com() {
        assert_eq!(default_host(), "gitlab.com");
    }

    #[test]
    fn resolve_token_reads_gitlab_token() {
        let _g = EnvGuard::set(&[("GITLAB_TOKEN", Some("glpat-xxx"))]);
        assert_eq!(resolve_token().unwrap(), "glpat-xxx");
    }

    #[test]
    fn resolve_token_fails_when_unset() {
        let _g = EnvGuard::set(&[("GITLAB_TOKEN", None)]);
        let err = resolve_token().unwrap_err();
        assert!(matches!(err, FetchError::Failed(m) if m == "GITLAB_TOKEN not set"));
    }

    #[test]
    fn http_reuses_the_same_client() {
        assert!(std::ptr::eq(http(), http()));
    }

    #[test]
    fn resolve_authenticated_username_returns_cached_value() {
        let _g = EnvGuard::set(&[("GITLAB_TOKEN", None)]);
        clear_authenticated_username_cache();
        *AUTHENTICATED_USERNAME_CACHE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap() = Some(("gitlab.com".into(), "root".into()));

        let user = run_async(resolve_authenticated_username("gitlab.com")).unwrap();
        assert_eq!(user, "root");
        clear_authenticated_username_cache();
    }

    #[test]
    fn resolve_authenticated_username_busts_cache_per_host() {
        // A cache hit must match the requested host; otherwise switching configs between two
        // GitLab instances would bleed identities. Requesting a different host than the cached
        // one falls through to `rest_get`, which (without a token) returns the auth error.
        let _g = EnvGuard::set(&[("GITLAB_TOKEN", None)]);
        clear_authenticated_username_cache();
        *AUTHENTICATED_USERNAME_CACHE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap() = Some(("gitlab.com".into(), "root".into()));

        let err = run_async(resolve_authenticated_username("gitlab.example.org")).unwrap_err();
        assert!(matches!(err, FetchError::Failed(m) if m == "GITLAB_TOKEN not set"));
        clear_authenticated_username_cache();
    }

    #[test]
    fn rest_get_send_error_surfaces_request_failed() {
        // `\n` in a PRIVATE-TOKEN header forces reqwest to fail at builder time, so the send-
        // error arm is reachable without hitting the live API.
        let _g = EnvGuard::set(&[("GITLAB_TOKEN", Some("tok\nbreak"))]);
        let err = run_async(rest_get::<TestPayload>("gitlab.com", "/user")).unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(m) if m.starts_with("gitlab request failed:")
        ));
    }

    #[test]
    fn parse_json_deserializes_success_bodies() {
        let payload = parse_test_payload("200 OK", r#"{"username":"alice"}"#).unwrap();
        assert_eq!(payload.username, "alice");
    }

    #[test]
    fn parse_json_surfaces_string_message_field() {
        let err =
            parse_test_payload("404 Not Found", r#"{"message":"404 Not Found"}"#).unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(m) if m == "gitlab 404 Not Found: 404 Not Found"
        ));
    }

    #[test]
    fn parse_json_handles_structured_message_objects() {
        // GitLab sometimes returns `{"message": {"error": "...", "details": [...]}}` rather
        // than a flat string; the error wrapper should stringify gracefully instead of dropping
        // it.
        let err = parse_test_payload(
            "400 Bad Request",
            r#"{"message":{"error":"invalid","scope":["user"]}}"#,
        )
        .unwrap_err();
        let FetchError::Failed(m) = err else {
            panic!("expected Failed");
        };
        assert!(m.starts_with("gitlab 400 Bad Request:"));
    }

    #[test]
    fn parse_json_falls_back_to_error_field() {
        let err =
            parse_test_payload("401 Unauthorized", r#"{"error":"invalid_token"}"#).unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(m) if m == "gitlab 401 Unauthorized: invalid_token"
        ));
    }

    #[test]
    fn parse_json_falls_back_to_status_without_a_message() {
        let err = parse_test_payload("500 Internal Server Error", "{}").unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(m) if m == "gitlab 500 Internal Server Error"
        ));
    }

    #[test]
    fn parse_json_surfaces_json_parse_errors() {
        let err = parse_test_payload("200 OK", "not-json").unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(m) if m.contains("gitlab json parse")
        ));
    }

    fn parse_test_payload(status: &str, body: &str) -> Result<TestPayload, FetchError> {
        let (url, server) = serve_once(status, body);
        let response = run_async(async {
            let response = http().get(&url).send().await.unwrap();
            parse_json::<TestPayload>(response).await
        });
        server.join().unwrap();
        response
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
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });
        (format!("http://{addr}"), handle)
    }
}
