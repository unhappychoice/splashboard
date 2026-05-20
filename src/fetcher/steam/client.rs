//! Shared HTTP client + auth for the `steam_*` fetcher family.
//!
//! Targets `api.steampowered.com` (Web API) and `store.steampowered.com` (store appdetails
//! used by `steam_charts` to resolve appid → game name). Both hosts are hardcoded — config
//! can only change *which resource* on those hosts is queried, never the host itself, so the
//! family stays `Safety::Safe`.
//!
//! Auth: `STEAM_API_KEY` (free, https://steamcommunity.com/dev/apikey). The chart endpoint
//! [`get_json_public`] is open and skips the key entirely.

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Client;
use serde::de::DeserializeOwned;

use crate::fetcher::FetchError;

const API_BASE: &str = "https://api.steampowered.com";
const STORE_BASE: &str = "https://store.steampowered.com";
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
    std::env::var("STEAM_API_KEY").map_err(|_| FetchError::Failed("STEAM_API_KEY not set".into()))
}

/// Resolve a Steam ID from explicit option or `STEAM_ID` env. Mirrors steamfetch's resolution
/// order so the same env works across both tools. Returns a trimmed non-empty `String`.
pub fn resolve_steam_id(explicit: Option<&str>) -> Result<String, FetchError> {
    let from_option = explicit.map(str::trim).filter(|s| !s.is_empty());
    if let Some(id) = from_option {
        return Ok(id.to_string());
    }
    std::env::var("STEAM_ID")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            FetchError::Failed(
                "steam_id missing: set `steam_id = \"<id64>\"` or export STEAM_ID".into(),
            )
        })
}

/// Authenticated `GET https://api.steampowered.com/<path>?key=<key>&<params>`.
pub async fn get_json<T: DeserializeOwned>(
    path: &str,
    params: &[(&str, &str)],
) -> Result<T, FetchError> {
    let key = resolve_api_key()?;
    let mut url = url::Url::parse(&format!("{API_BASE}/{path}"))
        .map_err(|e| FetchError::Failed(format!("steam invalid path {path:?}: {e}")))?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("key", &key);
        for (k, v) in params {
            q.append_pair(k, v);
        }
    }
    request_json(url.as_str()).await
}

/// Unauthenticated `GET https://api.steampowered.com/<path>?<params>`. Used by `steam_charts`
/// (`ISteamChartsService/GetMostPlayedGames`), which is public and rejects requests that carry
/// a stray `key=` parameter on some shards.
pub async fn get_json_public<T: DeserializeOwned>(
    path: &str,
    params: &[(&str, &str)],
) -> Result<T, FetchError> {
    let mut url = url::Url::parse(&format!("{API_BASE}/{path}"))
        .map_err(|e| FetchError::Failed(format!("steam invalid path {path:?}: {e}")))?;
    {
        let mut q = url.query_pairs_mut();
        for (k, v) in params {
            q.append_pair(k, v);
        }
    }
    request_json(url.as_str()).await
}

/// `GET https://store.steampowered.com/<path>?<params>`. Used to resolve appid → game name +
/// header image for the chart fetcher.
pub async fn get_store_json<T: DeserializeOwned>(
    path: &str,
    params: &[(&str, &str)],
) -> Result<T, FetchError> {
    let mut url = url::Url::parse(&format!("{STORE_BASE}/{path}"))
        .map_err(|e| FetchError::Failed(format!("steam store invalid path {path:?}: {e}")))?;
    {
        let mut q = url.query_pairs_mut();
        for (k, v) in params {
            q.append_pair(k, v);
        }
    }
    request_json(url.as_str()).await
}

async fn request_json<T: DeserializeOwned>(url: &str) -> Result<T, FetchError> {
    let res = http()
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("steam request failed: {e}")))?;
    parse_json(res).await
}

async fn parse_json<T: DeserializeOwned>(res: reqwest::Response) -> Result<T, FetchError> {
    let status = res.status();
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("steam read body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "steam response too large ({} bytes, cap {MAX_BYTES})",
            bytes.len()
        )));
    }
    if !status.is_success() {
        return Err(FetchError::Failed(format!("steam {status}")));
    }
    serde_json::from_slice(&bytes).map_err(|e| FetchError::Failed(format!("steam json parse: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_api_key_fails_with_clear_message_when_unset() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("STEAM_API_KEY").ok();
        unsafe { std::env::remove_var("STEAM_API_KEY") };
        let err = resolve_api_key().unwrap_err();
        assert!(matches!(err, FetchError::Failed(m) if m == "STEAM_API_KEY not set"));
        if let Some(v) = prev {
            unsafe { std::env::set_var("STEAM_API_KEY", v) };
        }
    }

    #[test]
    fn resolve_api_key_reads_env() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("STEAM_API_KEY").ok();
        unsafe { std::env::set_var("STEAM_API_KEY", "abc123") };
        assert_eq!(resolve_api_key().unwrap(), "abc123");
        match prev {
            Some(v) => unsafe { std::env::set_var("STEAM_API_KEY", v) },
            None => unsafe { std::env::remove_var("STEAM_API_KEY") },
        }
    }

    #[test]
    fn resolve_steam_id_prefers_explicit_option_over_env() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("STEAM_ID").ok();
        unsafe { std::env::set_var("STEAM_ID", "env-id") };
        assert_eq!(resolve_steam_id(Some("option-id")).unwrap(), "option-id");
        match prev {
            Some(v) => unsafe { std::env::set_var("STEAM_ID", v) },
            None => unsafe { std::env::remove_var("STEAM_ID") },
        }
    }

    #[test]
    fn resolve_steam_id_falls_back_to_env_when_option_missing_or_blank() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("STEAM_ID").ok();
        unsafe { std::env::set_var("STEAM_ID", "env-id") };
        assert_eq!(resolve_steam_id(None).unwrap(), "env-id");
        assert_eq!(resolve_steam_id(Some("   ")).unwrap(), "env-id");
        match prev {
            Some(v) => unsafe { std::env::set_var("STEAM_ID", v) },
            None => unsafe { std::env::remove_var("STEAM_ID") },
        }
    }

    #[test]
    fn resolve_steam_id_errors_when_neither_set() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("STEAM_ID").ok();
        unsafe { std::env::remove_var("STEAM_ID") };
        let err = resolve_steam_id(None).unwrap_err();
        let FetchError::Failed(msg) = err else {
            panic!("expected FetchError::Failed");
        };
        assert!(msg.contains("steam_id"));
        if let Some(v) = prev {
            unsafe { std::env::set_var("STEAM_ID", v) };
        }
    }

    #[test]
    fn http_reuses_the_same_client() {
        assert!(std::ptr::eq(http(), http()));
    }
}
