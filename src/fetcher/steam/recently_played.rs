//! `steam_recently_played` — games the configured Steam user has played in the last two
//! weeks, with per-game minutes-this-week from the `playtime_2weeks` field.
//!
//! Reads `IPlayerService/GetRecentlyPlayedGames/v1`. Rolls up the `playtime_2weeks` totals
//! across the response for the headline ("Xh across N games") so the catalog's
//! `steam_playtime_week` candidate is covered by this fetcher's `Text` shape rather than a
//! separate read.

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::steam::client;
use crate::fetcher::steam::common::format_minutes;
use crate::fetcher::steam::games::{
    GameRow, badge_body, bars_body, entries_body, image_body, image_linked_body, linked_text_body,
    markdown_body, number_series_body, ratio_body, text_block_body, text_body,
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
    Shape::Ratio,
    Shape::NumberSeries,
    Shape::Image,
    Shape::Badge,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "steam_id",
        type_hint: "string (Steam64 id)",
        required: false,
        default: None,
        description: "Steam64 id of the user whose recent playtime to read. Falls back to the `STEAM_ID` env var when omitted.",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=20)",
        required: false,
        default: Some("5"),
        description: "Maximum number of recently-played games to display.",
    },
];

const DEFAULT_COUNT: u32 = 5;
const MIN_COUNT: u32 = 1;
const MAX_COUNT: u32 = 20;
const EMPTY_LABEL: &str = "no playtime this week";

pub struct SteamRecentlyPlayed;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub steam_id: Option<String>,
    pub count: Option<u32>,
}

#[async_trait]
impl Fetcher for SteamRecentlyPlayed {
    fn name(&self) -> &str {
        "steam_recently_played"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Games the configured Steam user has launched in the last two weeks, with per-game minutes from `playtime_2weeks`. The Text shape sums the window for a `steam_playtime_week`-style headline; Bars / Entries break it out per game."
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
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let steam_id = client::resolve_steam_id(opts.steam_id.as_deref())?;
        let count = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_COUNT) as usize;
        let shape = ctx.shape.unwrap_or(Shape::Bars);

        let games = fetch_games(&steam_id, count).await?;
        let rows = rows_from_games(&games);
        let body = render_body(&rows, &games, shape).await;
        Ok(payload(body))
    }
}

async fn fetch_games(steam_id: &str, count: usize) -> Result<Vec<RawGame>, FetchError> {
    let count_str = count.to_string();
    let raw: RecentlyPlayedResponse = client::get_json(
        "IPlayerService/GetRecentlyPlayedGames/v1/",
        &[("steamid", steam_id), ("count", &count_str)],
    )
    .await?;
    Ok(raw.response.games.unwrap_or_default())
}

fn rows_from_games(games: &[RawGame]) -> Vec<GameRow> {
    games
        .iter()
        .enumerate()
        .map(|(i, g)| GameRow {
            rank: i + 1,
            appid: g.appid,
            name: display_name(g),
            value: g.playtime_2weeks as u64,
            value_label: format_minutes(g.playtime_2weeks),
        })
        .collect()
}

fn display_name(g: &RawGame) -> String {
    if g.name.trim().is_empty() {
        format!("app/{}", g.appid)
    } else {
        g.name.clone()
    }
}

async fn render_body(rows: &[GameRow], games: &[RawGame], shape: Shape) -> Body {
    match shape {
        Shape::Bars => bars_body(rows),
        Shape::Entries => entries_body(rows, EMPTY_LABEL),
        Shape::TextBlock => text_block_body(rows, EMPTY_LABEL),
        Shape::MarkdownTextBlock => markdown_body(rows, EMPTY_LABEL),
        Shape::LinkedTextBlock => linked_text_body(rows, EMPTY_LABEL),
        Shape::ImageLinkedList => image_linked_body(rows).await,
        Shape::Ratio => ratio_body(rows),
        Shape::NumberSeries => number_series_body(rows),
        Shape::Image => image_body(rows).await,
        Shape::Badge => badge_body(rows, "this week", "no playtime"),
        _ => text_body_with_summary(rows, games),
    }
}

/// Text shape collapses the row list into a `Xh across N games this week` headline so the
/// catalog's `steam_playtime_week` slot is covered without a second fetcher.
fn text_body_with_summary(rows: &[GameRow], games: &[RawGame]) -> Body {
    if rows.is_empty() {
        return text_body(rows, EMPTY_LABEL);
    }
    let total: u32 = games.iter().map(|g| g.playtime_2weeks).sum();
    Body::Text(crate::payload::TextData {
        value: format!(
            "{} across {} games this week",
            format_minutes(total),
            rows.len()
        ),
    })
}

#[derive(Debug, Deserialize)]
struct RecentlyPlayedResponse {
    response: RecentlyPlayedBody,
}

#[derive(Debug, Default, Deserialize)]
struct RecentlyPlayedBody {
    #[serde(default)]
    games: Option<Vec<RawGame>>,
}

#[derive(Debug, Deserialize)]
struct RawGame {
    appid: u32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    playtime_2weeks: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_games() -> Vec<RawGame> {
        vec![
            RawGame {
                appid: 730,
                name: "Counter-Strike 2".into(),
                playtime_2weeks: 480,
            },
            RawGame {
                appid: 570,
                name: "Dota 2".into(),
                playtime_2weeks: 120,
            },
        ]
    }

    #[test]
    fn fetcher_metadata_is_in_steam_family() {
        let f = SteamRecentlyPlayed;
        assert_eq!(f.name(), "steam_recently_played");
        assert_eq!(f.safety(), Safety::Safe);
        assert!(f.refresh_interval() > 0);
        assert!(f.shapes().contains(&Shape::Bars));
        assert!(f.shapes().contains(&Shape::Text));
    }

    #[test]
    fn rows_from_games_assigns_ranks_in_order() {
        let rows = rows_from_games(&sample_games());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].rank, 1);
        assert_eq!(rows[1].rank, 2);
        assert_eq!(rows[0].appid, 730);
        assert_eq!(rows[0].value_label, "8h");
    }

    #[test]
    fn display_name_falls_back_to_app_id_when_name_is_blank() {
        let g = RawGame {
            appid: 42,
            name: "  ".into(),
            playtime_2weeks: 10,
        };
        assert_eq!(display_name(&g), "app/42");
    }

    #[test]
    fn text_shape_collapses_into_weekly_summary() {
        let games = sample_games();
        let rows = rows_from_games(&games);
        let Body::Text(t) = text_body_with_summary(&rows, &games) else {
            panic!("expected text body");
        };
        // 480 + 120 = 600 minutes -> "10h"
        assert!(t.value.starts_with("10h"));
        assert!(t.value.contains("2 games"));
    }

    #[test]
    fn text_shape_falls_back_to_empty_label_when_no_games() {
        let Body::Text(t) = text_body_with_summary(&[], &[]) else {
            panic!("expected text body");
        };
        assert_eq!(t.value, EMPTY_LABEL);
    }

    #[test]
    fn options_struct_rejects_unknown_keys() {
        let raw = toml::Value::try_from(serde_json::json!({"bogus": 1})).unwrap();
        let err = parse_options::<Options>(Some(&raw)).unwrap_err();
        assert!(err.contains("invalid options"));
    }

    #[test]
    fn options_struct_parses_steam_id_and_count() {
        let raw = toml::Value::try_from(serde_json::json!({
            "steam_id": "76561197960287930",
            "count": 10
        }))
        .unwrap();
        let opts: Options = parse_options(Some(&raw)).unwrap();
        assert_eq!(opts.steam_id.as_deref(), Some("76561197960287930"));
        assert_eq!(opts.count, Some(10));
    }

    #[tokio::test]
    async fn render_body_dispatches_each_shape_to_its_body_variant() {
        let games = sample_games();
        let rows = rows_from_games(&games);
        assert!(matches!(
            render_body(&rows, &games, Shape::Bars).await,
            Body::Bars(_)
        ));
        assert!(matches!(
            render_body(&rows, &games, Shape::Entries).await,
            Body::Entries(_)
        ));
        assert!(matches!(
            render_body(&rows, &games, Shape::TextBlock).await,
            Body::TextBlock(_)
        ));
        assert!(matches!(
            render_body(&rows, &games, Shape::MarkdownTextBlock).await,
            Body::MarkdownTextBlock(_)
        ));
        assert!(matches!(
            render_body(&rows, &games, Shape::LinkedTextBlock).await,
            Body::LinkedTextBlock(_)
        ));
        assert!(matches!(
            render_body(&rows, &games, Shape::NumberSeries).await,
            Body::NumberSeries(_)
        ));
        assert!(matches!(
            render_body(&rows, &games, Shape::Ratio).await,
            Body::Ratio(_)
        ));
        assert!(matches!(
            render_body(&rows, &games, Shape::Badge).await,
            Body::Badge(_)
        ));
    }

    #[tokio::test]
    async fn render_body_text_arm_is_the_catch_all_weekly_summary() {
        // `Shape::Text` has no explicit match arm — it exercises the `_ =>` branch.
        let games = sample_games();
        let rows = rows_from_games(&games);
        let body = render_body(&rows, &games, Shape::Text).await;
        assert!(matches!(&body, Body::Text(t) if t.value.contains("2 games this week")));
    }

    #[tokio::test]
    async fn render_body_image_shapes_resolve_without_network_on_empty_rows() {
        assert!(matches!(
            render_body(&[], &[], Shape::ImageLinkedList).await,
            Body::ImageLinkedList(_)
        ));
        assert!(matches!(
            render_body(&[], &[], Shape::Image).await,
            Body::Image(_)
        ));
    }

    #[test]
    fn cache_key_is_name_prefixed_and_varies_with_options() {
        let f = SteamRecentlyPlayed;
        let base = f.cache_key(&FetchContext::default());
        assert!(base.starts_with("steam_recently_played-"));
        let with_opts = f.cache_key(&FetchContext {
            options: Some(toml::from_str("count = 10").unwrap()),
            ..Default::default()
        });
        assert_ne!(base, with_opts);
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_options_before_any_network() {
        let f = SteamRecentlyPlayed;
        let ctx = FetchContext {
            options: Some(toml::from_str("bogus = 1").unwrap()),
            ..Default::default()
        };
        let err = f.fetch(&ctx).await.unwrap_err();
        assert!(matches!(err, FetchError::Failed(m) if m.contains("invalid options")));
    }
}
