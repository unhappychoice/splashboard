//! Shared HTTP client + auth + API DTOs for the `deariary_*` fetcher family. The base host
//! `api.deariary.com` is hardcoded so config can never redirect the bearer token to a
//! third-party origin — that's why every fetcher in this family classifies as `Safety::Safe`.
//!
//! On top of the raw HTTP helpers there is a thin in-process layer that:
//! - **deduplicates concurrent calls** for the same `/entries/:date` (or list query) across
//!   widgets that differ only in shape / display options. Without it, two widgets viewing
//!   today as Badge + Markdown would trigger two HTTP calls for identical data.
//! - **caches successful responses** for 60s so back-to-back refreshes inside the burst
//!   window reuse the body. The splashboard payload cache keeps shape-specific copies for
//!   longer; this layer is just the burst smoother in front of it.
//! - **bounds concurrency** via a semaphore (4 in flight). The documented limit is
//!   120 req/min per key (rolling window, no burst sub-limit) so 10 calls per refresh
//!   cycle leaves plenty of headroom; the cap is mainly so `on_this_day`'s 8-anchor fan-
//!   out doesn't spike disk / network for a moment. If a 429 does fire (token shared with
//!   another client, repeatedly nuked payload cache, etc.), the server's `Retry-After` is
//!   honoured with a single retry capped at the request timeout.
//!
//! 404 has a dedicated success path (`Ok(None)`): for `/entries/:date` it just means "no
//! entry that day", which `deariary_today` and `deariary_on_this_day` treat as empty content.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::sync::{Mutex as AsyncMutex, Semaphore};

use crate::fetcher::FetchError;

pub const API_BASE: &str = "https://api.deariary.com/api/v1";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const RESPONSE_TTL: Duration = Duration::from_secs(60);
// Documented API limit is 120 req/min per key (no per-second sub-limit), so 8 parallel
// `on_this_day` anchors are well within budget. The cap is here so a fan-out doesn't pile
// FDs / sockets on the splashboard side, not because the API needs serialisation.
const MAX_CONCURRENT_REQUESTS: usize = 4;
// Cap on the `Retry-After` sleep when 429 fires. The server can ask for tens of seconds
// (the docs example says "Retry after 24 seconds"), but the per-fetch timeout would kill
// us before then anyway, so we cap at the same 10s window.
const RETRY_AFTER_CAP: Duration = Duration::from_secs(10);
pub const MAX_LIST_LIMIT: u32 = 100;

#[derive(Debug, Clone, Deserialize)]
pub struct ApiEntry {
    pub date: String,
    pub title: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(rename = "generatedAt", default)]
    pub generated_at: Option<String>,
    #[serde(rename = "wordCount", default)]
    pub word_count: Option<u32>,
}

/// Public deariary web URL for an entry on the given ISO date (`YYYY-MM-DD`). The web app
/// segments the path as `YYYY/MM/DD`, so we transform the dash-separated form into slashes.
/// Used by `LinkedTextBlock` shapes across the family so terminals that honour OSC 8
/// hyperlinks open the entry page directly.
pub fn entry_url(date: &str) -> String {
    let path = date.replace('-', "/");
    format!("https://app.deariary.com/entries/{path}")
}

/// Stable opaque scope string for a token. Used to namespace both the in-process slot cache
/// keys and the per-fetcher disk `cache_key` so account A's cached entries can't be served
/// to a widget configured against account B's token. Hashing avoids two distinct concerns
/// in one go: keeps the raw token out of the cache-key strings (which surface in logs /
/// file paths under `cache/`) and dodges any collision risk from a literal `|` separator
/// in the token. We take only the first 16 hex chars of SHA-256 — collisions are vanishingly
/// unlikely at that width and short keys keep the on-disk cache filenames reasonable.
/// Callers that don't have a token (resolution failed earlier) pass `""`, yielding a stable
/// empty-token scope rather than skipping the namespace entirely.
pub fn token_scope(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(token.as_bytes());
    let mut s = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        use std::fmt::Write;
        let _ = write!(s, "{byte:02x}");
    }
    s
}

/// Cache-key `extra` partition string for the deariary family. Resolves the token via the
/// same precedence as `fetch` (config option > `DEARIARY_TOKEN` env), then prefixes the raw
/// options blob with the scoped token hash so changing accounts produces distinct disk
/// cache files even when the token only lives in env (cf. CodeRabbit review on PR #176 —
/// without this, two users sharing a `$HOME/.splashboard/cache` directory would observe
/// each other's diary entries). Token-resolution failure folds to the empty scope; the
/// subsequent fetch surfaces the missing-token error via `error_placeholder`.
pub fn cache_extra(opts_token: Option<&str>, raw_opts: Option<&toml::Value>) -> String {
    let resolved = resolve_token(opts_token).unwrap_or_default();
    let opts_str = raw_opts
        .and_then(|v| toml::to_string(v).ok())
        .unwrap_or_default();
    format!("{}|{}", token_scope(&resolved), opts_str)
}

pub fn resolve_token(config_token: Option<&str>) -> Result<String, FetchError> {
    let trimmed = config_token
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var("DEARIARY_TOKEN")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
    trimmed.ok_or_else(|| {
        FetchError::Failed("deariary token missing: set options.token or DEARIARY_TOKEN".into())
    })
}

/// Fetches `/entries/:date` with deduplication + 60s cache. Returns `Ok(None)` on 404.
pub async fn cached_get_entry(token: &str, date: &str) -> Result<Option<ApiEntry>, FetchError> {
    let slot = entry_slot(token, date);
    let mut guard = slot.lock().await;
    if let Some(cached) = guard.as_ref()
        && Instant::now() < cached.expires
    {
        return Ok(cached.value.clone());
    }
    let _permit = acquire_permit().await?;
    let value = get_optional::<ApiEntry>(token, &format!("/entries/{date}")).await?;
    *guard = Some(CacheSlot {
        expires: Instant::now() + RESPONSE_TTL,
        value: value.clone(),
    });
    Ok(value)
}

/// Fetches `/entries?limit=100&tag=…` with deduplication + 60s cache. Always asks the API
/// for the maximum page size so widgets that differ only on `limit` share one HTTP call;
/// callers slice the returned vector locally.
pub async fn cached_get_entries(
    token: &str,
    tag: Option<&str>,
) -> Result<Vec<ApiEntry>, FetchError> {
    let slot = list_slot(token, tag.unwrap_or(""));
    let mut guard = slot.lock().await;
    if let Some(cached) = guard.as_ref()
        && Instant::now() < cached.expires
    {
        return Ok(cached.value.clone());
    }
    let _permit = acquire_permit().await?;
    let mut query: Vec<(&str, String)> = vec![("limit", MAX_LIST_LIMIT.to_string())];
    if let Some(t) = tag.filter(|s| !s.is_empty()) {
        query.push(("tag", t.to_string()));
    }
    let response: EntriesResponse = get_required(token, "/entries", &query).await?;
    let value = response.into_vec();
    *guard = Some(CacheSlot {
        expires: Instant::now() + RESPONSE_TTL,
        value: value.clone(),
    });
    Ok(value)
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
        .map_err(|e| FetchError::Failed(format!("deariary semaphore poisoned: {e}")))
}

struct CacheSlot<T> {
    expires: Instant,
    value: T,
}

type EntrySlot = Arc<AsyncMutex<Option<CacheSlot<Option<ApiEntry>>>>>;
type ListSlot = Arc<AsyncMutex<Option<CacheSlot<Vec<ApiEntry>>>>>;

fn entry_cache() -> &'static StdMutex<HashMap<String, EntrySlot>> {
    static C: OnceLock<StdMutex<HashMap<String, EntrySlot>>> = OnceLock::new();
    C.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn list_cache() -> &'static StdMutex<HashMap<String, ListSlot>> {
    static C: OnceLock<StdMutex<HashMap<String, ListSlot>>> = OnceLock::new();
    C.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// Cache key scopes the per-date slot to the token so a process holding multiple tokens
/// (e.g. one widget per account) doesn't serve account A's entries to account B's widget.
/// The token is hashed via [`token_scope`] before becoming part of the key — defends
/// against `|` colliding two distinct (token, date) pairs into one slot, and keeps the raw
/// token out of the in-memory key string.
fn entry_slot(token: &str, date: &str) -> EntrySlot {
    entry_cache()
        .lock()
        .unwrap()
        .entry(format!("{}|{date}", token_scope(token)))
        .or_insert_with(|| Arc::new(AsyncMutex::new(None)))
        .clone()
}

fn list_slot(token: &str, tag: &str) -> ListSlot {
    list_cache()
        .lock()
        .unwrap()
        .entry(format!("{}|{tag}", token_scope(token)))
        .or_insert_with(|| Arc::new(AsyncMutex::new(None)))
        .clone()
}

async fn get_optional<T: DeserializeOwned>(
    token: &str,
    path: &str,
) -> Result<Option<T>, FetchError> {
    let bytes = match send_with_retry(token, path, &[]).await? {
        FetchOutcome::NotFound => return Ok(None),
        FetchOutcome::Body(b) => b,
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| FetchError::Failed(format!("deariary json parse: {e}")))
}

async fn get_required<T: DeserializeOwned>(
    token: &str,
    path: &str,
    query: &[(&str, String)],
) -> Result<T, FetchError> {
    match send_with_retry(token, path, query).await? {
        FetchOutcome::NotFound => Err(FetchError::Failed(format!("deariary {path}: 404"))),
        FetchOutcome::Body(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| FetchError::Failed(format!("deariary json parse: {e}"))),
    }
}

enum FetchOutcome {
    NotFound,
    Body(Vec<u8>),
}

/// One retry on 429: read `Retry-After` (seconds), sleep, then re-issue. Capped at
/// `RETRY_AFTER_CAP` so a misbehaving header doesn't stall the splash for minutes.
async fn send_with_retry(
    token: &str,
    path: &str,
    query: &[(&str, String)],
) -> Result<FetchOutcome, FetchError> {
    let mut attempt = 0u8;
    loop {
        let url = format!("{API_BASE}{path}");
        let res = http()
            .get(&url)
            .bearer_auth(token)
            .query(query)
            .send()
            .await
            .map_err(|e| FetchError::Failed(format!("deariary request failed: {e}")))?;
        let status = res.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(FetchOutcome::NotFound);
        }
        if status == StatusCode::TOO_MANY_REQUESTS && attempt == 0 {
            let wait = parse_retry_after(res.headers()).min(RETRY_AFTER_CAP);
            tracing::warn!(
                limit = header_str(res.headers(), "X-RateLimit-Limit").as_deref(),
                remaining = header_str(res.headers(), "X-RateLimit-Remaining").as_deref(),
                reset = header_str(res.headers(), "X-RateLimit-Reset").as_deref(),
                retry_after_s = wait.as_secs(),
                path,
                "deariary 429: backing off before retry",
            );
            tokio::time::sleep(wait).await;
            attempt += 1;
            continue;
        }
        let bytes = res
            .bytes()
            .await
            .map_err(|e| FetchError::Failed(format!("deariary response body: {e}")))?;
        if !status.is_success() {
            return Err(FetchError::Failed(error_message(status, &bytes)));
        }
        return Ok(FetchOutcome::Body(bytes.to_vec()));
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

fn header_str(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum EntriesResponse {
    Wrapped { data: Vec<ApiEntry> },
    Plain(Vec<ApiEntry>),
}

impl EntriesResponse {
    fn into_vec(self) -> Vec<ApiEntry> {
        match self {
            Self::Wrapped { data } => data,
            Self::Plain(items) => items,
        }
    }
}

fn error_message(status: StatusCode, body: &[u8]) -> String {
    #[derive(Deserialize)]
    struct Problem {
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        detail: Option<String>,
    }
    let reported = serde_json::from_slice::<Problem>(body)
        .ok()
        .and_then(|p| p.detail.or(p.title));
    match reported {
        Some(m) => format!("deariary {status}: {m}"),
        None => format!("deariary {status}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_response_accepts_bare_array() {
        let raw = r#"[{"date":"2026-04-27","title":"Hello"}]"#;
        let parsed: EntriesResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.into_vec().len(), 1);
    }

    #[test]
    fn entries_response_accepts_wrapped_data() {
        let raw = r#"{"data":[{"date":"2026-04-27","title":"Hello"}]}"#;
        let parsed: EntriesResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.into_vec().len(), 1);
    }

    #[test]
    fn api_entry_tolerates_missing_optional_fields() {
        let raw = r#"{"date":"2026-04-27","title":"Hello"}"#;
        let entry: ApiEntry = serde_json::from_str(raw).unwrap();
        assert_eq!(entry.date, "2026-04-27");
        assert!(entry.content.is_none());
        assert!(entry.tags.is_empty());
        assert!(entry.word_count.is_none());
    }

    #[test]
    fn api_entry_reads_full_payload() {
        let raw = r##"{
            "date":"2026-04-27",
            "title":"My Day",
            "content":"# Hello\n\nWorld",
            "tags":["work","travel"],
            "sources":["github","calendar"],
            "generatedAt":"2026-04-27T08:00:00Z",
            "wordCount":482
        }"##;
        let entry: ApiEntry = serde_json::from_str(raw).unwrap();
        assert_eq!(entry.tags, vec!["work", "travel"]);
        assert_eq!(entry.sources, vec!["github", "calendar"]);
        assert_eq!(entry.word_count, Some(482));
        assert_eq!(entry.generated_at.as_deref(), Some("2026-04-27T08:00:00Z"));
    }

    #[test]
    fn error_message_picks_problem_detail_over_title() {
        let body = br#"{"title":"Bad Request","detail":"date out of range"}"#;
        let msg = error_message(StatusCode::BAD_REQUEST, body);
        assert!(msg.contains("date out of range"));
    }

    #[test]
    fn error_message_falls_back_to_title_when_detail_missing() {
        let body = br#"{"title":"Forbidden"}"#;
        let msg = error_message(StatusCode::FORBIDDEN, body);
        assert!(msg.contains("Forbidden"));
    }

    #[test]
    fn error_message_handles_non_problem_body() {
        let msg = error_message(StatusCode::INTERNAL_SERVER_ERROR, b"internal explosion");
        assert!(msg.contains("500"));
    }

    #[test]
    fn resolve_token_prefers_config_value_over_env() {
        let token = resolve_token(Some("from-config")).unwrap();
        assert_eq!(token, "from-config");
    }

    #[test]
    fn entry_url_uses_app_subdomain_with_slash_separated_date() {
        assert_eq!(
            entry_url("2026-04-27"),
            "https://app.deariary.com/entries/2026/04/27"
        );
    }

    #[test]
    fn token_scope_is_stable_collision_resistant_and_obscures_raw_token() {
        let a = token_scope("tok-A");
        let b = token_scope("tok-A");
        assert_eq!(a, b, "scope must be deterministic for repeat calls");
        assert_ne!(token_scope("tok-A"), token_scope("tok-B"));
        // 16 hex chars = 64 bits of entropy. Plenty for distinguishing local accounts.
        assert_eq!(a.len(), 16);
        // Token-content-derived but not the token itself — defends `cache/` filenames /
        // log lines that surface the scope from leaking the bearer.
        assert!(!a.contains("tok-A"));
    }

    #[test]
    fn token_scope_dodges_naive_pipe_separator_collision() {
        // The slot key was previously `format!("{token}|{date}")`. A token containing `|`
        // could collide with an entirely different (token, date) pair. Hashed scope removes
        // the ambiguity — `a|b` and `a` against date `|b` no longer hash to the same bucket.
        assert_ne!(token_scope("a|b"), token_scope("a"));
    }

    #[test]
    fn cache_extra_partitions_disk_cache_per_resolved_token() {
        // Direct config tokens — different tokens must produce different extras even with
        // the same options blob, otherwise account A's disk cache leaks to account B.
        let a = cache_extra(Some("tok-A"), None);
        let b = cache_extra(Some("tok-B"), None);
        assert_ne!(
            a, b,
            "different config tokens must yield different cache extras",
        );
    }

    #[test]
    fn cache_extra_includes_options_blob_for_per_invocation_keys() {
        // Same token, different options (e.g. different `tag`) → different extras so
        // cached entries don't bleed across invocation parameters.
        let v1: toml::Value = toml::from_str("tag = \"work\"").unwrap();
        let v2: toml::Value = toml::from_str("tag = \"travel\"").unwrap();
        let a = cache_extra(Some("tok-A"), Some(&v1));
        let b = cache_extra(Some("tok-A"), Some(&v2));
        assert_ne!(a, b);
    }

    #[test]
    fn cache_extra_includes_a_token_scope_prefix() {
        // Structural assertion: extra always begins with the scoped token followed by `|`,
        // so even when DEARIARY_TOKEN flips out from under us mid-cache-lifetime, the new
        // run produces a distinct cache key (we don't read stale account-A data into
        // account-B's slot). Asserting the prefix shape rather than the exact value sidesteps
        // env races with parallel tests.
        let extra = cache_extra(Some("tok-A"), None);
        let scope = token_scope("tok-A");
        assert!(extra.starts_with(&format!("{scope}|")), "got: {extra:?}");
    }

    #[tokio::test]
    async fn entry_slot_returns_same_arc_for_same_token_and_date() {
        let a = entry_slot("tok-A", "2026-04-27");
        let b = entry_slot("tok-A", "2026-04-27");
        assert!(Arc::ptr_eq(&a, &b));
        let c = entry_slot("tok-A", "2026-04-26");
        assert!(!Arc::ptr_eq(&a, &c));
    }

    #[tokio::test]
    async fn entry_slot_isolates_per_token() {
        let a = entry_slot("tok-A", "2026-04-27");
        let b = entry_slot("tok-B", "2026-04-27");
        assert!(
            !Arc::ptr_eq(&a, &b),
            "different tokens must not share a cache slot",
        );
    }

    #[tokio::test]
    async fn list_slot_returns_same_arc_for_same_token_and_tag() {
        let a = list_slot("tok-A", "travel");
        let b = list_slot("tok-A", "travel");
        assert!(Arc::ptr_eq(&a, &b));
        let c = list_slot("tok-A", "");
        assert!(!Arc::ptr_eq(&a, &c));
    }

    #[tokio::test]
    async fn list_slot_isolates_per_token() {
        let a = list_slot("tok-A", "travel");
        let b = list_slot("tok-B", "travel");
        assert!(
            !Arc::ptr_eq(&a, &b),
            "different tokens must not share a list cache slot",
        );
    }

    #[test]
    fn resolve_token_trims_whitespace() {
        let token = resolve_token(Some("  abc  ")).unwrap();
        assert_eq!(token, "abc");
    }

    #[test]
    fn resolve_token_rejects_whitespace_only_config_value() {
        // Whitespace-only config doesn't take precedence; falls through to env (which we
        // don't set here in the unit test, so this should fail). Asserts the trim-and-empty
        // check kicks in.
        let result = resolve_token(Some("   "));
        // Either falls back to env (if DEARIARY_TOKEN is set in CI) or errors. We just need
        // the config value not to be returned verbatim.
        if let Ok(t) = result {
            assert!(!t.trim().is_empty(), "should not return whitespace-only");
            assert_ne!(t, "   ");
        }
    }
}
