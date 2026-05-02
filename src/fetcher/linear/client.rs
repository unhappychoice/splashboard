//! Shared GraphQL client + auth + cache-key helpers for the `linear_*` fetcher family. The
//! base host `api.linear.app` is hardcoded so config can never redirect the personal API key
//! to a third-party origin — that's why every fetcher in this family classifies as
//! `Safety::Safe`.
//!
//! Linear authenticates personal API keys (`lin_api_*`) by sending the raw key in the
//! `Authorization` header — without a `Bearer` prefix. OAuth tokens use `Bearer`; we don't
//! ship an OAuth flow because splashboard's startup window has no place to host a callback.
//!
//! On top of the raw HTTP call there's:
//! - a 4-permit semaphore so a future multi-widget splash doesn't fan out beyond Linear's
//!   complexity-based rate limit;
//! - one `Retry-After`-honoured retry on 429, capped at the request timeout so a misbehaving
//!   header can't stall the splash.

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::fetcher::FetchError;

pub const API_URL: &str = "https://api.linear.app/graphql";
pub const APP_BASE: &str = "https://linear.app";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CONCURRENT_REQUESTS: usize = 4;
const RETRY_AFTER_CAP: Duration = Duration::from_secs(15);

pub fn resolve_token(config_token: Option<&str>) -> Result<String, FetchError> {
    config_token
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var("LINEAR_TOKEN")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .ok_or_else(|| {
            FetchError::Failed("linear token missing: set options.token or LINEAR_TOKEN".into())
        })
}

/// Stable opaque scope string for a token. Mirrors the deariary helper: 16 hex chars of
/// SHA-256, used to namespace disk cache keys so account A's cached payloads can't be served
/// to account B's widget.
pub fn token_scope(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut s = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        use std::fmt::Write;
        let _ = write!(s, "{byte:02x}");
    }
    s
}

/// Cache-key `extra` partition for the linear family. Resolves the token via the same
/// precedence as `fetch` (config option > `LINEAR_TOKEN` env), then prefixes the
/// (token-stripped) options blob with the scoped token hash so two users sharing a
/// `$HOME/.splashboard/cache` directory can't observe each other's payloads.
pub fn cache_extra(opts_token: Option<&str>, raw_opts: Option<&toml::Value>) -> String {
    let resolved = resolve_token(opts_token).unwrap_or_default();
    let opts_str = raw_opts
        .map(strip_token_field)
        .and_then(|v| toml::to_string(&v).ok())
        .unwrap_or_default();
    format!("{}|{}", token_scope(&resolved), opts_str)
}

fn strip_token_field(value: &toml::Value) -> toml::Value {
    match value {
        toml::Value::Table(table) => {
            let mut copy = table.clone();
            copy.remove("token");
            toml::Value::Table(copy)
        }
        other => other.clone(),
    }
}

pub fn issue_url(workspace: &str, identifier: &str) -> String {
    format!("{APP_BASE}/{workspace}/issue/{identifier}")
}

pub fn cycle_url(workspace: &str, team_key: &str, cycle_number: i64) -> String {
    format!("{APP_BASE}/{workspace}/team/{team_key}/cycle/{cycle_number}")
}

/// Issue a GraphQL query and deserialize `data` into `T`. Returns [`FetchError::Failed`] on
/// network / HTTP / parse / GraphQL errors.
pub async fn graphql_query<T: DeserializeOwned>(
    token: &str,
    query: &str,
    variables: Value,
) -> Result<T, FetchError> {
    let _permit = acquire_permit().await?;
    let body = json!({ "query": query, "variables": variables });
    let bytes = send_with_retry(token, &body).await?;
    let envelope: GraphqlResponse<T> = serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("linear json parse: {e}")))?;
    if let Some(errors) = envelope.errors.filter(|e| !e.is_empty()) {
        let msg = errors
            .into_iter()
            .map(|e| e.message)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(FetchError::Failed(format!("linear graphql: {msg}")));
    }
    envelope
        .data
        .ok_or_else(|| FetchError::Failed("linear graphql: empty data".into()))
}

#[derive(Debug, Deserialize)]
struct GraphqlResponse<T> {
    #[serde(default = "Option::default")]
    data: Option<T>,
    #[serde(default)]
    errors: Option<Vec<GraphqlError>>,
}

#[derive(Debug, Deserialize)]
struct GraphqlError {
    message: String,
}

async fn send_with_retry(token: &str, body: &Value) -> Result<Vec<u8>, FetchError> {
    let mut attempt = 0u8;
    loop {
        let res = http()
            .post(API_URL)
            .header("Authorization", token)
            .json(body)
            .send()
            .await
            .map_err(|e| FetchError::Failed(format!("linear request failed: {e}")))?;
        let status = res.status();
        if status == StatusCode::TOO_MANY_REQUESTS && attempt == 0 {
            let wait = parse_retry_after(res.headers()).min(RETRY_AFTER_CAP);
            tracing::warn!(retry_after_s = wait.as_secs(), "linear 429: backing off");
            tokio::time::sleep(wait).await;
            attempt += 1;
            continue;
        }
        let bytes = res
            .bytes()
            .await
            .map_err(|e| FetchError::Failed(format!("linear response body: {e}")))?;
        if !status.is_success() {
            return Err(FetchError::Failed(error_message(status, &bytes)));
        }
        return Ok(bytes.to_vec());
    }
}

fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Duration {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(1))
}

fn error_message(status: StatusCode, body: &[u8]) -> String {
    if let Ok(envelope) = serde_json::from_slice::<GraphqlResponse<Value>>(body)
        && let Some(errors) = envelope.errors.filter(|e| !e.is_empty())
    {
        let msg = errors
            .into_iter()
            .map(|e| e.message)
            .collect::<Vec<_>>()
            .join("; ");
        return format!("linear {status}: {msg}");
    }
    format!("linear {status}")
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

fn semaphore() -> &'static Semaphore {
    static S: OnceLock<Semaphore> = OnceLock::new();
    S.get_or_init(|| Semaphore::new(MAX_CONCURRENT_REQUESTS))
}

async fn acquire_permit() -> Result<tokio::sync::SemaphorePermit<'static>, FetchError> {
    semaphore()
        .acquire()
        .await
        .map_err(|e| FetchError::Failed(format!("linear semaphore poisoned: {e}")))
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::net::TcpListener;
    use std::process::Command;

    use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};

    use super::*;

    struct LinearTokenGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        previous: Option<String>,
    }

    impl LinearTokenGuard {
        fn set(value: Option<&str>) -> Self {
            let lock = crate::paths::TEST_ENV_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let previous = std::env::var("LINEAR_TOKEN").ok();
            match value {
                Some(value) => unsafe { std::env::set_var("LINEAR_TOKEN", value) },
                None => unsafe { std::env::remove_var("LINEAR_TOKEN") },
            }
            Self {
                _lock: lock,
                previous,
            }
        }
    }

    impl Drop for LinearTokenGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => unsafe { std::env::set_var("LINEAR_TOKEN", value) },
                None => unsafe { std::env::remove_var("LINEAR_TOKEN") },
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

    fn restore_linear_token(previous: Option<String>) {
        match previous {
            Some(value) => unsafe { std::env::set_var("LINEAR_TOKEN", value) },
            None => unsafe { std::env::remove_var("LINEAR_TOKEN") },
        }
    }

    fn unused_proxy_url() -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{addr}")
    }

    fn run_child_test(filter: &str, envs: &[(&str, &str)]) {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .arg(filter)
            .arg("--nocapture")
            .arg("--test-threads=1");
        envs.iter().for_each(|(key, value)| {
            command.env(key, value);
        });
        let status = command.status().unwrap();
        assert!(status.success(), "child test failed: {status}");
    }

    #[test]
    fn resolve_token_prefers_config_value() {
        let token = resolve_token(Some("from-config")).unwrap();
        assert_eq!(token, "from-config");
    }

    #[test]
    fn resolve_token_trims_whitespace() {
        let token = resolve_token(Some("  abc  ")).unwrap();
        assert_eq!(token, "abc");
    }

    #[test]
    fn resolve_token_falls_back_to_env_value_when_config_is_blank() {
        let _guard = LinearTokenGuard::set(Some("  env-token  "));
        assert_eq!(resolve_token(Some("   ")).unwrap(), "env-token");
        assert_eq!(resolve_token(None).unwrap(), "env-token");
    }

    #[test]
    fn resolve_token_errors_when_config_and_env_are_missing() {
        let _guard = LinearTokenGuard::set(None);
        assert!(matches!(
            resolve_token(None),
            Err(FetchError::Failed(msg)) if msg.contains("linear token missing")
        ));
    }

    #[test]
    fn resolve_token_rejects_blank_env_values() {
        let _guard = LinearTokenGuard::set(Some("   "));
        assert!(matches!(
            resolve_token(None),
            Err(FetchError::Failed(msg)) if msg.contains("linear token missing")
        ));
    }

    #[test]
    fn token_scope_is_deterministic_and_obscures_token() {
        let a = token_scope("lin_api_x");
        let b = token_scope("lin_api_x");
        assert_eq!(a, b);
        assert_ne!(token_scope("lin_api_x"), token_scope("lin_api_y"));
        assert_eq!(a.len(), 16);
        assert!(!a.contains("lin_api_x"));
    }

    #[test]
    fn cache_extra_partitions_per_token() {
        let a = cache_extra(Some("tok-A"), None);
        let b = cache_extra(Some("tok-B"), None);
        assert_ne!(a, b);
    }

    #[test]
    fn cache_extra_strips_token_from_options_blob() {
        let opts: toml::Value =
            toml::from_str("token = \"super-secret\"\nfilter_team = \"ENG\"").unwrap();
        let extra = cache_extra(Some("super-secret"), Some(&opts));
        assert!(!extra.contains("super-secret"), "got: {extra:?}");
        assert!(extra.contains("filter_team = \"ENG\""), "got: {extra:?}");
    }

    #[test]
    fn cache_extra_preserves_nested_token_fields() {
        let opts: toml::Value =
            toml::from_str("token = \"top-secret\"\n[nested]\ntoken = \"inner-secret\"").unwrap();
        let extra = cache_extra(Some("top-secret"), Some(&opts));
        assert!(!extra.contains("top-secret"), "got: {extra:?}");
        assert!(extra.contains("token = \"inner-secret\""), "got: {extra:?}");
    }

    #[test]
    fn cache_extra_uses_empty_scope_when_no_token_is_available() {
        let _guard = LinearTokenGuard::set(None);
        assert_eq!(cache_extra(None, None), format!("{}|", token_scope("")));
    }

    #[test]
    fn strip_token_field_preserves_non_table_values() {
        let value = toml::Value::String("plain".into());
        assert_eq!(strip_token_field(&value), value);
    }

    #[test]
    fn parse_retry_after_reads_numeric_seconds() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static(" 7 "));
        assert_eq!(parse_retry_after(&headers), Duration::from_secs(7));
    }

    #[test]
    fn parse_retry_after_defaults_to_one_second_for_invalid_values() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("soon"));
        assert_eq!(parse_retry_after(&headers), Duration::from_secs(1));
        assert_eq!(parse_retry_after(&HeaderMap::new()), Duration::from_secs(1));
    }

    #[test]
    fn issue_url_uses_workspace_and_identifier() {
        assert_eq!(
            issue_url("acme", "ENG-123"),
            "https://linear.app/acme/issue/ENG-123"
        );
    }

    #[test]
    fn cycle_url_uses_team_key_and_cycle_number() {
        assert_eq!(
            cycle_url("acme", "ENG", 24),
            "https://linear.app/acme/team/ENG/cycle/24"
        );
    }

    #[test]
    fn error_message_extracts_graphql_messages() {
        let body = br#"{"errors":[{"message":"Authentication required"}]}"#;
        let msg = error_message(StatusCode::UNAUTHORIZED, body);
        assert!(msg.contains("Authentication required"));
    }

    #[test]
    fn error_message_falls_back_to_status_when_no_graphql_payload() {
        let msg = error_message(StatusCode::BAD_GATEWAY, b"upstream gone");
        assert!(msg.contains("502"));
    }

    #[test]
    fn graphql_response_defaults_missing_fields() {
        let envelope: GraphqlResponse<serde_json::Value> = serde_json::from_str("{}").unwrap();
        assert!(envelope.data.is_none());
        assert!(envelope.errors.is_none());
    }

    #[test]
    fn linear_token_guard_restores_previous_value_on_drop() {
        let (previous, guard) = {
            let lock = crate::paths::TEST_ENV_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let previous = std::env::var("LINEAR_TOKEN").ok();
            unsafe { std::env::set_var("LINEAR_TOKEN", "before") };
            let guard = LinearTokenGuard {
                _lock: lock,
                previous: Some("before".into()),
            };
            unsafe { std::env::set_var("LINEAR_TOKEN", "during") };
            (previous, guard)
        };

        drop(guard);
        assert_eq!(
            std::env::var("LINEAR_TOKEN").ok().as_deref(),
            Some("before")
        );

        restore_linear_token(previous);
    }

    #[test]
    fn linear_token_guard_set_restores_outer_previous_value() {
        let outer_previous = {
            let _lock = crate::paths::TEST_ENV_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let outer_previous = std::env::var("LINEAR_TOKEN").ok();
            unsafe { std::env::set_var("LINEAR_TOKEN", "outer") };
            outer_previous
        };

        let guard = LinearTokenGuard::set(Some("inner"));
        drop(guard);

        assert_eq!(std::env::var("LINEAR_TOKEN").ok().as_deref(), Some("outer"));
        restore_linear_token(outer_previous);
    }

    #[test]
    fn http_reuses_a_single_client_instance() {
        assert!(std::ptr::eq(http(), http()));
    }

    #[test]
    fn acquire_permit_returns_a_live_permit() {
        let permit = run_async(acquire_permit()).unwrap();
        drop(permit);
    }

    #[test]
    fn graphql_query_surfaces_request_failures_via_child_process() {
        let proxy = unused_proxy_url();
        run_child_test(
            "graphql_query_surfaces_request_failures_child_only",
            &[
                ("SPLASHBOARD_LINEAR_PROXY_CHILD", "1"),
                ("HTTPS_PROXY", proxy.as_str()),
                ("https_proxy", proxy.as_str()),
                ("ALL_PROXY", proxy.as_str()),
                ("all_proxy", proxy.as_str()),
                ("NO_PROXY", ""),
                ("no_proxy", ""),
            ],
        );
    }

    #[test]
    fn graphql_query_surfaces_request_failures_child_only() {
        if std::env::var_os("SPLASHBOARD_LINEAR_PROXY_CHILD").is_none() {
            return;
        }

        let result = run_async(graphql_query::<serde_json::Value>(
            "lin_api_test",
            "query Viewer { viewer { id } }",
            serde_json::json!({}),
        ));

        assert!(matches!(
            result,
            Err(FetchError::Failed(msg)) if msg.contains("linear request failed")
        ));
    }
}
