//! `steam_owned_games` — the configured Steam user's library ranked by total playtime
//! (default) or most-recently launched (`sort = "recent"`).
//!
//! Reads `IPlayerService/GetOwnedGames/v1?include_appinfo=1&include_played_free_games=1`. The
//! complementary read to `steam_recently_played`: same source family, all-time window.

use async_trait::async_trait;
use chrono::TimeZone;
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
        description: "Steam64 id of the user whose library to read. Falls back to the `STEAM_ID` env var when omitted.",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=20)",
        required: false,
        default: Some("10"),
        description: "Maximum number of games to display.",
    },
    OptionSchema {
        name: "sort",
        type_hint: "\"playtime\" | \"recent\"",
        required: false,
        default: Some("\"playtime\""),
        description: "Sort order: `playtime` ranks by total minutes; `recent` ranks by last-launched time.",
    },
];

const DEFAULT_COUNT: u32 = 10;
const MIN_COUNT: u32 = 1;
const MAX_COUNT: u32 = 20;
const EMPTY_LABEL: &str = "no owned games";

pub struct SteamOwnedGames;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub steam_id: Option<String>,
    pub count: Option<u32>,
    pub sort: Option<Sort>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    #[default]
    Playtime,
    Recent,
}

#[async_trait]
impl Fetcher for SteamOwnedGames {
    fn name(&self) -> &str {
        "steam_owned_games"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "The configured Steam user's library ranked by total minutes (`sort = \"playtime\"`, default) or by most-recently launched (`sort = \"recent\"`). Complement to `steam_recently_played` over the all-time window."
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
        let steam_id = client::resolve_steam_id(opts.steam_id.as_deref())?;
        let count = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_COUNT) as usize;
        let sort = opts.sort.unwrap_or_default();
        let shape = ctx.shape.unwrap_or(Shape::Bars);

        let games = fetch_games(&steam_id).await?;
        let rows = rows_from_games(&games, sort, count);
        let body = render_body(&rows, &games, shape, sort).await;
        Ok(payload(body))
    }
}

async fn fetch_games(steam_id: &str) -> Result<Vec<RawGame>, FetchError> {
    let raw: OwnedGamesResponse = client::get_json(
        "IPlayerService/GetOwnedGames/v1/",
        &[
            ("steamid", steam_id),
            ("include_appinfo", "1"),
            ("include_played_free_games", "1"),
        ],
    )
    .await?;
    Ok(raw.response.games.unwrap_or_default())
}

fn rows_from_games(games: &[RawGame], sort: Sort, count: usize) -> Vec<GameRow> {
    let mut sorted: Vec<&RawGame> = games.iter().collect();
    match sort {
        Sort::Playtime => sorted.sort_by_key(|g| std::cmp::Reverse(g.playtime_forever)),
        Sort::Recent => sorted.sort_by_key(|g| std::cmp::Reverse(g.rtime_last_played)),
    }
    sorted
        .into_iter()
        .take(count)
        .enumerate()
        .map(|(i, g)| GameRow {
            rank: i + 1,
            appid: g.appid,
            name: display_name(g),
            value: g.playtime_forever as u64,
            // Bars / NumberSeries always carry minutes so the bar heights stay comparable
            // even when sorted by recency; only the displayed `value_label` switches with the
            // sort axis so the row "means what its column says".
            value_label: match sort {
                Sort::Playtime => format_minutes(g.playtime_forever),
                Sort::Recent => last_played_label(g.rtime_last_played),
            },
        })
        .collect()
}

/// Steam's `rtime_last_played` is unix seconds. A value of 0 means "never launched" (Steam
/// records this for owned-but-never-opened games). Recency reads faster as a relative pill
/// (`"today"` / `"2d ago"` / `"1w ago"`); falls back to an absolute ISO date past ~30 days so
/// the label doesn't decay to a giant unit. The 6-hour refresh interval means the relative
/// label can drift by at most one quantum (e.g. "today" → "1d ago") before the next refresh,
/// which is acceptable for a glance widget.
fn last_played_label(ts: u64) -> String {
    if ts == 0 {
        return "never".into();
    }
    let now = chrono::Utc::now().timestamp();
    relative_label(now, ts as i64)
}

fn relative_label(now: i64, ts: i64) -> String {
    let secs = (now - ts).max(0);
    const DAY: i64 = 86_400;
    const WEEK: i64 = 7 * DAY;
    let days = secs / DAY;
    match days {
        0 => "today".into(),
        1 => "yesterday".into(),
        2..=6 => format!("{days}d ago"),
        7..=29 => format!("{}w ago", days / 7),
        _ => match chrono::Utc.timestamp_opt(ts, 0).single() {
            Some(dt) => dt.format("%Y-%m-%d").to_string(),
            None => format!("ts:{ts}"),
        },
    }
}

fn display_name(g: &RawGame) -> String {
    if g.name.trim().is_empty() {
        format!("app/{}", g.appid)
    } else {
        g.name.clone()
    }
}

async fn render_body(rows: &[GameRow], games: &[RawGame], shape: Shape, sort: Sort) -> Body {
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
        Shape::Badge => badge_body(rows, sort.label(), "library empty"),
        _ => text_body_with_summary(rows, games),
    }
}

impl Sort {
    fn label(self) -> &'static str {
        match self {
            Self::Playtime => "all-time",
            Self::Recent => "recent",
        }
    }
}

fn text_body_with_summary(rows: &[GameRow], games: &[RawGame]) -> Body {
    if rows.is_empty() {
        return text_body(rows, EMPTY_LABEL);
    }
    let total: u32 = games.iter().map(|g| g.playtime_forever).sum();
    Body::Text(crate::payload::TextData {
        value: format!(
            "{} across {} owned games",
            format_minutes(total),
            games.len()
        ),
    })
}

#[derive(Debug, Deserialize)]
struct OwnedGamesResponse {
    response: OwnedGamesBody,
}

#[derive(Debug, Default, Deserialize)]
struct OwnedGamesBody {
    #[serde(default)]
    games: Option<Vec<RawGame>>,
}

#[derive(Debug, Deserialize)]
struct RawGame {
    appid: u32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    playtime_forever: u32,
    #[serde(default)]
    rtime_last_played: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_games() -> Vec<RawGame> {
        vec![
            RawGame {
                appid: 730,
                name: "Counter-Strike 2".into(),
                playtime_forever: 4830,
                rtime_last_played: 1_715_000_000,
            },
            RawGame {
                appid: 570,
                name: "Dota 2".into(),
                playtime_forever: 12_000,
                rtime_last_played: 1_700_000_000,
            },
            RawGame {
                appid: 440,
                name: "Team Fortress 2".into(),
                playtime_forever: 0,
                rtime_last_played: 1_720_000_000,
            },
        ]
    }

    #[test]
    fn fetcher_metadata_is_in_steam_family() {
        let f = SteamOwnedGames;
        assert_eq!(f.name(), "steam_owned_games");
        assert_eq!(f.safety(), Safety::Safe);
        assert!(f.refresh_interval() > 0);
    }

    #[test]
    fn playtime_sort_ranks_dota_first_and_labels_with_hours() {
        let rows = rows_from_games(&sample_games(), Sort::Playtime, 10);
        assert_eq!(rows[0].name, "Dota 2");
        assert_eq!(rows[0].rank, 1);
        assert_eq!(rows[1].name, "Counter-Strike 2");
        // 12_000 min / 60 = 200h
        assert_eq!(rows[0].value_label, "200h");
    }

    #[test]
    fn recent_sort_ranks_tf2_first_and_labels_with_relative_pill() {
        let rows = rows_from_games(&sample_games(), Sort::Recent, 10);
        assert_eq!(rows[0].name, "Team Fortress 2");
        assert_eq!(rows[1].name, "Counter-Strike 2");
        // ts 1_720_000_000 is years in the past from any plausible test runtime → ISO fallback.
        assert!(
            rows[0].value_label.starts_with("20"),
            "expected ISO date fallback for old ts, got {:?}",
            rows[0].value_label
        );
    }

    #[test]
    fn last_played_label_reports_never_when_timestamp_is_zero() {
        assert_eq!(last_played_label(0), "never");
    }

    #[test]
    fn relative_label_picks_unit_by_age_bucket() {
        // Anchor a fixed `now` so the table doesn't drift with the real clock.
        let now = 1_750_000_000_i64;
        const DAY: i64 = 86_400;
        assert_eq!(relative_label(now, now), "today");
        assert_eq!(relative_label(now, now - DAY), "yesterday");
        assert_eq!(relative_label(now, now - 3 * DAY), "3d ago");
        assert_eq!(relative_label(now, now - 14 * DAY), "2w ago");
        // 60 days back falls through to the ISO date fallback.
        let iso = relative_label(now, now - 60 * DAY);
        assert!(iso.starts_with("20"), "expected ISO fallback, got {iso:?}");
        assert_eq!(iso.len(), 10);
    }

    #[test]
    fn relative_label_does_not_panic_on_future_timestamps() {
        // Clock skew between Steam and the host can produce ts > now; clamp to "today".
        let now = 1_750_000_000_i64;
        assert_eq!(relative_label(now, now + 3600), "today");
    }

    #[test]
    fn count_caps_the_returned_rows() {
        let rows = rows_from_games(&sample_games(), Sort::Playtime, 1);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn display_name_falls_back_to_app_id_when_name_is_blank() {
        let g = RawGame {
            appid: 42,
            name: "  ".into(),
            playtime_forever: 0,
            rtime_last_played: 0,
        };
        assert_eq!(display_name(&g), "app/42");
    }

    #[test]
    fn text_shape_summarises_total_library_playtime() {
        let games = sample_games();
        let rows = rows_from_games(&games, Sort::Playtime, 10);
        let Body::Text(t) = text_body_with_summary(&rows, &games) else {
            panic!("expected text body");
        };
        // 4830 + 12000 + 0 = 16830 -> 280h
        assert!(t.value.starts_with("280h"));
        assert!(t.value.contains("3 owned"));
    }

    #[test]
    fn sort_default_is_playtime() {
        assert_eq!(Sort::default(), Sort::Playtime);
    }

    #[test]
    fn options_struct_parses_sort_keyword() {
        let raw = toml::Value::try_from(serde_json::json!({"sort": "recent"})).unwrap();
        let opts: Options = parse_options(Some(&raw)).unwrap();
        assert_eq!(opts.sort, Some(Sort::Recent));
    }
}
