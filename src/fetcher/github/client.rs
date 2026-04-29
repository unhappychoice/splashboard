//! Shared HTTP client + auth for the `github_*` fetcher family. Reads `GH_TOKEN` or
//! `GITHUB_TOKEN` (in that order) once per process. Every fetcher talks to `api.github.com`
//! only — there is no config-controlled host, so leaking a user's token to an attacker-chosen
//! origin is not possible by design.
//!
//! The rest helpers accept a path like `"/user"` or `"/repos/foo/bar"`; the base URL is joined
//! here so fetchers stay free of URL plumbing. GraphQL goes through a single POST helper.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::fetcher::FetchError;

const API_BASE: &str = "https://api.github.com";
const GRAPHQL_URL: &str = "https://api.github.com/graphql";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const ACCEPT: &str = "application/vnd.github+json";
const API_VERSION: &str = "2022-11-28";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
static AUTHENTICATED_USER_CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();

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
    std::env::var("GH_TOKEN")
        .or_else(|_| std::env::var("GITHUB_TOKEN"))
        .map_err(|_| FetchError::Failed("GH_TOKEN / GITHUB_TOKEN not set".into()))
}

/// Login of the token-authenticated user. Resolved lazily via `GET /user` and memoised for the
/// rest of the process so we never spend more than one roundtrip on a value that never changes
/// within a session. Lets `github_avatar` / `github_user` work with zero config when a token is
/// already set.
pub async fn resolve_authenticated_user() -> Result<String, FetchError> {
    let slot = AUTHENTICATED_USER_CACHE.get_or_init(|| Mutex::new(None));
    if let Some(cached) = slot.lock().ok().and_then(|g| g.clone()) {
        return Ok(cached);
    }
    #[derive(Deserialize)]
    struct Me {
        login: String,
    }
    let me: Me = rest_get("/user").await?;
    if let Ok(mut g) = slot.lock() {
        *g = Some(me.login.clone());
    }
    Ok(me.login)
}

#[cfg(test)]
pub(crate) fn clear_authenticated_user_cache() {
    if let Ok(mut guard) = AUTHENTICATED_USER_CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
    {
        *guard = None;
    }
}

/// REST GET → deserialize JSON. Non-2xx responses surface the GitHub-reported `message` when
/// present (`{"message":"Not Found"}`) so the runtime's error placeholder and the log line
/// are both actionable.
pub async fn rest_get<T: DeserializeOwned>(path: &str) -> Result<T, FetchError> {
    let token = resolve_token()?;
    let url = format!("{API_BASE}{path}");
    let res = http()
        .get(&url)
        .bearer_auth(&token)
        .header("Accept", ACCEPT)
        .header("X-GitHub-Api-Version", API_VERSION)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("github request failed: {e}")))?;
    parse_json(res).await
}

/// GraphQL POST. GitHub returns 200 even on query errors, so we also look at the `errors` array.
pub async fn graphql<T: DeserializeOwned>(
    query: &str,
    variables: serde_json::Value,
) -> Result<T, FetchError> {
    let token = resolve_token()?;
    let body = serde_json::json!({ "query": query, "variables": variables });
    let res = http()
        .post(GRAPHQL_URL)
        .bearer_auth(&token)
        .header("Accept", ACCEPT)
        .header("X-GitHub-Api-Version", API_VERSION)
        .json(&body)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("github graphql request failed: {e}")))?;
    let wrapper: GqlResponse<T> = parse_json(res).await?;
    if let Some(errs) = wrapper.errors.filter(|e| !e.is_empty()) {
        let joined = errs
            .iter()
            .map(|e| e.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(FetchError::Failed(format!("github graphql: {joined}")));
    }
    wrapper
        .data
        .ok_or_else(|| FetchError::Failed("github graphql: empty data".into()))
}

async fn parse_json<T: DeserializeOwned>(res: reqwest::Response) -> Result<T, FetchError> {
    let status = res.status();
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("github response body: {e}")))?;
    if !status.is_success() {
        return Err(FetchError::Failed(error_message(status, &bytes)));
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("github json parse: {e}")))
}

fn error_message(status: StatusCode, body: &[u8]) -> String {
    #[derive(Deserialize)]
    struct ApiError {
        message: Option<String>,
    }
    let reported = serde_json::from_slice::<ApiError>(body)
        .ok()
        .and_then(|e| e.message);
    match reported {
        Some(m) => format!("github {status}: {m}"),
        None => format!("github {status}"),
    }
}

#[derive(Debug, Deserialize)]
struct GqlResponse<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Option<Vec<GqlError>>,
}

#[derive(Debug, Deserialize)]
struct GqlError {
    message: String,
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
        login: String,
    }

    /// Serialises env mutation with other env-touching tests. `GH_TOKEN` / `GITHUB_TOKEN` are
    /// read unconditionally inside `resolve_token`, so two parallel tests racing on the same
    /// vars would otherwise flap.
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

    #[test]
    fn resolve_token_prefers_gh_token() {
        let _g = EnvGuard::set(&[
            ("GH_TOKEN", Some("from-gh")),
            ("GITHUB_TOKEN", Some("from-github")),
        ]);
        assert_eq!(resolve_token().unwrap(), "from-gh");
    }

    #[test]
    fn resolve_token_falls_back_to_github_token() {
        let _g = EnvGuard::set(&[("GH_TOKEN", None), ("GITHUB_TOKEN", Some("fallback"))]);
        assert_eq!(resolve_token().unwrap(), "fallback");
    }

    #[test]
    fn resolve_token_fails_when_both_missing() {
        let _g = EnvGuard::set(&[("GH_TOKEN", None), ("GITHUB_TOKEN", None)]);
        assert!(resolve_token().is_err());
    }

    #[test]
    fn http_reuses_the_same_client() {
        assert!(std::ptr::eq(http(), http()));
    }

    #[test]
    fn resolve_authenticated_user_returns_cached_login() {
        let _g = EnvGuard::set(&[("GH_TOKEN", None), ("GITHUB_TOKEN", None)]);
        clear_authenticated_user_cache();
        *AUTHENTICATED_USER_CACHE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap() = Some("octocat".into());

        assert_eq!(run_async(resolve_authenticated_user()).unwrap(), "octocat");
        clear_authenticated_user_cache();
    }

    #[test]
    fn resolve_authenticated_user_requires_a_token_without_cache() {
        let _g = EnvGuard::set(&[("GH_TOKEN", None), ("GITHUB_TOKEN", None)]);
        clear_authenticated_user_cache();

        let err = run_async(resolve_authenticated_user()).unwrap_err();

        assert!(matches!(
            err,
            FetchError::Failed(message) if message == "GH_TOKEN / GITHUB_TOKEN not set"
        ));
    }

    #[test]
    fn graphql_requires_a_token_before_sending() {
        let _g = EnvGuard::set(&[("GH_TOKEN", None), ("GITHUB_TOKEN", None)]);

        let err = run_async(graphql::<TestPayload>(
            "query { viewer { login } }",
            serde_json::json!({}),
        ))
        .unwrap_err();

        assert!(matches!(
            err,
            FetchError::Failed(message) if message == "GH_TOKEN / GITHUB_TOKEN not set"
        ));
    }

    #[test]
    fn parse_json_deserializes_success_bodies() {
        let payload = parse_test_payload("200 OK", r#"{"login":"octocat"}"#).unwrap();
        assert_eq!(payload.login, "octocat");
    }

    #[test]
    fn parse_json_surfaces_reported_api_messages() {
        let err = parse_test_payload("404 Not Found", r#"{"message":"Not Found"}"#).unwrap_err();

        assert!(matches!(
            err,
            FetchError::Failed(message) if message == "github 404 Not Found: Not Found"
        ));
    }

    #[test]
    fn parse_json_falls_back_to_status_without_a_message() {
        let err = parse_test_payload("500 Internal Server Error", "{}").unwrap_err();

        assert!(matches!(
            err,
            FetchError::Failed(message) if message == "github 500 Internal Server Error"
        ));
    }

    #[test]
    fn parse_json_surfaces_json_parse_errors() {
        let err = parse_test_payload("200 OK", "not-json").unwrap_err();

        assert!(matches!(
            err,
            FetchError::Failed(message) if message.contains("github json parse")
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

    fn run_async<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }
}
