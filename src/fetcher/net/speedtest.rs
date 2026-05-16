//! `net_speedtest` — actual measured internet bandwidth (download / upload / latency).
//!
//! Unlike a throughput monitor (which only shows whatever happens to be flowing right now), this
//! runs a real transfer test and reports the connection's *capacity* — the number people mean by
//! "internet speed". The endpoint is the hardcoded `speed.cloudflare.com` (`__down` / `__up`,
//! purpose-built, no API key, globally CDN'd); config can't redirect it, so the fetcher is
//! `Safety::Safe` under the host-fixed rule.
//!
//! The download is *time-boxed*: it pulls fixed-size chunks back-to-back and stops once
//! [`DOWNLOAD_BUDGET`] has elapsed, so a fast link gets an accurate sample and a slow one stays
//! bounded. Upload posts a single modest fixed payload. The whole thing takes a few seconds —
//! the background fetch daemon (30s budget) absorbs that; a `--wait` invocation on a slow link
//! may fall back to the cached value.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::Client;

use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, MarkdownTextBlockData, Payload, Status,
    TextBlockData, TextData,
};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{entry, payload};

const DOWN_URL: &str = "https://speed.cloudflare.com/__down";
const UP_URL: &str = "https://speed.cloudflare.com/__up";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
/// Per-request ceiling so one slow chunk can't hang the whole test.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Wall-clock cap on the download phase — chunks are pulled until this elapses.
const DOWNLOAD_BUDGET: Duration = Duration::from_secs(3);
/// Bytes per download request. Small enough that a single slow chunk overruns the budget only
/// marginally, large enough to measure a fast link without HTTP overhead dominating.
const CHUNK_BYTES: u64 = 2_000_000;
/// Single upload payload size. Modest so a typical link finishes well inside the daemon budget.
const UPLOAD_BYTES: usize = 2_000_000;

/// Download Mbps below this flips the `Badge` to `Warn`; below [`SLOW_MBPS`] to `Error`. Rough
/// "usable broadband" / "barely working" lines — not lab thresholds, just a glance-level tier.
const OK_MBPS: f64 = 25.0;
const SLOW_MBPS: f64 = 5.0;

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::Badge,
];

pub struct NetSpeedtest;

/// One speed-test result. `latency_ms` is optional because a failed latency probe shouldn't sink
/// an otherwise-good download / upload measurement.
#[derive(Debug, Clone, Copy)]
struct Speedtest {
    download_mbps: f64,
    upload_mbps: f64,
    latency_ms: Option<u64>,
}

#[async_trait]
impl Fetcher for NetSpeedtest {
    fn name(&self) -> &str {
        "net_speedtest"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Measured internet bandwidth — runs a real download / upload test against the fixed `speed.cloudflare.com` endpoint and reports connection capacity (not whatever happens to be flowing now). `Text` (default) headlines down + up; `TextBlock` / `MarkdownTextBlock` / `Entries` roll up download / upload / latency; `Bars` is download vs upload; `Badge` tiers the connection by download speed. No API key required."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        body_for_shape(&sample_speedtest(), shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let client = http();
        let latency_ms = measure_latency(client).await;
        let download_mbps = measure_download(client).await?;
        // A blocked POST shouldn't sink the whole reading — report download with upload as 0.
        let upload_mbps = measure_upload(client).await.unwrap_or(0.0);
        let result = Speedtest {
            download_mbps,
            upload_mbps,
            latency_ms,
        };
        let shape = ctx.shape.unwrap_or(Shape::Text);
        Ok(payload(body_for_shape(&result, shape).unwrap_or_else(
            || {
                Body::Text(TextData {
                    value: text_value(&result),
                })
            },
        )))
    }
}

fn http() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            // No transparent decompression — a throughput test must measure wire bytes, not the
            // inflated size of a decompressed body.
            .gzip(false)
            .build()
            .expect("reqwest client should build with default config")
    })
}

/// Pulls `CHUNK_BYTES` chunks back-to-back until [`DOWNLOAD_BUDGET`] elapses, then divides total
/// bytes by actual elapsed time. A chunk error ends the loop early; a run that moved no bytes at
/// all is a hard error (nothing to report).
async fn measure_download(client: &Client) -> Result<f64, FetchError> {
    let start = Instant::now();
    let mut total: u64 = 0;
    while start.elapsed() < DOWNLOAD_BUDGET {
        match download_chunk(client).await {
            Ok(n) => total += n,
            Err(_) => break,
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    if total == 0 {
        return Err(FetchError::Failed(
            "net_speedtest: download produced no data".into(),
        ));
    }
    Ok(bytes_to_mbps(total, elapsed))
}

async fn download_chunk(client: &Client) -> Result<u64, FetchError> {
    let url = format!("{DOWN_URL}?bytes={CHUNK_BYTES}");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("net_speedtest download: {e}")))?;
    if !resp.status().is_success() {
        return Err(FetchError::Failed(format!(
            "net_speedtest download: {}",
            resp.status()
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("net_speedtest download body: {e}")))?;
    Ok(bytes.len() as u64)
}

async fn measure_upload(client: &Client) -> Result<f64, FetchError> {
    let body = vec![0u8; UPLOAD_BYTES];
    let start = Instant::now();
    let resp = client
        .post(UP_URL)
        .body(body)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("net_speedtest upload: {e}")))?;
    if !resp.status().is_success() {
        return Err(FetchError::Failed(format!(
            "net_speedtest upload: {}",
            resp.status()
        )));
    }
    let _ = resp.bytes().await;
    Ok(bytes_to_mbps(
        UPLOAD_BYTES as u64,
        start.elapsed().as_secs_f64(),
    ))
}

/// Round-trip time of a near-empty request — close enough to a latency figure for a glance.
async fn measure_latency(client: &Client) -> Option<u64> {
    let url = format!("{DOWN_URL}?bytes=0");
    let start = Instant::now();
    let resp = client.get(&url).send().await.ok()?;
    let _ = resp.bytes().await.ok()?;
    Some(start.elapsed().as_millis() as u64)
}

fn bytes_to_mbps(bytes: u64, secs: f64) -> f64 {
    if secs <= 0.0 {
        return 0.0;
    }
    (bytes as f64 * 8.0) / secs / 1_000_000.0
}

fn body_for_shape(s: &Speedtest, shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::Text => Body::Text(TextData {
            value: text_value(s),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: vec![
                format!("download  {}", format_mbps(s.download_mbps)),
                format!("upload  {}", format_mbps(s.upload_mbps)),
                format!("latency  {}", format_latency(s.latency_ms)),
            ],
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: format!(
                "- **download** {}\n- **upload** {}\n- **latency** {}",
                format_mbps(s.download_mbps),
                format_mbps(s.upload_mbps),
                format_latency(s.latency_ms),
            ),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: vec![
                entry("download", &format_mbps(s.download_mbps)),
                entry("upload", &format_mbps(s.upload_mbps)),
                entry("latency", &format_latency(s.latency_ms)),
            ],
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: vec![
                Bar {
                    label: "↓ download".into(),
                    value: s.download_mbps.round() as u64,
                },
                Bar {
                    label: "↑ upload".into(),
                    value: s.upload_mbps.round() as u64,
                },
            ],
        }),
        Shape::Badge => Body::Badge(speedtest_badge(s)),
        _ => return None,
    })
}

fn text_value(s: &Speedtest) -> String {
    format!(
        "↓ {}  ↑ {}",
        format_mbps(s.download_mbps),
        format_mbps(s.upload_mbps)
    )
}

/// Tiers the connection by *download* speed — the figure that gates everyday use. Direction is
/// one number here (it's a capacity reading, not long/short sentiment), so the colour can carry
/// quality honestly.
fn speedtest_badge(s: &Speedtest) -> BadgeData {
    let status = if s.download_mbps >= OK_MBPS {
        Status::Ok
    } else if s.download_mbps >= SLOW_MBPS {
        Status::Warn
    } else {
        Status::Error
    };
    BadgeData {
        status,
        label: format!("↓ {}", format_mbps(s.download_mbps)),
    }
}

fn format_mbps(mbps: f64) -> String {
    if mbps >= 10.0 {
        format!("{mbps:.0} Mbps")
    } else {
        format!("{mbps:.1} Mbps")
    }
}

fn format_latency(ms: Option<u64>) -> String {
    ms.map(|m| format!("{m} ms")).unwrap_or_else(|| "—".into())
}

fn sample_speedtest() -> Speedtest {
    Speedtest {
        download_mbps: 487.0,
        upload_mbps: 42.0,
        latency_ms: Some(12),
    }
}

#[cfg(test)]
mod tests {
    use std::net::{SocketAddr, TcpListener};

    use super::*;

    /// Borrow a free local port, drop the listener so the port is unbound, then point the
    /// cloudflare host at it via reqwest's resolver override. Every chunk request now fails
    /// fast with connection refused (or the TLS handshake fails if something rebinds it) —
    /// either way we exercise the error path without hitting the real network.
    fn unreachable_client() -> Client {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let dead: SocketAddr = listener.local_addr().unwrap();
        drop(listener);
        Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_millis(500))
            .gzip(false)
            .resolve("speed.cloudflare.com", dead)
            .build()
            .unwrap()
    }

    #[test]
    fn body_for_shape_covers_every_supported_shape() {
        let s = sample_speedtest();
        for &shape in SHAPES {
            let body = body_for_shape(&s, shape).unwrap();
            assert_eq!(crate::render::shape_of(&body), shape);
        }
        assert!(body_for_shape(&s, Shape::Ratio).is_none());
        assert!(body_for_shape(&s, Shape::Timeline).is_none());
        assert!(body_for_shape(&s, Shape::NumberSeries).is_none());
    }

    #[test]
    fn bytes_to_mbps_converts_bytes_and_seconds_to_megabits() {
        // 1_000_000 bytes in 1s = 8 Mbps.
        assert_eq!(bytes_to_mbps(1_000_000, 1.0), 8.0);
        // 12_500_000 bytes in 2s = 50 Mbps.
        assert_eq!(bytes_to_mbps(12_500_000, 2.0), 50.0);
        assert_eq!(bytes_to_mbps(1_000_000, 0.0), 0.0);
    }

    #[test]
    fn format_mbps_drops_decimals_above_ten() {
        assert_eq!(format_mbps(487.3), "487 Mbps");
        assert_eq!(format_mbps(42.0), "42 Mbps");
        assert_eq!(format_mbps(4.27), "4.3 Mbps");
        assert_eq!(format_mbps(0.0), "0.0 Mbps");
    }

    #[test]
    fn format_latency_falls_back_to_dash() {
        assert_eq!(format_latency(Some(12)), "12 ms");
        assert_eq!(format_latency(None), "—");
    }

    #[test]
    fn badge_tiers_by_download_speed() {
        let tier = |down: f64| {
            speedtest_badge(&Speedtest {
                download_mbps: down,
                upload_mbps: 10.0,
                latency_ms: None,
            })
            .status
        };
        assert_eq!(tier(500.0), Status::Ok);
        assert_eq!(tier(OK_MBPS), Status::Ok);
        assert_eq!(tier(12.0), Status::Warn);
        assert_eq!(tier(SLOW_MBPS), Status::Warn);
        assert_eq!(tier(1.0), Status::Error);
    }

    #[test]
    fn text_and_entries_carry_both_directions() {
        let s = sample_speedtest();
        assert_eq!(text_value(&s), "↓ 487 Mbps  ↑ 42 Mbps");
        assert!(matches!(
            body_for_shape(&s, Shape::Entries),
            Some(Body::Entries(d))
                if d.items.len() == 3
                    && d.items[2].key == "latency"
                    && d.items[2].value.as_deref() == Some("12 ms"),
        ));
    }

    #[test]
    fn fetcher_contract_metadata() {
        assert_eq!(NetSpeedtest.name(), "net_speedtest");
        assert_eq!(NetSpeedtest.safety(), Safety::Safe);
        assert_eq!(NetSpeedtest.default_shape(), Shape::Text);
        assert_eq!(NetSpeedtest.shapes(), SHAPES);
        assert_eq!(NetSpeedtest.refresh_interval(), 60 * 60);
        assert!(NetSpeedtest.option_schemas().is_empty());
        let description = NetSpeedtest.description();
        assert!(description.contains("Measured internet bandwidth"));
        assert!(description.contains("speed.cloudflare.com"));
        for &shape in SHAPES {
            assert!(NetSpeedtest.sample_body(shape).is_some());
        }
        assert!(NetSpeedtest.sample_body(Shape::Ratio).is_none());
    }

    #[test]
    fn http_reuses_the_same_client() {
        assert!(std::ptr::eq(http(), http()));
    }

    #[tokio::test]
    async fn measure_download_errors_when_no_chunk_returns_bytes() {
        let client = unreachable_client();
        let err = measure_download(&client).await.unwrap_err();
        assert!(matches!(err, FetchError::Failed(msg) if msg.contains("no data")));
    }

    #[tokio::test]
    async fn measure_latency_returns_none_when_request_fails() {
        let client = unreachable_client();
        assert!(measure_latency(&client).await.is_none());
    }

    #[tokio::test]
    async fn measure_upload_surfaces_request_failure() {
        let client = unreachable_client();
        let err = measure_upload(&client).await.unwrap_err();
        assert!(matches!(err, FetchError::Failed(msg) if msg.contains("upload")));
    }

    #[tokio::test]
    async fn download_chunk_surfaces_request_failure() {
        let client = unreachable_client();
        let err = download_chunk(&client).await.unwrap_err();
        assert!(matches!(err, FetchError::Failed(msg) if msg.contains("download")));
    }

    /// Live smoke test — hits Cloudflare. `#[ignore]` keeps CI offline-safe; run with
    /// `cargo test -- --ignored fetcher::net::speedtest::tests::live` to verify the real path.
    #[tokio::test]
    #[ignore]
    async fn live_speedtest_returns_a_reading() {
        let ctx = FetchContext {
            shape: Some(Shape::Text),
            ..Default::default()
        };
        let p = NetSpeedtest.fetch(&ctx).await.unwrap();
        let Body::Text(t) = p.body else {
            panic!("expected text");
        };
        eprintln!("net_speedtest → {}", t.value);
        assert!(t.value.contains("Mbps"));
    }
}
