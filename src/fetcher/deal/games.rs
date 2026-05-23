//! `deal_games` — CheapShark cross-store gaming deals via the public JSON API.
//!
//! Safety::Safe — host hardcoded at `www.cheapshark.com`. Config picks the discount floor /
//! row cap / optional store IDs; the URL stays on-host regardless. No API key needed.

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use super::common::{self, DealRow, MAX_ROWS};
use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;
use crate::samples;

const API_BASE: &str = "https://www.cheapshark.com/api/1.0/deals";
const REDIRECT_BASE: &str = "https://www.cheapshark.com/redirect";
const NAME: &str = "deal_games";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BYTES: usize = 5 * 1024 * 1024;

const DEFAULT_MIN_DISCOUNT: u32 = 50;
const DEFAULT_LIMIT: u32 = 10;
const MIN_LIMIT: u32 = 1;

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Text,
    Shape::Entries,
    Shape::Bars,
    Shape::ImageLinkedList,
    Shape::Badge,
    Shape::Timeline,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "min_discount",
        type_hint: "integer (0..=100)",
        required: false,
        default: Some("50"),
        description: "Discount-percent floor. Deals below this percent off are filtered out.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=20)",
        required: false,
        default: Some("10"),
        description: "Maximum number of deals to display.",
    },
    OptionSchema {
        name: "stores",
        type_hint: "list of integers (CheapShark store IDs)",
        required: false,
        default: None,
        description: "Restrict to specific stores (CheapShark IDs: 1=Steam, 7=GOG, 11=Humble, 25=Epic, …). Omit for every store.",
    },
];

pub struct GamesFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    min_discount: Option<u32>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    stores: Option<Vec<u32>>,
}

#[async_trait]
impl Fetcher for GamesFetcher {
    fn name(&self) -> &str {
        NAME
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Cross-store gaming deals via the public CheapShark API. Spans Steam, GOG, Epic, Humble, Fanatical, GreenManGaming, and more (filter via `stores`). Sorted by savings; the `min_discount` floor keeps the list focused on actually-meaningful discounts. No API key required."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let extra = ctx
            .options
            .as_ref()
            .and_then(|v| toml::to_string(v).ok())
            .unwrap_or_default();
        cache_key(NAME, ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        sample_body_for(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let min_discount = opts.min_discount.unwrap_or(DEFAULT_MIN_DISCOUNT).min(100);
        let limit = opts
            .limit
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(MIN_LIMIT, MAX_ROWS as u32);
        let url = build_url(min_discount, limit, opts.stores.as_deref());
        let raw = fetch_deals(&url).await?;
        let rows: Vec<DealRow> = raw.into_iter().map(api_to_row).collect();
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let body = match shape {
            Shape::ImageLinkedList => common::image_linked_body(&rows).await,
            other => common::body_for_shape(&rows, other)
                .unwrap_or_else(|| common::linked_text_block_body(&rows)),
        };
        Ok(payload(body))
    }
}

fn build_url(min_discount: u32, limit: u32, stores: Option<&[u32]>) -> String {
    let mut url =
        format!("{API_BASE}?sortBy=Savings&desc=1&onSale=1&pageSize={limit}&lowerPrice=0");
    if min_discount > 0 {
        url.push_str(&format!("&minDiscount={min_discount}"));
    }
    if let Some(ids) = stores.filter(|s| !s.is_empty()) {
        let csv = ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        url.push_str(&format!("&storeID={csv}"));
    }
    url
}

async fn fetch_deals(url: &str) -> Result<Vec<ApiDeal>, FetchError> {
    let res = http()
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("{NAME} request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("{NAME} {status}")));
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("{NAME} read body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "{NAME} response too large ({} bytes, cap {MAX_BYTES})",
            bytes.len()
        )));
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("{NAME} json parse: {e}")))
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
struct ApiDeal {
    title: String,
    #[serde(rename = "dealID")]
    deal_id: String,
    #[serde(rename = "storeID")]
    store_id: String,
    #[serde(rename = "salePrice")]
    sale_price: String,
    #[serde(rename = "normalPrice")]
    normal_price: String,
    savings: String,
    thumb: Option<String>,
}

fn api_to_row(deal: ApiDeal) -> DealRow {
    let pct = deal
        .savings
        .parse::<f64>()
        .ok()
        .map(|f| f.round() as u32)
        .map(|p| p.min(100));
    DealRow {
        title: deal.title,
        image_url: deal.thumb.filter(|s| !s.is_empty()),
        sale_price: Some(format_price(&deal.sale_price)),
        original_price: Some(format_price(&deal.normal_price)),
        discount_pct: pct,
        store: Some(store_name(&deal.store_id).to_string()),
        link: format!("{REDIRECT_BASE}?dealID={}", deal.deal_id),
        published: None,
    }
}

fn format_price(raw: &str) -> String {
    match raw.parse::<f64>() {
        Ok(0.0) => "Free".into(),
        Ok(v) => format!("${v:.2}"),
        Err(_) => raw.to_string(),
    }
}

/// CheapShark store IDs we surface a friendly name for. Anything else falls through to
/// `"Store <id>"` so a new store doesn't trigger a release; the catalog can be backfilled
/// from the docs when it next changes.
fn store_name(id: &str) -> &'static str {
    match id {
        "1" => "Steam",
        "2" => "GamersGate",
        "3" => "GreenManGaming",
        "4" => "Amazon",
        "5" => "GameStop",
        "7" => "GOG",
        "8" => "Origin",
        "11" => "Humble Store",
        "13" => "Ubisoft Store",
        "15" => "Fanatical",
        "21" => "WinGameStore",
        "23" => "GameBillet",
        "24" => "Voidu",
        "25" => "Epic Games Store",
        "27" => "Gamesplanet",
        "29" => "2Game",
        "30" => "IndieGala",
        "31" => "Blizzard Shop",
        "33" => "DLGamer",
        _ => "Other store",
    }
}

fn sample_body_for(shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::LinkedTextBlock => samples::linked_text_block(&[
            (
                "[Steam] Disco Elysium  $9.99 (75% off from $39.99)",
                Some("https://www.cheapshark.com/redirect?dealID=sample-1"),
            ),
            (
                "[Epic Games Store] Cyberpunk 2077  $29.99 (50% off from $59.99)",
                Some("https://www.cheapshark.com/redirect?dealID=sample-2"),
            ),
            (
                "[GOG] The Witcher 3  $4.99 (75% off from $19.99)",
                Some("https://www.cheapshark.com/redirect?dealID=sample-3"),
            ),
        ]),
        Shape::TextBlock => samples::text_block(&[
            "[Steam] Disco Elysium  $9.99 (75% off from $39.99)",
            "[Epic Games Store] Cyberpunk 2077  $29.99 (50% off from $59.99)",
            "[GOG] The Witcher 3  $4.99 (75% off from $19.99)",
        ]),
        Shape::MarkdownTextBlock => samples::markdown(
            "- [[Steam] Disco Elysium  $9.99 (75% off)](https://www.cheapshark.com/redirect?dealID=sample-1)\n- [[GOG] The Witcher 3  $4.99 (75% off)](https://www.cheapshark.com/redirect?dealID=sample-3)",
        ),
        Shape::Text => samples::text("[Steam] Disco Elysium  $9.99 (75% off from $39.99)"),
        Shape::Entries => samples::entries(&[
            ("Disco Elysium", "$9.99 (75% off)"),
            ("Cyberpunk 2077", "$29.99 (50% off)"),
            ("The Witcher 3", "$4.99 (75% off)"),
        ]),
        Shape::Bars => samples::bars(&[
            ("Disco Elysium", 75),
            ("The Witcher 3", 75),
            ("Cyberpunk 2077", 50),
        ]),
        Shape::Badge => samples::badge(crate::payload::Status::Ok, "75% off"),
        Shape::Timeline => samples::timeline(&[
            (
                1_745_625_600,
                "Disco Elysium",
                Some("Steam · $9.99 · 75% off"),
            ),
            (
                1_745_539_200,
                "Cyberpunk 2077",
                Some("Epic Games Store · $29.99 · 50% off"),
            ),
        ]),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    #[test]
    fn build_url_emits_default_params() {
        let url = build_url(50, 10, None);
        assert!(url.contains("sortBy=Savings"));
        assert!(url.contains("onSale=1"));
        assert!(url.contains("pageSize=10"));
        assert!(url.contains("minDiscount=50"));
        assert!(!url.contains("storeID="));
    }

    #[test]
    fn build_url_appends_store_filter_csv() {
        let url = build_url(40, 5, Some(&[1, 7, 25]));
        assert!(url.contains("storeID=1,7,25"));
    }

    #[test]
    fn build_url_drops_min_discount_when_zero() {
        let url = build_url(0, 5, None);
        assert!(!url.contains("minDiscount"));
    }

    #[test]
    fn format_price_handles_zero_as_free() {
        assert_eq!(format_price("0.00"), "Free");
        assert_eq!(format_price("9.99"), "$9.99");
        assert_eq!(format_price("garbage"), "garbage");
    }

    #[test]
    fn store_name_falls_back_for_unknown_ids() {
        assert_eq!(store_name("1"), "Steam");
        assert_eq!(store_name("25"), "Epic Games Store");
        assert_eq!(store_name("999"), "Other store");
    }

    #[test]
    fn api_to_row_parses_realistic_payload() {
        let deal = ApiDeal {
            title: "Vaudeville".into(),
            deal_id: "abc%3D".into(),
            store_id: "25".into(),
            sale_price: "0.00".into(),
            normal_price: "19.99".into(),
            savings: "100.000000".into(),
            thumb: Some("https://example.com/thumb.jpg".into()),
        };
        let row = api_to_row(deal);
        assert_eq!(row.title, "Vaudeville");
        assert_eq!(row.store.as_deref(), Some("Epic Games Store"));
        assert_eq!(row.sale_price.as_deref(), Some("Free"));
        assert_eq!(row.original_price.as_deref(), Some("$19.99"));
        assert_eq!(row.discount_pct, Some(100));
        assert!(row.link.contains("dealID=abc%3D"));
    }

    #[test]
    fn api_to_row_clamps_implausible_savings() {
        let deal = ApiDeal {
            title: "Bug".into(),
            deal_id: "d".into(),
            store_id: "1".into(),
            sale_price: "5.00".into(),
            normal_price: "10.00".into(),
            savings: "250".into(),
            thumb: None,
        };
        let row = api_to_row(deal);
        assert_eq!(row.discount_pct, Some(100));
        assert!(row.image_url.is_none());
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("min_discount = 60\nbogus = 1").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn options_parse_stores_list() {
        let raw: toml::Value = toml::from_str("stores = [1, 7, 25]").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.stores, Some(vec![1, 7, 25]));
    }

    #[test]
    fn fetcher_exposes_safety_safe_and_default_linked_shape() {
        let f = GamesFetcher;
        assert_eq!(f.name(), NAME);
        assert_eq!(f.safety(), Safety::Safe);
        assert_eq!(f.default_shape(), Shape::LinkedTextBlock);
    }

    #[test]
    fn sample_body_covers_every_supported_shape() {
        let f = GamesFetcher;
        for shape in [
            Shape::LinkedTextBlock,
            Shape::TextBlock,
            Shape::MarkdownTextBlock,
            Shape::Text,
            Shape::Entries,
            Shape::Bars,
            Shape::Badge,
            Shape::Timeline,
        ] {
            assert!(
                f.sample_body(shape).is_some(),
                "sample missing for {shape:?}"
            );
        }
        assert!(f.sample_body(Shape::ImageLinkedList).is_none());
        assert!(f.sample_body(Shape::Heatmap).is_none());
    }

    #[test]
    fn cache_key_varies_with_min_discount() {
        let f = GamesFetcher;
        let base = FetchContext::default();
        let mut a = base.clone();
        let mut b = base.clone();
        a.options = Some(toml::from_str("min_discount = 50").unwrap());
        b.options = Some(toml::from_str("min_discount = 80").unwrap());
        assert_ne!(f.cache_key(&a), f.cache_key(&b));
    }

    #[test]
    fn store_name_maps_every_known_id() {
        let cases = [
            ("2", "GamersGate"),
            ("3", "GreenManGaming"),
            ("4", "Amazon"),
            ("5", "GameStop"),
            ("7", "GOG"),
            ("8", "Origin"),
            ("11", "Humble Store"),
            ("13", "Ubisoft Store"),
            ("15", "Fanatical"),
            ("21", "WinGameStore"),
            ("23", "GameBillet"),
            ("24", "Voidu"),
            ("27", "Gamesplanet"),
            ("29", "2Game"),
            ("30", "IndieGala"),
            ("31", "Blizzard Shop"),
            ("33", "DLGamer"),
        ];
        for (id, name) in cases {
            assert_eq!(store_name(id), name, "store id {id}");
        }
    }

    #[test]
    fn fetch_deals_parses_success_body() {
        let body = r#"[{"title":"Disco Elysium","dealID":"d1","storeID":"1","salePrice":"9.99","normalPrice":"39.99","savings":"75.0","thumb":"https://example.com/t.jpg"}]"#;
        let (url, server) = serve_once("200 OK", body);
        let deals = run_async(fetch_deals(&url)).unwrap();
        server.join().unwrap();
        assert_eq!(deals.len(), 1);
        assert_eq!(deals[0].title, "Disco Elysium");
        assert_eq!(deals[0].deal_id, "d1");
        assert_eq!(deals[0].store_id, "1");
    }

    #[test]
    fn fetch_deals_surfaces_non_success_status() {
        let (url, server) = serve_once("503 Service Unavailable", "");
        let err = run_async(fetch_deals(&url)).unwrap_err();
        server.join().unwrap();
        assert!(matches!(err, FetchError::Failed(msg) if msg.contains("deal_games 503")));
    }

    #[test]
    fn fetch_deals_surfaces_json_parse_errors() {
        let (url, server) = serve_once("200 OK", "not-json");
        let err = run_async(fetch_deals(&url)).unwrap_err();
        server.join().unwrap();
        assert!(matches!(err, FetchError::Failed(msg) if msg.contains("deal_games json parse")));
    }

    #[test]
    fn fetch_deals_surfaces_request_failures() {
        let err = run_async(fetch_deals("not-a-url")).unwrap_err();
        assert!(
            matches!(err, FetchError::Failed(msg) if msg.contains("deal_games request failed"))
        );
    }

    #[test]
    fn fetch_deals_rejects_oversized_body() {
        let body = format!("[{}", " ".repeat(MAX_BYTES + 1));
        let (url, server) = serve_once("200 OK", &body);
        let err = run_async(fetch_deals(&url)).unwrap_err();
        server.join().unwrap();
        assert!(matches!(err, FetchError::Failed(msg) if msg.contains("response too large")));
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
