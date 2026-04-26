//! Shared HTTP client + listing fetch for the `reddit_*` family.
//!
//! Reddit's public `.json` endpoints often 403 a bare reqwest UA, so the request is retried
//! once with a more browser-ish header set before giving up.

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::fetcher::FetchError;

pub(super) const SITE_BASE: &str = "https://www.reddit.com";

const USER_AGENT: &str = concat!(
    "splashboard/",
    env!("CARGO_PKG_VERSION"),
    " (by /u/unhappychoice)"
);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) async fn fetch_listing<T: DeserializeOwned>(
    path_and_query: &str,
) -> Result<Vec<T>, FetchError> {
    let url = format!("{SITE_BASE}{path_and_query}");
    let body = get_with_retry(&url).await?;
    let response: Listing<T> = serde_json::from_str(&body)
        .map_err(|e| FetchError::Failed(format!("reddit json parse: {e}")))?;
    Ok(response
        .data
        .children
        .into_iter()
        .map(|child| child.data)
        .collect())
}

async fn get_with_retry(url: &str) -> Result<String, FetchError> {
    match get_with_profile(url, HeaderProfile::Minimal).await {
        Ok(body) => Ok(body),
        Err(_) => get_with_profile(url, HeaderProfile::Browserish).await,
    }
}

#[derive(Clone, Copy)]
enum HeaderProfile {
    Minimal,
    Browserish,
}

async fn get_with_profile(url: &str, profile: HeaderProfile) -> Result<String, FetchError> {
    let mut req = http().get(url);
    if matches!(profile, HeaderProfile::Browserish) {
        req = req
            .header("accept", "application/json, text/plain, */*")
            .header("accept-language", "en-US,en;q=0.9")
            .header("cache-control", "no-cache")
            .header("pragma", "no-cache")
            .header("referer", "https://www.reddit.com/")
            .header("dnt", "1")
            .header("accept-encoding", "identity");
    }
    let res = req
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("reddit request failed: {e}")))?;
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("reddit {status} @ {url}")));
    }
    if body.is_empty() {
        return Err(FetchError::Failed(format!("reddit empty body @ {url}")));
    }
    Ok(body)
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

#[derive(Debug, Deserialize)]
struct Listing<T> {
    data: ListingData<T>,
}

#[derive(Debug, Deserialize)]
struct ListingData<T> {
    children: Vec<Child<T>>,
}

#[derive(Debug, Deserialize)]
struct Child<T> {
    data: T,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_unwraps_children_data() {
        let raw = r#"{
          "data": {
            "children": [
              { "data": { "title": "first" } },
              { "data": { "title": "second" } }
            ]
          }
        }"#;
        let parsed: Listing<TestChild> = serde_json::from_str(raw).unwrap();
        let titles: Vec<_> = parsed
            .data
            .children
            .into_iter()
            .map(|c| c.data.title)
            .collect();
        assert_eq!(titles, vec!["first", "second"]);
    }

    #[derive(Debug, Deserialize)]
    struct TestChild {
        title: String,
    }
}
