//! Shared HTTP client + base URL for the `hackernews_*` family. Targets HN's read-only
//! Firebase backend (`hacker-news.firebaseio.com/v0`) — fixed host, no auth, so every fetcher
//! in the family stays Safety::Safe regardless of what config supplies.

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Client;
use serde::de::DeserializeOwned;

use crate::fetcher::FetchError;

pub const API_BASE: &str = "https://hacker-news.firebaseio.com/v0";
pub const HN_ITEM_URL: &str = "https://news.ycombinator.com/item?id=";
pub const HN_USER_URL: &str = "https://news.ycombinator.com/user?id=";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

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

pub async fn get<T: DeserializeOwned>(url: &str) -> Result<T, FetchError> {
    let res = http()
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("hn request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("hn {status}")));
    }
    res.json()
        .await
        .map_err(|e| FetchError::Failed(format!("hn json parse: {e}")))
}
