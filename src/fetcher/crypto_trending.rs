//! `crypto_trending` — top 15 trending coins on CoinGecko, by user search activity.
//!
//! Safety::Safe: both the API host (`api.coingecko.com`) and the coin-page host
//! (`www.coingecko.com`) are hardcoded; the response carries thumbnail URLs on
//! `assets.coingecko.com`, which `thumbnails::download_to_cache` accepts as-is once
//! sniffed for image magic bytes. No API key required.
//!
//! Distinct from the shipped `crypto_watchlist` (a user-curated price snapshot for chosen
//! coins): trending is the inverse — CoinGecko picks the list based on what people are
//! searching, the user supplies only `count` and an optional quote currency.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, ImageLinkedItem, ImageLinkedListData, LinkedLine,
    LinkedTextBlockData, MarkdownTextBlockData, Payload, TextBlockData, TextData,
};
use crate::render::Shape;

use super::github::common::{cache_key, parse_options, payload};
use super::thumbnails;
use super::{FetchContext, FetchError, Fetcher, Safety};

const API_URL: &str = "https://api.coingecko.com/api/v3/search/trending";
const COIN_PAGE_BASE: &str = "https://www.coingecko.com/en/coins";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BYTES: usize = 5 * 1024 * 1024;

const DEFAULT_COUNT: u32 = 10;
const MIN_COUNT: u32 = 1;
/// CoinGecko `/search/trending` returns at most 15 coins; clamping at the source.
const MAX_COUNT: u32 = 15;
const DEFAULT_VS_CURRENCY: &str = "usd";

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::ImageLinkedList,
    Shape::TextBlock,
    Shape::Text,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=15)",
        required: false,
        default: Some("10"),
        description: "Number of trending coins to display.",
    },
    OptionSchema {
        name: "vs_currency",
        type_hint: "string (lowercase 2-5 letter code)",
        required: false,
        default: Some("\"usd\""),
        description: "Quote currency for the 24h-change column (e.g. `usd`, `jpy`, `eur`, `btc`). Letters only; matches `crypto_watchlist`.",
    },
];

pub struct CryptoTrendingFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub vs_currency: Option<String>,
}

#[async_trait]
impl Fetcher for CryptoTrendingFetcher {
    fn name(&self) -> &str {
        "crypto_trending"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Top trending coins on CoinGecko (ranked by user search activity over the last 24h). No API key required. Companion to the shipped `crypto_watchlist` — same source, but CoinGecko picks the list instead of the user."
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
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        sample_trending_body(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let count = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_COUNT) as usize;
        let vs_currency = resolve_vs_currency(opts.vs_currency.as_deref())?;
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let coins = fetch_trending(&vs_currency, count).await?;
        let body = render_for_shape(&coins, shape).await;
        Ok(payload(body))
    }
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
            "crypto_trending: invalid vs_currency `{value}` (lowercase 2-5 letter code)"
        )));
    }
    Ok(value)
}

/// Trending coin as the renderer wants it. `position` is the 0-indexed slot in CoinGecko's
/// response (= trending rank). `price_change_24h` is the 24h % move in the requested quote
/// currency, falling through `None` when CoinGecko hasn't populated that currency yet for a
/// freshly-trending obscure coin.
#[derive(Debug, Clone)]
struct Coin {
    name: String,
    symbol: String,
    slug: String,
    market_cap_rank: Option<i32>,
    price_change_24h: Option<f64>,
    thumb_url: Option<String>,
    position: usize,
}

impl Coin {
    fn url(&self) -> String {
        format!("{COIN_PAGE_BASE}/{}", self.slug)
    }

    fn display_label(&self) -> String {
        format!("{}  {}", self.symbol, self.name)
    }

    /// Single-line score used by Text / LinkedTextBlock / TextBlock / Entries rows: `+5.2%`,
    /// `-2.1%`, or `· trending` when the 24h change isn't available for this quote currency.
    fn score_line(&self) -> String {
        match self.price_change_24h {
            Some(pct) => format!("{pct:+.1}% 24h"),
            None => "· trending".into(),
        }
    }
}

async fn fetch_trending(vs_currency: &str, limit: usize) -> Result<Vec<Coin>, FetchError> {
    let response = fetch_response().await?;
    Ok(response
        .coins
        .into_iter()
        .enumerate()
        .take(limit)
        .map(|(i, entry)| coin_from(i, entry.item, vs_currency))
        .collect())
}

async fn fetch_response() -> Result<TrendingResponse, FetchError> {
    let res = http()
        .get(API_URL)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("crypto_trending request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("crypto_trending {status}")));
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("crypto_trending read body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "crypto_trending response too large ({} bytes, cap {MAX_BYTES})",
            bytes.len()
        )));
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("crypto_trending json parse: {e}")))
}

fn coin_from(position: usize, item: ApiItem, vs_currency: &str) -> Coin {
    let price_change_24h = item
        .data
        .as_ref()
        .and_then(|d| d.price_change_percentage_24h.as_ref())
        .and_then(|map| map.get(vs_currency).copied());
    Coin {
        name: item.name,
        symbol: item.symbol.to_uppercase(),
        slug: item.slug,
        market_cap_rank: item.market_cap_rank,
        price_change_24h,
        thumb_url: item.thumb.filter(|u| !u.is_empty()),
        position,
    }
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

async fn render_for_shape(coins: &[Coin], shape: Shape) -> Body {
    if matches!(shape, Shape::ImageLinkedList) {
        let paths = resolve_thumbnails(coins).await;
        return image_linked_body(coins, &paths);
    }
    render_sync(coins, shape)
}

async fn resolve_thumbnails(coins: &[Coin]) -> Vec<Option<PathBuf>> {
    let urls: Vec<Option<String>> = coins.iter().map(|c| c.thumb_url.clone()).collect();
    thumbnails::download_many(&urls).await
}

fn render_sync(coins: &[Coin], shape: Shape) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: headline(coins),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: coins.iter().map(row_line).collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: markdown_body(coins),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: coins.iter().map(entry_row).collect(),
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: coins.iter().map(rank_bar).collect(),
        }),
        _ => Body::LinkedTextBlock(LinkedTextBlockData {
            items: coins
                .iter()
                .map(|c| LinkedLine {
                    text: row_line(c),
                    url: Some(c.url()),
                })
                .collect(),
        }),
    }
}

fn image_linked_body(coins: &[Coin], paths: &[Option<PathBuf>]) -> Body {
    Body::ImageLinkedList(ImageLinkedListData {
        items: coins
            .iter()
            .enumerate()
            .map(|(i, c)| ImageLinkedItem {
                title: c.display_label(),
                url: Some(c.url()),
                thumbnail_path: paths
                    .get(i)
                    .and_then(|p| p.as_ref())
                    .map(|p| p.to_string_lossy().into_owned()),
                subtitle: Some(subtitle_line(c)),
            })
            .collect(),
    })
}

fn headline(coins: &[Coin]) -> String {
    coins
        .first()
        .map(|c| format!("#1 {}  {}", c.display_label(), c.score_line()))
        .unwrap_or_else(|| "(no trending coins)".into())
}

fn row_line(coin: &Coin) -> String {
    format!(
        "#{} {}  {}",
        coin.position + 1,
        coin.display_label(),
        coin.score_line()
    )
}

fn subtitle_line(coin: &Coin) -> String {
    let mut s = format!("#{} trending · {}", coin.position + 1, coin.score_line());
    if let Some(rank) = coin.market_cap_rank {
        s.push_str(&format!("  · mcap #{rank}"));
    }
    s
}

fn markdown_body(coins: &[Coin]) -> String {
    coins
        .iter()
        .map(|c| {
            format!(
                "- **#{} {}** — {}  ([page]({}))",
                c.position + 1,
                c.display_label(),
                c.score_line(),
                c.url()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn entry_row(coin: &Coin) -> Entry {
    Entry {
        key: format!("#{} {}", coin.position + 1, coin.display_label()),
        value: Some(coin.score_line()),
        status: None,
    }
}

/// Trending rank as `(count - position)` so the top coin gets the tallest bar. Coingecko's
/// own `score` field is just `0..N` ordering and decays to zero for the lower half — using
/// `count - position` keeps the chart legible regardless of slot width.
fn rank_bar(coin: &Coin) -> Bar {
    Bar {
        label: coin.display_label(),
        value: (MAX_COUNT as usize - coin.position).max(1) as u64,
    }
}

fn sample_trending_body(shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::LinkedTextBlock => crate::samples::linked_text_block(&[
            (
                "#1 BTC  Bitcoin  +5.2% 24h",
                Some("https://www.coingecko.com/en/coins/bitcoin"),
            ),
            (
                "#2 ETH  Ethereum  +3.1% 24h",
                Some("https://www.coingecko.com/en/coins/ethereum"),
            ),
        ]),
        Shape::ImageLinkedList => crate::samples::image_linked_list(&[
            (
                "BTC  Bitcoin",
                Some("https://www.coingecko.com/en/coins/bitcoin"),
                None,
                Some("#1 trending · +5.2% 24h  · mcap #1"),
            ),
            (
                "ETH  Ethereum",
                Some("https://www.coingecko.com/en/coins/ethereum"),
                None,
                Some("#2 trending · +3.1% 24h  · mcap #2"),
            ),
        ]),
        Shape::TextBlock => crate::samples::text_block(&[
            "#1 BTC  Bitcoin  +5.2% 24h",
            "#2 ETH  Ethereum  +3.1% 24h",
        ]),
        Shape::Text => crate::samples::text("#1 BTC  Bitcoin  +5.2% 24h"),
        Shape::MarkdownTextBlock => crate::samples::markdown(
            "- **#1 BTC  Bitcoin** — +5.2% 24h  ([page](https://www.coingecko.com/en/coins/bitcoin))\n- **#2 ETH  Ethereum** — +3.1% 24h  ([page](https://www.coingecko.com/en/coins/ethereum))",
        ),
        Shape::Entries => crate::samples::entries(&[
            ("#1 BTC  Bitcoin", "+5.2% 24h"),
            ("#2 ETH  Ethereum", "+3.1% 24h"),
        ]),
        Shape::Bars => crate::samples::bars(&[("BTC  Bitcoin", 15), ("ETH  Ethereum", 14)]),
        _ => return None,
    })
}

#[derive(Debug, Deserialize)]
struct TrendingResponse {
    coins: Vec<TrendingEntry>,
}

#[derive(Debug, Deserialize)]
struct TrendingEntry {
    item: ApiItem,
}

#[derive(Debug, Deserialize)]
struct ApiItem {
    name: String,
    symbol: String,
    slug: String,
    #[serde(default)]
    market_cap_rank: Option<i32>,
    #[serde(default)]
    thumb: Option<String>,
    #[serde(default)]
    data: Option<ApiData>,
}

#[derive(Debug, Deserialize)]
struct ApiData {
    #[serde(default)]
    price_change_percentage_24h: Option<std::collections::HashMap<String, f64>>,
}

#[cfg(test)]
mod tests {
    use std::time::Duration as StdDuration;

    use super::*;

    fn ctx(options: Option<&str>, shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "crypto-trending".into(),
            timeout: StdDuration::from_secs(1),
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            ..Default::default()
        }
    }

    fn coin(name: &str, symbol: &str, slug: &str, position: usize, change: Option<f64>) -> Coin {
        Coin {
            name: name.into(),
            symbol: symbol.into(),
            slug: slug.into(),
            market_cap_rank: Some(position as i32 + 1),
            price_change_24h: change,
            thumb_url: Some(format!("https://example.com/{slug}.png")),
            position,
        }
    }

    #[test]
    fn options_default_to_none() {
        let opts = Options::default();
        assert!(opts.count.is_none());
        assert!(opts.vs_currency.is_none());
    }

    #[test]
    fn options_deserialize_count_and_currency() {
        let raw: toml::Value = toml::from_str("count = 5\nvs_currency = \"jpy\"").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.count, Some(5));
        assert_eq!(opts.vs_currency.as_deref(), Some("jpy"));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("bogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn resolve_vs_currency_falls_back_to_default() {
        assert_eq!(resolve_vs_currency(None).unwrap(), "usd");
        assert_eq!(resolve_vs_currency(Some("")).unwrap(), "usd");
    }

    #[test]
    fn resolve_vs_currency_lowercases_and_rejects_invalid() {
        assert_eq!(resolve_vs_currency(Some("JPY")).unwrap(), "jpy");
        assert!(resolve_vs_currency(Some("us")).is_ok());
        assert!(resolve_vs_currency(Some("a")).is_err());
        assert!(resolve_vs_currency(Some("toolong")).is_err());
        assert!(resolve_vs_currency(Some("us1")).is_err());
    }

    #[test]
    fn coin_url_uses_slug_off_coingecko_page_base() {
        let c = coin("Bitcoin", "BTC", "bitcoin", 0, Some(5.2));
        assert_eq!(c.url(), "https://www.coingecko.com/en/coins/bitcoin");
    }

    #[test]
    fn coin_score_line_renders_signed_percent_or_trending_fallback() {
        assert_eq!(coin("a", "A", "a", 0, Some(5.2)).score_line(), "+5.2% 24h");
        assert_eq!(coin("a", "A", "a", 0, Some(-2.7)).score_line(), "-2.7% 24h");
        assert_eq!(coin("a", "A", "a", 0, None).score_line(), "· trending");
    }

    #[test]
    fn coin_from_extracts_vs_currency_change_and_uppercases_symbol() {
        let mut prices = std::collections::HashMap::new();
        prices.insert("usd".to_string(), 5.2);
        let item = ApiItem {
            name: "Bitcoin".into(),
            symbol: "btc".into(),
            slug: "bitcoin".into(),
            market_cap_rank: Some(1),
            thumb: Some("https://assets.coingecko.com/coins/images/1/thumb.png".into()),
            data: Some(ApiData {
                price_change_percentage_24h: Some(prices),
            }),
        };
        let c = coin_from(0, item, "usd");
        assert_eq!(c.symbol, "BTC");
        assert_eq!(c.price_change_24h, Some(5.2));
        assert_eq!(c.position, 0);
        assert_eq!(c.market_cap_rank, Some(1));
        assert!(c.thumb_url.is_some());
    }

    #[test]
    fn coin_from_drops_empty_thumb_url() {
        let item = ApiItem {
            name: "x".into(),
            symbol: "x".into(),
            slug: "x".into(),
            market_cap_rank: None,
            thumb: Some(String::new()),
            data: None,
        };
        let c = coin_from(0, item, "usd");
        assert!(c.thumb_url.is_none());
        assert!(c.price_change_24h.is_none());
    }

    #[test]
    fn coin_from_returns_none_change_when_vs_currency_absent_from_response() {
        let mut prices = std::collections::HashMap::new();
        prices.insert("eur".to_string(), 5.2);
        let item = ApiItem {
            name: "x".into(),
            symbol: "x".into(),
            slug: "x".into(),
            market_cap_rank: None,
            thumb: None,
            data: Some(ApiData {
                price_change_percentage_24h: Some(prices),
            }),
        };
        let c = coin_from(0, item, "usd");
        assert!(c.price_change_24h.is_none());
    }

    #[test]
    fn render_sync_text_uses_first_coin_with_rank_prefix() {
        let body = render_sync(
            &[coin("Bitcoin", "BTC", "bitcoin", 0, Some(5.2))],
            Shape::Text,
        );
        let Body::Text(t) = body else {
            panic!("expected text");
        };
        assert!(t.value.contains("#1"));
        assert!(t.value.contains("BTC"));
        assert!(t.value.contains("Bitcoin"));
        assert!(t.value.contains("+5.2%"));
    }

    #[test]
    fn render_sync_text_handles_empty_coins() {
        let body = render_sync(&[], Shape::Text);
        let Body::Text(t) = body else {
            panic!("expected text");
        };
        assert_eq!(t.value, "(no trending coins)");
    }

    #[test]
    fn render_sync_linked_text_block_carries_coingecko_url() {
        let body = render_sync(
            &[coin("Bitcoin", "BTC", "bitcoin", 0, Some(5.2))],
            Shape::LinkedTextBlock,
        );
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked_text_block");
        };
        assert_eq!(
            b.items[0].url.as_deref(),
            Some("https://www.coingecko.com/en/coins/bitcoin")
        );
    }

    #[test]
    fn render_sync_bars_use_decreasing_rank_score() {
        let coins = vec![
            coin("a", "A", "a", 0, None),
            coin("b", "B", "b", 1, None),
            coin("c", "C", "c", 14, None),
        ];
        let body = render_sync(&coins, Shape::Bars);
        let Body::Bars(b) = body else {
            panic!("expected bars");
        };
        assert_eq!(b.bars[0].value, 15);
        assert_eq!(b.bars[1].value, 14);
        // position 14 (the last slot) should still be > 0
        assert_eq!(b.bars[2].value, 1);
    }

    #[test]
    fn render_sync_markdown_includes_page_link() {
        let body = render_sync(
            &[coin("Bitcoin", "BTC", "bitcoin", 0, Some(5.2))],
            Shape::MarkdownTextBlock,
        );
        let Body::MarkdownTextBlock(m) = body else {
            panic!("expected markdown");
        };
        assert!(m.value.contains("[page]("));
        assert!(m.value.contains("bitcoin"));
    }

    #[test]
    fn render_sync_entries_use_rank_prefixed_key_and_score_value() {
        let body = render_sync(
            &[coin("Bitcoin", "BTC", "bitcoin", 0, Some(5.2))],
            Shape::Entries,
        );
        let Body::Entries(e) = body else {
            panic!("expected entries");
        };
        assert_eq!(e.items[0].key, "#1 BTC  Bitcoin");
        assert_eq!(e.items[0].value.as_deref(), Some("+5.2% 24h"));
    }

    #[test]
    fn image_linked_body_pins_paths_and_subtitle_with_mcap_rank() {
        let coins = vec![coin("Bitcoin", "BTC", "bitcoin", 0, Some(5.2))];
        let paths = vec![Some(PathBuf::from("/tmp/btc.png"))];
        let body = image_linked_body(&coins, &paths);
        let Body::ImageLinkedList(d) = body else {
            panic!("expected image_linked_list");
        };
        assert_eq!(d.items[0].title, "BTC  Bitcoin");
        assert_eq!(d.items[0].thumbnail_path.as_deref(), Some("/tmp/btc.png"));
        assert!(
            d.items[0]
                .subtitle
                .as_deref()
                .unwrap()
                .contains("#1 trending")
        );
        assert!(d.items[0].subtitle.as_deref().unwrap().contains("mcap #1"));
    }

    #[test]
    fn fetcher_exposes_catalog_metadata_and_samples() {
        let fetcher = CryptoTrendingFetcher;
        assert_eq!(fetcher.name(), "crypto_trending");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(
            fetcher
                .option_schemas()
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>(),
            vec!["count", "vs_currency"]
        );
        for shape in SHAPES {
            assert!(
                fetcher.sample_body(*shape).is_some(),
                "missing sample for {shape:?}"
            );
        }
        assert!(fetcher.sample_body(Shape::Ratio).is_none());
    }

    #[test]
    fn cache_key_partitions_by_shape_and_options() {
        let fetcher = CryptoTrendingFetcher;
        let linked = fetcher.cache_key(&ctx(None, Some(Shape::LinkedTextBlock)));
        let bars = fetcher.cache_key(&ctx(None, Some(Shape::Bars)));
        let jpy = fetcher.cache_key(&ctx(
            Some("vs_currency = \"jpy\""),
            Some(Shape::LinkedTextBlock),
        ));
        assert_ne!(linked, bars);
        assert_ne!(linked, jpy);
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_options_before_network() {
        let err = CryptoTrendingFetcher
            .fetch(&ctx(Some("bogus = true"), Some(Shape::Text)))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("unknown field"));
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_vs_currency() {
        let err = CryptoTrendingFetcher
            .fetch(&ctx(Some("vs_currency = \"u\""), Some(Shape::Text)))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("invalid vs_currency"));
    }
}
