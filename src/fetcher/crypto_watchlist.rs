//! `crypto_watchlist` — CoinGecko price snapshot for a configurable set of coins.
//!
//! Safety::Safe because the host (`api.coingecko.com`) is hardcoded: the user supplies coin
//! ids and a quote currency, both of which become query parameters on that fixed host. No API
//! key is required and no token leaves the machine.

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData,
    MarkdownTextBlockData, NumberSeriesData, Payload, PointSeries, PointSeriesData, Status,
    TextBlockData, TextData,
};
use crate::render::Shape;

use super::github::common::cache_key;
use super::{FetchContext, FetchError, Fetcher, Safety};

const API_BASE: &str = "https://api.coingecko.com/api/v3/coins/markets";
const COIN_PAGE_BASE: &str = "https://www.coingecko.com/en/coins";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BYTES: usize = 5 * 1024 * 1024;

const DEFAULT_COINS: &[&str] = &["bitcoin", "ethereum"];
const DEFAULT_VS_CURRENCY: &str = "usd";
/// Cap the coin list so a misconfig can't fan out arbitrarily large requests.
const MAX_COINS: usize = 20;
/// Cap chart series so multi-coin `PointSeries` stays readable in a typical widget slot.
const MAX_SERIES_COINS: usize = 5;
/// `Bars` carry abs(% change) × 100 (basis points) since `Bar.value` is `u64`.
const PCT_TO_BP: f64 = 100.0;
/// Volatility threshold (%) for badge / entry status flip from Ok to Warn. Direction-neutral
/// because a watchlist isn't sentiment-bearing — the user might be long or short any coin.
const VOLATILITY_THRESHOLD: f64 = 5.0;

const SHAPES: &[Shape] = &[
    Shape::Entries,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::LinkedTextBlock,
    Shape::NumberSeries,
    Shape::PointSeries,
    Shape::Bars,
    Shape::Badge,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "coins",
        type_hint: "list of strings (CoinGecko coin ids)",
        required: false,
        default: Some("[\"bitcoin\", \"ethereum\"]"),
        description: "CoinGecko coin ids to include (e.g. `[\"bitcoin\", \"ethereum\", \"solana\"]`). Capped at 20 entries.",
    },
    OptionSchema {
        name: "vs_currency",
        type_hint: "string (lowercase 2-5 letter code)",
        required: false,
        default: Some("\"usd\""),
        description: "Quote currency code passed to CoinGecko (e.g. `usd`, `jpy`, `eur`, `btc`). Letters only.",
    },
];

pub struct CryptoWatchlistFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub coins: Option<Vec<String>>,
    #[serde(default)]
    pub vs_currency: Option<String>,
}

#[async_trait]
impl Fetcher for CryptoWatchlistFetcher {
    fn name(&self) -> &str {
        "crypto_watchlist"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "CoinGecko price snapshot for a configurable list of coins. `Entries` (default) / `Text` / `TextBlock` / `MarkdownTextBlock` / `LinkedTextBlock` summarise spot price + 24h change; `NumberSeries` carries the first coin's 7-day hourly price (cents) for sparklines; `PointSeries` carries one series per coin for line / scatter charts; `Bars` ranks coins by 24h |% change| (basis points); `Badge` flags the top mover. No API key required."
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
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        body_for_shape(&sample_snapshot(), shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let coins = resolve_coins(opts.coins.as_deref())?;
        let vs_currency = resolve_vs_currency(opts.vs_currency.as_deref())?;
        let snapshot = fetch_snapshot(&coins, &vs_currency).await?;
        let shape = ctx.shape.unwrap_or(Shape::Entries);
        let body = body_for_shape(&snapshot, shape).unwrap_or_else(|| entries_body(&snapshot));
        Ok(payload(body))
    }
}

fn body_for_shape(snapshot: &Snapshot, shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::Entries => entries_body(snapshot),
        Shape::Text => Body::Text(TextData {
            value: text_value(snapshot),
        }),
        Shape::TextBlock => text_block_body(snapshot),
        Shape::MarkdownTextBlock => markdown_block_body(snapshot),
        Shape::LinkedTextBlock => linked_text_block_body(snapshot),
        Shape::NumberSeries => number_series_body(snapshot),
        Shape::PointSeries => point_series_body(snapshot),
        Shape::Bars => bars_body(snapshot),
        Shape::Badge => Body::Badge(badge_for(snapshot)),
        _ => return None,
    })
}

fn resolve_coins(raw: Option<&[String]>) -> Result<Vec<String>, FetchError> {
    let supplied = match raw {
        Some(items) if !items.is_empty() => {
            items.iter().map(|s| s.to_lowercase()).collect::<Vec<_>>()
        }
        _ => DEFAULT_COINS.iter().map(|s| (*s).to_string()).collect(),
    };
    let coins: Vec<String> = supplied
        .iter()
        .filter_map(|c| sanitise_coin_id(c))
        .take(MAX_COINS)
        .collect();
    if coins.is_empty() {
        return Err(FetchError::Failed(
            "crypto_watchlist: `coins` must contain at least one CoinGecko coin id".into(),
        ));
    }
    Ok(coins)
}

fn resolve_vs_currency(raw: Option<&str>) -> Result<String, FetchError> {
    let value = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .unwrap_or_else(|| DEFAULT_VS_CURRENCY.to_string());
    let valid_len = (2..=5).contains(&value.len());
    if !valid_len || !value.chars().all(|c| c.is_ascii_lowercase()) {
        return Err(FetchError::Failed(format!(
            "crypto_watchlist: invalid vs_currency `{value}` (lowercase 2-5 letter code)"
        )));
    }
    Ok(value)
}

fn sanitise_coin_id(raw: &str) -> Option<String> {
    let id = raw.trim().to_lowercase();
    let valid_len = (1..=64).contains(&id.len());
    let valid_chars = id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    (valid_len && valid_chars).then_some(id)
}

async fn fetch_snapshot(coins: &[String], vs_currency: &str) -> Result<Snapshot, FetchError> {
    let url = build_url(coins, vs_currency);
    let res = http()
        .get(&url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("crypto_watchlist request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("crypto_watchlist {status}")));
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("crypto_watchlist read body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "crypto_watchlist response too large ({} bytes, cap {MAX_BYTES})",
            bytes.len()
        )));
    }
    let raw: Vec<ApiCoin> = serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("crypto_watchlist json parse: {e}")))?;
    Ok(Snapshot {
        vs_currency: vs_currency.to_string(),
        coins: raw.into_iter().filter_map(CoinPoint::from_api).collect(),
    })
}

fn build_url(coins: &[String], vs_currency: &str) -> String {
    let ids = coins.join(",");
    format!(
        "{API_BASE}?vs_currency={vs_currency}&ids={ids}\
         &sparkline=true&order=market_cap_desc&per_page={count}&page=1",
        count = coins.len(),
    )
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
struct ApiCoin {
    id: String,
    symbol: String,
    #[allow(dead_code)]
    name: String,
    #[serde(default)]
    current_price: Option<f64>,
    #[serde(default)]
    price_change_percentage_24h: Option<f64>,
    #[serde(default)]
    sparkline_in_7d: Option<ApiSparkline>,
}

#[derive(Debug, Deserialize)]
struct ApiSparkline {
    #[serde(default)]
    price: Vec<f64>,
}

#[derive(Debug, Clone)]
struct Snapshot {
    vs_currency: String,
    coins: Vec<CoinPoint>,
}

#[derive(Debug, Clone)]
struct CoinPoint {
    id: String,
    symbol: String,
    price: f64,
    change_24h: f64,
    sparkline: Vec<f64>,
}

impl CoinPoint {
    fn from_api(c: ApiCoin) -> Option<Self> {
        let price = c.current_price?;
        let symbol = c.symbol.to_uppercase();
        let change = c.price_change_percentage_24h.unwrap_or(0.0);
        let sparkline = c.sparkline_in_7d.map(|s| s.price).unwrap_or_default();
        Some(Self {
            id: c.id,
            symbol,
            price,
            change_24h: change,
            sparkline,
        })
    }

    fn arrow(&self) -> char {
        if self.change_24h > 0.0 {
            '▲'
        } else if self.change_24h < 0.0 {
            '▼'
        } else {
            '·'
        }
    }
}

fn entries_body(snapshot: &Snapshot) -> Body {
    Body::Entries(EntriesData {
        items: snapshot
            .coins
            .iter()
            .map(|c| Entry {
                key: c.symbol.clone(),
                value: Some(format!(
                    "{} ({})",
                    format_price(c.price, &snapshot.vs_currency),
                    format_change(c.change_24h),
                )),
                status: Some(status_for_change(c.change_24h)),
            })
            .collect(),
    })
}

fn text_value(snapshot: &Snapshot) -> String {
    match top_mover(snapshot) {
        Some(c) => format!(
            "{} {} {} ({})",
            c.arrow(),
            c.symbol,
            format_price(c.price, &snapshot.vs_currency),
            format_change(c.change_24h),
        ),
        None => "no coins".into(),
    }
}

fn text_block_body(snapshot: &Snapshot) -> Body {
    Body::TextBlock(TextBlockData {
        lines: snapshot
            .coins
            .iter()
            .map(|c| line_for(c, &snapshot.vs_currency))
            .collect(),
    })
}

fn markdown_block_body(snapshot: &Snapshot) -> Body {
    let value = snapshot
        .coins
        .iter()
        .map(|c| {
            format!(
                "- **{}** {} ({})",
                c.symbol,
                format_price(c.price, &snapshot.vs_currency),
                format_change(c.change_24h),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Body::MarkdownTextBlock(MarkdownTextBlockData { value })
}

fn linked_text_block_body(snapshot: &Snapshot) -> Body {
    Body::LinkedTextBlock(LinkedTextBlockData {
        items: snapshot
            .coins
            .iter()
            .map(|c| LinkedLine {
                text: line_for(c, &snapshot.vs_currency),
                url: Some(format!("{COIN_PAGE_BASE}/{}", c.id)),
            })
            .collect(),
    })
}

/// First coin's 7-day hourly price as cents — `NumberSeries.values: Vec<u64>` can't carry
/// fractional currency, so the implicit unit is "cents of `vs_currency`". Negative or missing
/// readings clamp to 0 rather than wrap into huge u64s.
fn number_series_body(snapshot: &Snapshot) -> Body {
    let values: Vec<u64> = snapshot
        .coins
        .first()
        .map(|c| {
            c.sparkline
                .iter()
                .map(|p| (p.max(0.0) * 100.0).round() as u64)
                .collect()
        })
        .unwrap_or_default();
    Body::NumberSeries(NumberSeriesData { values })
}

fn point_series_body(snapshot: &Snapshot) -> Body {
    Body::PointSeries(PointSeriesData {
        series: snapshot
            .coins
            .iter()
            .take(MAX_SERIES_COINS)
            .map(|c| PointSeries {
                name: c.symbol.clone(),
                points: c
                    .sparkline
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (i as f64, *p))
                    .collect(),
            })
            .collect(),
    })
}

fn bars_body(snapshot: &Snapshot) -> Body {
    Body::Bars(BarsData {
        bars: snapshot
            .coins
            .iter()
            .map(|c| Bar {
                label: format!("{} {}", c.arrow(), c.symbol),
                value: (c.change_24h.abs() * PCT_TO_BP).round() as u64,
            })
            .collect(),
    })
}

fn badge_for(snapshot: &Snapshot) -> BadgeData {
    match top_mover(snapshot) {
        Some(c) => BadgeData {
            status: status_for_change(c.change_24h),
            label: format!("{} {} {}", c.arrow(), c.symbol, format_change(c.change_24h)),
        },
        None => BadgeData {
            status: Status::Warn,
            label: "no coins".into(),
        },
    }
}

fn line_for(c: &CoinPoint, vs_currency: &str) -> String {
    format!(
        "{} {}  {}  {}",
        c.arrow(),
        c.symbol,
        format_price(c.price, vs_currency),
        format_change(c.change_24h),
    )
}

fn top_mover(snapshot: &Snapshot) -> Option<&CoinPoint> {
    snapshot.coins.iter().max_by(|a, b| {
        a.change_24h
            .abs()
            .partial_cmp(&b.change_24h.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// Volatility-coded status — large absolute moves flip Ok → Warn so the badge stands out.
/// Direction itself is encoded in the arrow / sign in the label, leaving the status agnostic
/// to whether the user is long or short.
fn status_for_change(change_pct: f64) -> Status {
    if change_pct.abs() >= VOLATILITY_THRESHOLD {
        Status::Warn
    } else {
        Status::Ok
    }
}

fn format_price(value: f64, vs_currency: &str) -> String {
    let amount = format_amount(value);
    match currency_symbol(vs_currency) {
        Some(sym) => format!("{sym}{amount}"),
        None => format!("{amount} {}", vs_currency.to_uppercase()),
    }
}

fn currency_symbol(code: &str) -> Option<&'static str> {
    match code {
        "usd" => Some("$"),
        "eur" => Some("€"),
        "jpy" => Some("¥"),
        "gbp" => Some("£"),
        "krw" => Some("₩"),
        _ => None,
    }
}

/// Locale-agnostic "1,234.56" with adaptive precision: ≥ 1 → 2 decimals, ≥ 0.01 → 4 decimals,
/// otherwise → 6 decimals. Keeps meme-coin sub-cent prices from collapsing to `0.00`.
fn format_amount(value: f64) -> String {
    let abs = value.abs();
    let formatted = if abs == 0.0 {
        "0.00".to_string()
    } else if abs < 0.01 {
        format!("{value:.6}")
    } else if abs < 1.0 {
        format!("{value:.4}")
    } else {
        format!("{value:.2}")
    };
    add_thousands_separators(&formatted)
}

fn add_thousands_separators(formatted: &str) -> String {
    let (int_part, fractional) = formatted.split_once('.').unwrap_or((formatted, ""));
    let (sign, digits) = match int_part.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", int_part),
    };
    let with_sep: String = digits
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or(""))
        .collect::<Vec<_>>()
        .join(",");
    if fractional.is_empty() {
        format!("{sign}{with_sep}")
    } else {
        format!("{sign}{with_sep}.{fractional}")
    }
}

fn format_change(pct: f64) -> String {
    let sign = if pct >= 0.0 { "+" } else { "" };
    format!("{sign}{pct:.2}%")
}

fn parse_options<T: serde::de::DeserializeOwned + Default>(
    raw: Option<&toml::Value>,
) -> Result<T, String> {
    match raw {
        None => Ok(T::default()),
        Some(value) => value
            .clone()
            .try_into::<T>()
            .map_err(|e| format!("invalid options: {e}")),
    }
}

fn payload(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}

fn sample_snapshot() -> Snapshot {
    let make_sparkline = |start: f64, drift: f64| -> Vec<f64> {
        (0..168u32)
            .map(|i| start + drift * (i as f64 / 24.0) + ((i as f64 / 8.0).sin() * start * 0.01))
            .collect()
    };
    Snapshot {
        vs_currency: "usd".into(),
        coins: vec![
            CoinPoint {
                id: "bitcoin".into(),
                symbol: "BTC".into(),
                price: 42_150.32,
                change_24h: 2.34,
                sparkline: make_sparkline(41_000.0, 8.0),
            },
            CoinPoint {
                id: "ethereum".into(),
                symbol: "ETH".into(),
                price: 2_280.18,
                change_24h: -1.21,
                sparkline: make_sparkline(2_300.0, -1.0),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_with(changes: &[(f64, f64)]) -> Snapshot {
        Snapshot {
            vs_currency: "usd".into(),
            coins: changes
                .iter()
                .enumerate()
                .map(|(i, (price, change))| CoinPoint {
                    id: format!("coin-{i}"),
                    symbol: format!("C{i}"),
                    price: *price,
                    change_24h: *change,
                    sparkline: vec![*price; 4],
                })
                .collect(),
        }
    }

    #[test]
    fn build_url_joins_ids_with_commas_and_includes_sparkline() {
        let url = build_url(&["bitcoin".into(), "ethereum".into()], "jpy");
        assert!(url.starts_with(API_BASE));
        assert!(url.contains("vs_currency=jpy"));
        assert!(url.contains("ids=bitcoin,ethereum"));
        assert!(url.contains("sparkline=true"));
        assert!(url.contains("per_page=2"));
    }

    #[test]
    fn sanitise_coin_id_accepts_lowercase_alnum_and_hyphen() {
        assert_eq!(sanitise_coin_id("bitcoin"), Some("bitcoin".into()));
        assert_eq!(sanitise_coin_id("0x0-token"), Some("0x0-token".into()));
        assert_eq!(sanitise_coin_id("  ETHEREUM  "), Some("ethereum".into()));
    }

    #[test]
    fn sanitise_coin_id_rejects_special_characters() {
        // The id is interpolated into the query string — accepting `&` or `=` would let a user
        // sneak extra params in. Lock the allowlist down to alphanumeric + hyphen.
        for bad in ["", "btc&ids=evil", "btc=evil", "btc/eth", "btc.eth", " "] {
            assert_eq!(sanitise_coin_id(bad), None, "should reject {bad:?}");
        }
    }

    #[test]
    fn resolve_coins_falls_back_to_default_when_unset_or_empty() {
        assert_eq!(resolve_coins(None).unwrap(), vec!["bitcoin", "ethereum"]);
        let empty: Vec<String> = vec![];
        assert_eq!(
            resolve_coins(Some(&empty)).unwrap(),
            vec!["bitcoin", "ethereum"]
        );
    }

    #[test]
    fn resolve_coins_drops_invalid_ids_and_caps_to_max() {
        let mut input: Vec<String> = (0..MAX_COINS + 5).map(|i| format!("valid{i}")).collect();
        input.insert(0, "bad&id".into());
        let coins = resolve_coins(Some(&input)).unwrap();
        assert_eq!(coins.len(), MAX_COINS);
        assert!(coins.iter().all(|c| !c.contains('&')));
    }

    #[test]
    fn resolve_coins_errors_when_every_id_is_invalid() {
        let bad: Vec<String> = vec!["?".into(), "/".into(), "".into()];
        assert!(resolve_coins(Some(&bad)).is_err());
    }

    #[test]
    fn resolve_vs_currency_defaults_to_usd_and_lowercases() {
        assert_eq!(resolve_vs_currency(None).unwrap(), "usd");
        assert_eq!(resolve_vs_currency(Some("  ")).unwrap(), "usd");
        assert_eq!(resolve_vs_currency(Some("JPY")).unwrap(), "jpy");
    }

    #[test]
    fn resolve_vs_currency_rejects_non_letters_and_wrong_length() {
        // Same injection concern as `coins`: vs_currency goes straight into the URL query.
        // (Surrounding whitespace is intentionally trimmed before validation, so `"us "` would
        // accept as `"us"` — that case lives in the happy-path test instead.)
        for bad in ["u", "verylong", "us1", "us&ids=evil", "u s"] {
            assert!(
                resolve_vs_currency(Some(bad)).is_err(),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn currency_symbol_known_codes_get_glyphs_others_get_iso() {
        assert_eq!(format_price(1234.5, "usd"), "$1,234.50");
        assert_eq!(format_price(1234.5, "jpy"), "¥1,234.50");
        assert_eq!(format_price(1234.5, "krw"), "₩1,234.50");
        // Unknown codes fall through to "<amount> <CODE>" so config can still pick exotic
        // currencies (e.g. CoinGecko supports `vs_currency = "btc"`).
        assert_eq!(format_price(0.001234, "btc"), "0.001234 BTC");
    }

    #[test]
    fn format_amount_picks_precision_by_magnitude() {
        assert_eq!(format_amount(0.0), "0.00");
        assert_eq!(format_amount(0.0001234), "0.000123");
        assert_eq!(format_amount(0.5432), "0.5432");
        assert_eq!(format_amount(42_150.327), "42,150.33");
        assert_eq!(format_amount(-1_500_000.5), "-1,500,000.50");
    }

    #[test]
    fn format_change_keeps_sign_and_two_decimals() {
        assert_eq!(format_change(2.345), "+2.35%");
        assert_eq!(format_change(-1.2), "-1.20%");
        assert_eq!(format_change(0.0), "+0.00%");
    }

    #[test]
    fn status_for_change_flips_above_volatility_threshold() {
        assert_eq!(status_for_change(0.0), Status::Ok);
        assert_eq!(status_for_change(4.99), Status::Ok);
        assert_eq!(status_for_change(5.0), Status::Warn);
        assert_eq!(status_for_change(-7.5), Status::Warn);
    }

    #[test]
    fn top_mover_picks_largest_absolute_change() {
        let snap = snapshot_with(&[(1.0, 1.5), (2.0, -3.2), (3.0, 0.5)]);
        let top = top_mover(&snap).unwrap();
        assert_eq!(top.symbol, "C1");
    }

    #[test]
    fn top_mover_returns_none_for_empty_snapshot() {
        let snap = Snapshot {
            vs_currency: "usd".into(),
            coins: vec![],
        };
        assert!(top_mover(&snap).is_none());
    }

    #[test]
    fn entries_body_marks_each_row_with_volatility_status() {
        let snap = snapshot_with(&[(100.0, 1.0), (200.0, 6.0)]);
        let Body::Entries(data) = entries_body(&snap) else {
            panic!("expected entries");
        };
        assert_eq!(data.items.len(), 2);
        assert_eq!(data.items[0].status, Some(Status::Ok));
        assert_eq!(data.items[1].status, Some(Status::Warn));
        assert!(data.items[0].value.as_deref().unwrap().contains("(+1.00%)"));
    }

    #[test]
    fn number_series_carries_first_coin_sparkline_in_cents() {
        let snap = Snapshot {
            vs_currency: "usd".into(),
            coins: vec![CoinPoint {
                id: "x".into(),
                symbol: "X".into(),
                price: 10.0,
                change_24h: 0.0,
                sparkline: vec![1.234, -0.5, 0.0, 12.345_678],
            }],
        };
        let Body::NumberSeries(d) = number_series_body(&snap) else {
            panic!("expected number series");
        };
        // 1.234 → 123, -0.5 clamped to 0 → 0, 0.0 → 0, 12.345_678 → 1235.
        assert_eq!(d.values, vec![123, 0, 0, 1235]);
    }

    #[test]
    fn point_series_caps_to_max_series_coins_and_indexes_x_by_hour() {
        let snap = snapshot_with(&[(1.0, 0.0); 7]);
        let Body::PointSeries(d) = point_series_body(&snap) else {
            panic!("expected point series");
        };
        assert_eq!(d.series.len(), MAX_SERIES_COINS);
        assert_eq!(d.series[0].points.first().unwrap().0, 0.0);
    }

    #[test]
    fn bars_body_encodes_basis_points_and_direction_arrow() {
        let snap = snapshot_with(&[(1.0, 2.5), (1.0, -3.0), (1.0, 0.0)]);
        let Body::Bars(d) = bars_body(&snap) else {
            panic!("expected bars");
        };
        assert_eq!(d.bars[0].value, 250);
        assert_eq!(d.bars[1].value, 300);
        assert_eq!(d.bars[2].value, 0);
        assert!(d.bars[0].label.starts_with('▲'));
        assert!(d.bars[1].label.starts_with('▼'));
        assert!(d.bars[2].label.starts_with('·'));
    }

    #[test]
    fn linked_text_block_links_each_row_to_the_coin_page() {
        let snap = snapshot_with(&[(1.0, 1.0)]);
        let Body::LinkedTextBlock(d) = linked_text_block_body(&snap) else {
            panic!("expected linked text block");
        };
        assert_eq!(
            d.items[0].url.as_deref(),
            Some("https://www.coingecko.com/en/coins/coin-0")
        );
    }

    #[test]
    fn markdown_block_lists_coins_with_bold_symbol() {
        let snap = snapshot_with(&[(1.0, 1.0)]);
        let Body::MarkdownTextBlock(d) = markdown_block_body(&snap) else {
            panic!("expected markdown text block");
        };
        assert!(d.value.starts_with("- **C0**"));
    }

    #[test]
    fn badge_carries_top_mover_label_with_volatility_status() {
        let snap = snapshot_with(&[(1.0, 1.0), (1.0, -7.5)]);
        let badge = badge_for(&snap);
        assert!(badge.label.contains("C1"));
        assert!(badge.label.contains("-7.50"));
        assert_eq!(badge.status, Status::Warn);
    }

    #[test]
    fn badge_for_empty_snapshot_falls_back_to_no_coins() {
        let snap = Snapshot {
            vs_currency: "usd".into(),
            coins: vec![],
        };
        let badge = badge_for(&snap);
        assert_eq!(badge.label, "no coins");
        assert_eq!(badge.status, Status::Warn);
    }

    #[test]
    fn parse_options_accepts_empty_input_and_rejects_unknown_keys() {
        assert!(parse_options::<Options>(None).unwrap().coins.is_none());
        let raw: toml::Value = toml::from_str("coins = [\"bitcoin\"]").unwrap();
        let parsed: Options = parse_options(Some(&raw)).unwrap();
        assert_eq!(parsed.coins.unwrap(), vec!["bitcoin"]);
        let bogus: toml::Value = toml::from_str("coins = [\"bitcoin\"]\nbogus = true").unwrap();
        assert!(parse_options::<Options>(Some(&bogus)).is_err());
    }

    #[test]
    fn api_response_deserialises_coingecko_markets_row() {
        let raw = r#"[{
            "id":"bitcoin","symbol":"btc","name":"Bitcoin",
            "current_price":42150.32,"price_change_percentage_24h":2.34,
            "sparkline_in_7d":{"price":[41000.0,41100.0]}
        }]"#;
        let parsed: Vec<ApiCoin> = serde_json::from_str(raw).unwrap();
        let coin = CoinPoint::from_api(parsed.into_iter().next().unwrap()).unwrap();
        assert_eq!(coin.symbol, "BTC");
        assert_eq!(coin.price, 42150.32);
        assert_eq!(coin.sparkline.len(), 2);
    }

    #[test]
    fn coin_point_from_api_drops_rows_without_a_price() {
        let coin = CoinPoint::from_api(ApiCoin {
            id: "x".into(),
            symbol: "x".into(),
            name: "x".into(),
            current_price: None,
            price_change_percentage_24h: None,
            sparkline_in_7d: None,
        });
        assert!(coin.is_none());
    }

    #[test]
    fn fetcher_metadata_cache_key_and_samples_cover_supported_shapes() {
        let fetcher = CryptoWatchlistFetcher;
        let ctx = FetchContext {
            widget_id: "crypto".into(),
            timeout: Duration::from_secs(1),
            shape: Some(Shape::Entries),
            ..Default::default()
        };
        let with_options = FetchContext {
            options: Some(toml::from_str("coins = [\"bitcoin\"]\nvs_currency = \"jpy\"").unwrap()),
            ..ctx.clone()
        };
        assert_eq!(fetcher.name(), "crypto_watchlist");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("CoinGecko"));
        assert_eq!(
            fetcher
                .option_schemas()
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>(),
            vec!["coins", "vs_currency"]
        );
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::Entries);
        assert_ne!(fetcher.cache_key(&ctx), fetcher.cache_key(&with_options));
        for &shape in fetcher.shapes() {
            let body = fetcher
                .sample_body(shape)
                .unwrap_or_else(|| panic!("missing sample for {shape:?}"));
            let observed = crate::render::shape_of(&body);
            assert_eq!(observed, shape, "sample shape mismatch for {shape:?}");
        }
        assert!(fetcher.sample_body(Shape::Image).is_none());
        assert!(fetcher.sample_body(Shape::Calendar).is_none());
        assert!(fetcher.sample_body(Shape::Heatmap).is_none());
        assert!(fetcher.sample_body(Shape::Timeline).is_none());
        assert!(fetcher.sample_body(Shape::Ratio).is_none());
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_vs_currency_before_network() {
        let ctx = FetchContext {
            widget_id: "crypto".into(),
            timeout: Duration::from_secs(1),
            shape: Some(Shape::Entries),
            options: Some(toml::from_str("vs_currency = \"u\"").unwrap()),
            ..Default::default()
        };
        let err = CryptoWatchlistFetcher.fetch(&ctx).await.unwrap_err();
        assert!(matches!(err, FetchError::Failed(msg) if msg.contains("vs_currency")));
    }

    /// Live smoke test — hits CoinGecko. `#[ignore]` keeps CI offline-safe; run with
    /// `cargo test -- --ignored fetcher::crypto_watchlist::tests::live` to verify the real API.
    #[tokio::test]
    #[ignore]
    async fn live_default_watchlist_returns_two_coins() {
        let snapshot = fetch_snapshot(&["bitcoin".into(), "ethereum".into()], "usd")
            .await
            .unwrap();
        assert!(!snapshot.coins.is_empty(), "expected at least one coin");
        for coin in &snapshot.coins {
            eprintln!(
                "{} {} ({})",
                coin.symbol,
                format_price(coin.price, &snapshot.vs_currency),
                format_change(coin.change_24h),
            );
        }
    }
}
