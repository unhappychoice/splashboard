//! `steam_charts` — global top-played games on Steam right now (concurrent-player count).
//!
//! Reads the public `ISteamChartsService/GetMostPlayedGames/v1` endpoint (no auth required).
//! That endpoint returns appid + concurrent count but no name, so the fetcher fans out parallel
//! `store.steampowered.com/api/appdetails?filters=basic` calls to resolve names; failures fall
//! through to a `app/<appid>` placeholder so a single 404 doesn't sink the chart.
//!
//! The Steam slot in the `*_trending` family alongside `crypto_trending` /
//! `huggingface_trending` / `lastfm_charts` — "what the world is playing right now".

use std::collections::HashMap;
use std::sync::OnceLock;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::steam::client;
use crate::fetcher::steam::common::format_count;
use crate::fetcher::steam::games::{
    GameRow, badge_body, bars_body, entries_body, image_body, image_linked_body, linked_text_body,
    markdown_body, number_series_body, text_block_body, text_body,
};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;

const SHAPES: &[Shape] = &[
    Shape::Bars,
    Shape::Entries,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::LinkedTextBlock,
    Shape::ImageLinkedList,
    Shape::Text,
    Shape::NumberSeries,
    Shape::Image,
    Shape::Badge,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "count",
    type_hint: "integer (1..=30)",
    required: false,
    default: Some("10"),
    description: "Number of chart entries to display.",
}];

const DEFAULT_COUNT: u32 = 10;
const MIN_COUNT: u32 = 1;
const MAX_COUNT: u32 = 30;
const APPDETAIL_CONCURRENCY: usize = 4;
const EMPTY_LABEL: &str = "chart unavailable";

pub struct SteamCharts;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub count: Option<u32>,
}

#[async_trait]
impl Fetcher for SteamCharts {
    fn name(&self) -> &str {
        "steam_charts"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Global most-played games on Steam right now, ranked by concurrent in-game count. Reads the public `ISteamChartsService/GetMostPlayedGames` endpoint (no auth) and resolves game names via parallel `appdetails` calls — the Steam slot in the `*_trending` family."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60 * 6
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
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let count = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_COUNT) as usize;
        let shape = ctx.shape.unwrap_or(Shape::Bars);

        let ranks = fetch_ranks(count).await?;
        let names = resolve_names(&ranks).await;
        let rows = rows_from_ranks(&ranks, &names);
        Ok(payload(render_body(&rows, shape).await))
    }
}

async fn fetch_ranks(count: usize) -> Result<Vec<RawRank>, FetchError> {
    let raw: ChartsResponse =
        client::get_json_public("ISteamChartsService/GetMostPlayedGames/v1/", &[]).await?;
    let mut ranks = raw.response.ranks.unwrap_or_default();
    ranks.truncate(count);
    Ok(ranks)
}

async fn resolve_names(ranks: &[RawRank]) -> HashMap<u32, String> {
    let sem = appdetail_semaphore();
    let mut set: JoinSet<(u32, Option<String>)> = JoinSet::new();
    for r in ranks {
        let appid = r.appid;
        let sem = sem.clone();
        set.spawn(async move {
            let _permit = sem.acquire_owned().await.ok();
            (appid, fetch_app_name(appid).await)
        });
    }
    let mut out = HashMap::new();
    while let Some(res) = set.join_next().await {
        if let Ok((appid, Some(name))) = res {
            out.insert(appid, name);
        }
    }
    out
}

fn appdetail_semaphore() -> std::sync::Arc<Semaphore> {
    static SEM: OnceLock<std::sync::Arc<Semaphore>> = OnceLock::new();
    SEM.get_or_init(|| std::sync::Arc::new(Semaphore::new(APPDETAIL_CONCURRENCY)))
        .clone()
}

async fn fetch_app_name(appid: u32) -> Option<String> {
    let appid_str = appid.to_string();
    let raw: AppDetailsResponse = client::get_store_json(
        "api/appdetails",
        &[
            ("appids", &appid_str),
            ("filters", "basic"),
            ("cc", "us"),
            ("l", "english"),
        ],
    )
    .await
    .ok()?;
    raw.0
        .get(&appid_str)
        .and_then(|entry| entry.data.as_ref())
        .map(|data| data.name.clone())
        .filter(|n| !n.is_empty())
}

fn rows_from_ranks(ranks: &[RawRank], names: &HashMap<u32, String>) -> Vec<GameRow> {
    ranks
        .iter()
        .map(|r| GameRow {
            rank: r.rank as usize,
            appid: r.appid,
            name: names
                .get(&r.appid)
                .cloned()
                .unwrap_or_else(|| format!("app/{}", r.appid)),
            value: r.peak_in_game,
            value_label: peak_label(r.peak_in_game),
        })
        .collect()
}

/// `1.2M peak` — the chart endpoint reports 24-hour peak concurrent count rather than the
/// live "right now" number (Valve doesn't expose the latter on this rollup), so the unit
/// reads "peak" rather than "players" to stay honest about what's shown.
fn peak_label(n: u64) -> String {
    format!("{} peak", format_count(n))
}

async fn render_body(rows: &[GameRow], shape: Shape) -> Body {
    match shape {
        Shape::Bars => bars_body(rows),
        Shape::Entries => entries_body(rows, EMPTY_LABEL),
        Shape::TextBlock => text_block_body(rows, EMPTY_LABEL),
        Shape::MarkdownTextBlock => markdown_body(rows, EMPTY_LABEL),
        Shape::LinkedTextBlock => linked_text_body(rows, EMPTY_LABEL),
        Shape::ImageLinkedList => image_linked_body(rows).await,
        Shape::NumberSeries => number_series_body(rows),
        Shape::Image => image_body(rows).await,
        Shape::Badge => badge_body(rows, "today", "chart unavailable"),
        _ => text_body(rows, EMPTY_LABEL),
    }
}

#[derive(Debug, Deserialize)]
struct ChartsResponse {
    response: ChartsBody,
}

#[derive(Debug, Default, Deserialize)]
struct ChartsBody {
    #[serde(default)]
    ranks: Option<Vec<RawRank>>,
}

#[derive(Debug, Deserialize)]
struct RawRank {
    rank: u32,
    appid: u32,
    /// 24-hour peak concurrent players. `ISteamChartsService/GetMostPlayedGames/v1` omits the
    /// instantaneous "right now" count; this `peak_in_game` rollup is what the endpoint exposes
    /// and what SteamDB / steamcharts.com surface as their headline metric.
    #[serde(default)]
    peak_in_game: u64,
}

/// Steam returns `appdetails` as a top-level map keyed by appid string. Each value carries a
/// `success` flag plus an optional `data` block; failed lookups omit `data` entirely.
#[derive(Debug, Deserialize)]
struct AppDetailsResponse(HashMap<String, AppDetailsEntry>);

#[derive(Debug, Deserialize)]
struct AppDetailsEntry {
    #[serde(default)]
    data: Option<AppDetailsData>,
}

#[derive(Debug, Deserialize)]
struct AppDetailsData {
    #[serde(default)]
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetcher_metadata_is_in_steam_family_and_safe_for_public_endpoint() {
        let f = SteamCharts;
        assert_eq!(f.name(), "steam_charts");
        assert_eq!(f.safety(), Safety::Safe);
        assert!(f.refresh_interval() > 0);
    }

    #[test]
    fn rows_from_ranks_fills_names_when_present_and_falls_back_otherwise() {
        let ranks = vec![
            RawRank {
                rank: 1,
                appid: 730,
                peak_in_game: 1_500_000,
            },
            RawRank {
                rank: 2,
                appid: 999_999,
                peak_in_game: 50_000,
            },
        ];
        let mut names = HashMap::new();
        names.insert(730_u32, "Counter-Strike 2".into());

        let rows = rows_from_ranks(&ranks, &names);
        assert_eq!(rows[0].name, "Counter-Strike 2");
        assert_eq!(rows[1].name, "app/999999");
        assert_eq!(rows[0].value, 1_500_000);
        assert_eq!(rows[0].value_label, "1.5M peak");
    }

    #[test]
    fn peak_label_drops_player_unit_and_keeps_magnitude() {
        assert_eq!(peak_label(0), "0 peak");
        assert_eq!(peak_label(750), "750 peak");
        assert_eq!(peak_label(1_500_000), "1.5M peak");
    }

    #[test]
    fn options_clamp_count_to_supported_range() {
        let opts = Options { count: Some(99) };
        let clamped = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_COUNT);
        assert_eq!(clamped, MAX_COUNT);
    }

    #[test]
    fn options_struct_rejects_unknown_keys() {
        let raw = toml::Value::try_from(serde_json::json!({"unknown": 1})).unwrap();
        let err = parse_options::<Options>(Some(&raw)).unwrap_err();
        assert!(err.contains("invalid options"));
    }

    fn sample_rows() -> Vec<GameRow> {
        vec![
            GameRow {
                rank: 1,
                appid: 730,
                name: "Counter-Strike 2".into(),
                value: 1_500_000,
                value_label: "1.5M peak".into(),
            },
            GameRow {
                rank: 2,
                appid: 570,
                name: "Dota 2".into(),
                value: 600_000,
                value_label: "600k peak".into(),
            },
        ]
    }

    #[tokio::test]
    async fn render_body_dispatches_each_shape_to_its_body_variant() {
        let rows = sample_rows();
        assert!(matches!(
            render_body(&rows, Shape::Bars).await,
            Body::Bars(_)
        ));
        assert!(matches!(
            render_body(&rows, Shape::Entries).await,
            Body::Entries(_)
        ));
        assert!(matches!(
            render_body(&rows, Shape::TextBlock).await,
            Body::TextBlock(_)
        ));
        assert!(matches!(
            render_body(&rows, Shape::MarkdownTextBlock).await,
            Body::MarkdownTextBlock(_)
        ));
        assert!(matches!(
            render_body(&rows, Shape::LinkedTextBlock).await,
            Body::LinkedTextBlock(_)
        ));
        assert!(matches!(
            render_body(&rows, Shape::NumberSeries).await,
            Body::NumberSeries(_)
        ));
        assert!(matches!(
            render_body(&rows, Shape::Badge).await,
            Body::Badge(_)
        ));
    }

    #[tokio::test]
    async fn render_body_text_arm_is_the_catch_all_fallback() {
        // `Shape::Text` is not an explicit match arm — it exercises the `_ =>` branch.
        let rows = sample_rows();
        let Body::Text(t) = render_body(&rows, Shape::Text).await else {
            panic!("expected text body");
        };
        assert!(t.value.contains("Counter-Strike 2"));
    }

    #[tokio::test]
    async fn render_body_image_shapes_resolve_without_network_on_empty_rows() {
        assert!(matches!(
            render_body(&[], Shape::ImageLinkedList).await,
            Body::ImageLinkedList(_)
        ));
        assert!(matches!(
            render_body(&[], Shape::Image).await,
            Body::Image(_)
        ));
    }

    #[tokio::test]
    async fn resolve_names_is_empty_when_no_ranks_supplied() {
        assert!(resolve_names(&[]).await.is_empty());
    }

    #[test]
    fn appdetail_semaphore_returns_one_shared_instance() {
        let a = appdetail_semaphore();
        let b = appdetail_semaphore();
        assert!(std::sync::Arc::ptr_eq(&a, &b));
        assert!(a.available_permits() <= APPDETAIL_CONCURRENCY);
    }

    fn ctx(options: Option<toml::Value>) -> FetchContext {
        FetchContext {
            widget_id: "w".into(),
            timeout: std::time::Duration::from_secs(1),
            options,
            ..Default::default()
        }
    }

    #[test]
    fn cache_key_is_name_prefixed_and_varies_with_options() {
        let f = SteamCharts;
        let base = f.cache_key(&ctx(None));
        assert!(base.starts_with("steam_charts-"));

        let opts = toml::Value::try_from(serde_json::json!({"count": 5})).unwrap();
        assert_ne!(f.cache_key(&ctx(Some(opts))), base);
    }

    #[test]
    fn charts_response_parses_ranks_with_peak_count() {
        let raw = r#"{"response":{"ranks":[{"rank":1,"appid":730,"peak_in_game":1500000}]}}"#;
        let parsed: ChartsResponse = serde_json::from_str(raw).unwrap();
        let ranks = parsed.response.ranks.unwrap();
        assert_eq!(ranks.len(), 1);
        assert_eq!(ranks[0].appid, 730);
        assert_eq!(ranks[0].peak_in_game, 1_500_000);
    }

    #[test]
    fn charts_body_defaults_ranks_to_none_when_field_absent() {
        let parsed: ChartsResponse = serde_json::from_str(r#"{"response":{}}"#).unwrap();
        assert!(parsed.response.ranks.is_none());
    }

    #[test]
    fn raw_rank_defaults_peak_in_game_to_zero_when_omitted() {
        let parsed: RawRank = serde_json::from_str(r#"{"rank":3,"appid":440}"#).unwrap();
        assert_eq!(parsed.peak_in_game, 0);
    }

    #[test]
    fn app_details_response_keeps_data_on_success_and_drops_it_on_failure() {
        let raw = r#"{"730":{"success":true,"data":{"name":"Counter-Strike 2"}},"9999":{"success":false}}"#;
        let parsed: AppDetailsResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(
            parsed.0["730"].data.as_ref().unwrap().name,
            "Counter-Strike 2"
        );
        assert!(parsed.0["9999"].data.is_none());
    }

    #[test]
    fn app_details_data_defaults_name_to_empty_string() {
        let parsed: AppDetailsData = serde_json::from_str("{}").unwrap();
        assert!(parsed.name.is_empty());
    }
}
