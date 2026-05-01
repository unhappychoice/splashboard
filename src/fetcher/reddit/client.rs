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
    use std::future::Future;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

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

    #[test]
    fn http_reuses_the_same_client() {
        assert!(std::ptr::eq(http(), http()));
    }

    #[test]
    fn get_with_profile_returns_body_on_success() {
        let (url, server) = serve_sequence(&[("200 OK", r#"{"ok":true}"#)]);
        let body = run_async(get_with_profile(&url, HeaderProfile::Minimal)).unwrap();
        let requests = server.join().unwrap();

        assert_eq!(body, r#"{"ok":true}"#);
        assert_eq!(requests.len(), 1);
    }

    #[test]
    fn get_with_profile_rejects_non_success_status() {
        let (url, server) = serve_sequence(&[("403 Forbidden", r#"{"message":"blocked"}"#)]);
        let err = run_async(get_with_profile(&url, HeaderProfile::Minimal)).unwrap_err();
        let _ = server.join().unwrap();

        assert!(matches!(
            err,
            FetchError::Failed(message) if message == format!("reddit 403 Forbidden @ {url}")
        ));
    }

    #[test]
    fn get_with_profile_rejects_empty_body() {
        let (url, server) = serve_sequence(&[("200 OK", "")]);
        let err = run_async(get_with_profile(&url, HeaderProfile::Minimal)).unwrap_err();
        let _ = server.join().unwrap();

        assert!(matches!(
            err,
            FetchError::Failed(message) if message == format!("reddit empty body @ {url}")
        ));
    }

    #[test]
    fn get_with_retry_returns_first_success_without_retry() {
        let (url, server) = serve_sequence(&[("200 OK", r#"{"ok":true}"#)]);
        let body = run_async(get_with_retry(&url)).unwrap();
        let requests = server.join().unwrap();

        assert_eq!(body, r#"{"ok":true}"#);
        assert_eq!(requests.len(), 1);
    }

    #[test]
    fn get_with_retry_retries_with_browserish_headers_after_failure() {
        let (url, server) = serve_sequence(&[
            ("403 Forbidden", r#"{"message":"blocked"}"#),
            ("200 OK", r#"{"ok":true}"#),
        ]);
        let body = run_async(get_with_retry(&url)).unwrap();
        let requests = server.join().unwrap();

        assert_eq!(body, r#"{"ok":true}"#);
        assert_eq!(requests.len(), 2);
        assert!(!requests[0].contains("referer: https://www.reddit.com/"));
        assert!(requests[1].contains("referer: https://www.reddit.com/"));
        assert!(requests[1].contains("accept-language: en-US,en;q=0.9"));
        assert!(requests[1].contains("pragma: no-cache"));
    }

    fn serve_sequence(responses: &[(&str, &str)]) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let responses = responses
            .iter()
            .map(|(status, body)| (status.to_string(), body.to_string()))
            .collect::<Vec<_>>();
        let handle = thread::spawn(move || {
            responses
                .into_iter()
                .map(|(status, body)| {
                    let (mut stream, _) = listener.accept().unwrap();
                    let mut request = [0; 4096];
                    let read = stream.read(&mut request).unwrap();
                    let response = format!(
                        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                    stream.flush().unwrap();
                    String::from_utf8_lossy(&request[..read]).into_owned()
                })
                .collect()
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

    #[derive(Debug, Deserialize)]
    struct TestChild {
        title: String,
    }
}
