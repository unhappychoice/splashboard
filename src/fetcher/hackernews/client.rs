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
        title: String,
    }

    #[test]
    fn get_deserializes_success_body() {
        let (url, server) = serve_once("200 OK", r#"{"title":"Launch HN"}"#);
        let payload: TestPayload = run_async(get(&url)).unwrap();
        server.join().unwrap();
        assert_eq!(payload.title, "Launch HN");
    }

    #[test]
    fn get_surfaces_non_success_status() {
        let (url, server) = serve_once("503 Service Unavailable", "");
        let err = run_async(get::<TestPayload>(&url)).unwrap_err();
        server.join().unwrap();
        assert!(matches!(
            err,
            FetchError::Failed(msg) if msg == "hn 503 Service Unavailable"
        ));
    }

    #[test]
    fn get_surfaces_json_parse_errors() {
        let (url, server) = serve_once("200 OK", "not-json");
        let err = run_async(get::<TestPayload>(&url)).unwrap_err();
        server.join().unwrap();
        assert!(matches!(
            err,
            FetchError::Failed(msg) if msg.contains("hn json parse")
        ));
    }

    #[test]
    fn get_surfaces_request_failures() {
        let err = run_async(get::<TestPayload>("not-a-url")).unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(msg) if msg.contains("hn request failed")
        ));
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
