//! `steam_player_summary` — the configured Steam user's profile snapshot: persona name,
//! Steam level, online status, currently-playing game, and last-logoff timestamp.
//!
//! Reads `ISteamUser/GetPlayerSummaries/v2` and `IPlayerService/GetSteamLevel/v1` in parallel.
//! Same single-read-many-shapes pattern as `lastfm` family's user-scoped fetchers: every shape
//! reformats the same snapshot.

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::steam::client;
use crate::fetcher::thumbnails;
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Body, EntriesData, Entry, ImageData, ImageLinkedItem, ImageLinkedListData,
    LinkedLine, LinkedTextBlockData, MarkdownTextBlockData, Payload, Status, TextBlockData,
    TextData,
};
use crate::render::Shape;

const SHAPES: &[Shape] = &[
    Shape::TextBlock,
    Shape::Text,
    Shape::MarkdownTextBlock,
    Shape::LinkedTextBlock,
    Shape::ImageLinkedList,
    Shape::Entries,
    Shape::Badge,
    Shape::Image,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "steam_id",
    type_hint: "string (Steam64 id)",
    required: false,
    default: None,
    description: "Steam64 id of the profile to read. Falls back to the `STEAM_ID` env var when omitted.",
}];

pub struct SteamPlayerSummary;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub steam_id: Option<String>,
}

#[async_trait]
impl Fetcher for SteamPlayerSummary {
    fn name(&self) -> &str {
        "steam_player_summary"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "The configured Steam user's profile snapshot: persona name, level, online status, currently-playing game, and last-logoff timestamp. Reads `GetPlayerSummaries` and `GetSteamLevel` in parallel."
    }
    fn refresh_interval(&self) -> u64 {
        30 * 60
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
        let shape = ctx.shape.unwrap_or(Shape::TextBlock);

        let snapshot = fetch_snapshot(&steam_id).await?;
        Ok(payload(render_body(&snapshot, shape).await))
    }
}

#[derive(Debug, Clone)]
struct Snapshot {
    steam_id: String,
    persona: String,
    profile_url: String,
    avatar_url: Option<String>,
    state: PersonaState,
    current_game: Option<String>,
    last_logoff: Option<i64>,
    level: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PersonaState {
    Offline,
    Online,
    Busy,
    Away,
    Snooze,
    LookingToTrade,
    LookingToPlay,
    InGame,
}

impl PersonaState {
    fn from_code(code: u8, in_game: bool) -> Self {
        if in_game {
            return Self::InGame;
        }
        match code {
            1 => Self::Online,
            2 => Self::Busy,
            3 => Self::Away,
            4 => Self::Snooze,
            5 => Self::LookingToTrade,
            6 => Self::LookingToPlay,
            _ => Self::Offline,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Online => "online",
            Self::Busy => "busy",
            Self::Away => "away",
            Self::Snooze => "snooze",
            Self::LookingToTrade => "trading",
            Self::LookingToPlay => "looking to play",
            Self::InGame => "in game",
        }
    }

    fn badge_status(self) -> Status {
        match self {
            Self::Online | Self::InGame | Self::LookingToTrade | Self::LookingToPlay => Status::Ok,
            Self::Busy | Self::Away | Self::Snooze | Self::Offline => Status::Warn,
        }
    }

    /// Per-row `Entries` status pip. Offline returns `None` so a fresh-spawned offline profile
    /// doesn't decorate the status row with a yellow pip — only "actively in something other
    /// than online" surfaces a warning.
    fn entry_status(self) -> Option<Status> {
        match self {
            Self::Offline => None,
            other => Some(other.badge_status()),
        }
    }
}

async fn fetch_snapshot(steam_id: &str) -> Result<Snapshot, FetchError> {
    let (player_res, level_res) = tokio::join!(fetch_player(steam_id), fetch_level(steam_id));
    let player = player_res?;
    let level = level_res.ok().flatten();
    Ok(Snapshot {
        steam_id: steam_id.to_string(),
        persona: player.personaname,
        profile_url: player.profileurl,
        avatar_url: Some(player.avatarfull).filter(|s| !s.is_empty()),
        state: PersonaState::from_code(player.personastate, player.gameid.is_some()),
        current_game: player
            .gameextrainfo
            .filter(|s| !s.is_empty())
            .or_else(|| player.gameid.map(|id| format!("app/{id}"))),
        last_logoff: player.lastlogoff,
        level,
    })
}

async fn fetch_player(steam_id: &str) -> Result<RawPlayer, FetchError> {
    let raw: PlayerSummariesResponse = client::get_json(
        "ISteamUser/GetPlayerSummaries/v2/",
        &[("steamids", steam_id)],
    )
    .await?;
    raw.response
        .players
        .into_iter()
        .next()
        .ok_or_else(|| FetchError::Failed(format!("steam: no player for id {steam_id}")))
}

async fn fetch_level(steam_id: &str) -> Result<Option<u32>, FetchError> {
    let raw: SteamLevelResponse =
        client::get_json("IPlayerService/GetSteamLevel/v1/", &[("steamid", steam_id)]).await?;
    Ok(raw.response.player_level)
}

async fn render_body(snap: &Snapshot, shape: Shape) -> Body {
    match shape {
        Shape::Text => text_body(snap),
        Shape::TextBlock => text_block_body(snap),
        Shape::MarkdownTextBlock => markdown_body(snap),
        Shape::LinkedTextBlock => linked_text_body(snap),
        Shape::ImageLinkedList => image_linked_body(snap).await,
        Shape::Entries => entries_body(snap),
        Shape::Badge => badge_body(snap),
        Shape::Image => image_body(snap).await,
        _ => text_body(snap),
    }
}

fn headline(snap: &Snapshot) -> String {
    let mut parts = vec![snap.persona.clone()];
    if let Some(lv) = snap.level {
        parts.push(format!("Lv {lv}"));
    }
    if let Some(game) = &snap.current_game {
        parts.push(format!("in {game}"));
    } else {
        parts.push(snap.state.label().into());
    }
    parts.join(" · ")
}

fn text_body(snap: &Snapshot) -> Body {
    Body::Text(TextData {
        value: headline(snap),
    })
}

fn text_block_body(snap: &Snapshot) -> Body {
    Body::TextBlock(TextBlockData { lines: lines(snap) })
}

fn markdown_body(snap: &Snapshot) -> Body {
    let value = lines(snap)
        .into_iter()
        .enumerate()
        .map(|(i, line)| if i == 0 { format!("**{line}**") } else { line })
        .collect::<Vec<_>>()
        .join("\n");
    Body::MarkdownTextBlock(MarkdownTextBlockData { value })
}

fn lines(snap: &Snapshot) -> Vec<String> {
    let mut lines = vec![snap.persona.clone()];
    if let Some(lv) = snap.level {
        lines.push(format!("Level {lv}"));
    }
    lines.push(format!("Status: {}", snap.state.label()));
    if let Some(game) = &snap.current_game {
        lines.push(format!("Playing: {game}"));
    } else if let Some(ts) = snap.last_logoff {
        lines.push(format!("Last seen: {}", format_last_logoff(ts)));
    }
    lines
}

fn linked_text_body(snap: &Snapshot) -> Body {
    Body::LinkedTextBlock(LinkedTextBlockData {
        items: vec![LinkedLine {
            text: headline(snap),
            url: Some(snap.profile_url.clone()),
        }],
    })
}

async fn image_linked_body(snap: &Snapshot) -> Body {
    let path = match snap.avatar_url.as_deref() {
        Some(url) => thumbnails::download_to_cache(url)
            .await
            .ok()
            .flatten()
            .map(|p| p.to_string_lossy().into_owned()),
        None => None,
    };
    Body::ImageLinkedList(ImageLinkedListData {
        items: vec![ImageLinkedItem {
            title: snap.persona.clone(),
            url: Some(snap.profile_url.clone()),
            thumbnail_path: path,
            subtitle: Some(subtitle(snap)),
        }],
    })
}

fn subtitle(snap: &Snapshot) -> String {
    match (&snap.level, &snap.current_game) {
        (Some(lv), Some(game)) => format!("Lv {lv} · in {game}"),
        (Some(lv), None) => format!("Lv {lv} · {}", snap.state.label()),
        (None, Some(game)) => format!("in {game}"),
        (None, None) => snap.state.label().into(),
    }
}

fn entries_body(snap: &Snapshot) -> Body {
    let mut items = vec![Entry {
        key: "persona".into(),
        value: Some(snap.persona.clone()),
        status: None,
    }];
    if let Some(lv) = snap.level {
        items.push(Entry {
            key: "level".into(),
            value: Some(lv.to_string()),
            status: None,
        });
    }
    items.push(Entry {
        key: "status".into(),
        value: Some(snap.state.label().into()),
        status: snap.state.entry_status(),
    });
    if let Some(game) = &snap.current_game {
        items.push(Entry {
            key: "playing".into(),
            value: Some(game.clone()),
            status: None,
        });
    }
    if let Some(ts) = snap.last_logoff {
        items.push(Entry {
            key: "last seen".into(),
            value: Some(format_last_logoff(ts)),
            status: None,
        });
    }
    items.push(Entry {
        key: "steam id".into(),
        value: Some(snap.steam_id.clone()),
        status: None,
    });
    Body::Entries(EntriesData { items })
}

fn badge_body(snap: &Snapshot) -> Body {
    let label = match &snap.current_game {
        Some(game) => format!("in {game}"),
        None => snap.state.label().into(),
    };
    Body::Badge(BadgeData {
        status: snap.state.badge_status(),
        label,
    })
}

async fn image_body(snap: &Snapshot) -> Body {
    let path = match snap.avatar_url.as_deref() {
        Some(url) => thumbnails::download_to_cache(url)
            .await
            .ok()
            .flatten()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        None => String::new(),
    };
    Body::Image(ImageData { path })
}

fn format_last_logoff(ts: i64) -> String {
    // The `lastlogoff` field is unix seconds UTC; format it as an ISO date so cached payloads
    // don't bake in a stale relative-time string. Renderers that want "3h ago" can re-derive
    // it from a Timeline shape — this fetcher only emits absolute snapshots.
    match Utc.timestamp_opt(ts, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d").to_string(),
        None => format!("ts:{ts}"),
    }
}

#[derive(Debug, Deserialize)]
struct PlayerSummariesResponse {
    response: PlayerSummariesBody,
}

#[derive(Debug, Default, Deserialize)]
struct PlayerSummariesBody {
    #[serde(default)]
    players: Vec<RawPlayer>,
}

#[derive(Debug, Deserialize)]
struct RawPlayer {
    #[serde(default)]
    personaname: String,
    #[serde(default)]
    profileurl: String,
    #[serde(default)]
    avatarfull: String,
    #[serde(default)]
    personastate: u8,
    #[serde(default)]
    lastlogoff: Option<i64>,
    #[serde(default)]
    gameid: Option<String>,
    #[serde(default)]
    gameextrainfo: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SteamLevelResponse {
    response: SteamLevelBody,
}

#[derive(Debug, Default, Deserialize)]
struct SteamLevelBody {
    #[serde(default)]
    player_level: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(state: PersonaState, current_game: Option<&str>) -> Snapshot {
        Snapshot {
            steam_id: "76561197960287930".into(),
            persona: "Robin".into(),
            profile_url: "https://steamcommunity.com/id/robinwalker/".into(),
            avatar_url: Some("https://example.com/avatar.jpg".into()),
            state,
            current_game: current_game.map(String::from),
            last_logoff: Some(1_700_000_000),
            level: Some(25),
        }
    }

    #[test]
    fn fetcher_metadata_is_in_steam_family() {
        let f = SteamPlayerSummary;
        assert_eq!(f.name(), "steam_player_summary");
        assert_eq!(f.safety(), Safety::Safe);
        assert!(f.shapes().contains(&Shape::Badge));
    }

    #[test]
    fn persona_state_promotes_in_game_when_gameid_is_present() {
        assert_eq!(PersonaState::from_code(1, true), PersonaState::InGame);
        assert_eq!(PersonaState::from_code(0, true), PersonaState::InGame);
    }

    #[test]
    fn persona_state_labels_each_variant() {
        for s in [
            PersonaState::Offline,
            PersonaState::Online,
            PersonaState::Busy,
            PersonaState::Away,
            PersonaState::Snooze,
            PersonaState::LookingToTrade,
            PersonaState::LookingToPlay,
            PersonaState::InGame,
        ] {
            assert!(!s.label().is_empty());
        }
    }

    #[test]
    fn persona_state_offline_drops_entry_pip_but_keeps_warn_badge_status() {
        // Offline is benign in the per-row Entries view (no warning pip) but still degrades
        // the all-in-one Badge status — "user is not online" is a warning compared to
        // "actively in game".
        assert!(PersonaState::Offline.entry_status().is_none());
        assert_eq!(PersonaState::Offline.badge_status(), Status::Warn);
        assert_eq!(PersonaState::Online.entry_status(), Some(Status::Ok));
        assert_eq!(PersonaState::Busy.badge_status(), Status::Warn);
    }

    #[test]
    fn headline_includes_level_and_current_game_when_present() {
        let snap = sample(PersonaState::InGame, Some("Counter-Strike 2"));
        let h = headline(&snap);
        assert!(h.contains("Robin"));
        assert!(h.contains("Lv 25"));
        assert!(h.contains("in Counter-Strike 2"));
    }

    #[test]
    fn headline_falls_back_to_state_label_when_not_in_game() {
        let snap = sample(PersonaState::Online, None);
        let h = headline(&snap);
        assert!(h.contains("Robin"));
        assert!(h.contains("online"));
    }

    #[test]
    fn entries_body_tags_status_row_with_state_status() {
        let snap = sample(PersonaState::Online, None);
        let Body::Entries(e) = entries_body(&snap) else {
            panic!("expected entries");
        };
        let status_row = e.items.iter().find(|i| i.key == "status").unwrap();
        assert_eq!(status_row.status, Some(Status::Ok));
        assert!(e.items.iter().any(|i| i.key == "steam id"));
    }

    #[test]
    fn linked_text_body_carries_profile_url() {
        let snap = sample(PersonaState::Online, None);
        let Body::LinkedTextBlock(b) = linked_text_body(&snap) else {
            panic!("expected linked_text_block");
        };
        assert_eq!(b.items.len(), 1);
        assert_eq!(
            b.items[0].url.as_deref(),
            Some("https://steamcommunity.com/id/robinwalker/")
        );
    }

    #[test]
    fn badge_body_uses_in_game_label_when_playing() {
        let snap = sample(PersonaState::InGame, Some("Dota 2"));
        let Body::Badge(b) = badge_body(&snap) else {
            panic!("expected badge");
        };
        assert_eq!(b.status, Status::Ok);
        assert!(b.label.contains("Dota 2"));
    }

    #[test]
    fn format_last_logoff_renders_iso_date_for_valid_timestamps() {
        let s = format_last_logoff(1_700_000_000);
        // 2023-11-14 UTC
        assert_eq!(s, "2023-11-14");
    }

    #[test]
    fn markdown_body_bolds_only_the_persona_line() {
        let snap = sample(PersonaState::Online, None);
        let Body::MarkdownTextBlock(m) = markdown_body(&snap) else {
            panic!("expected markdown");
        };
        assert!(m.value.starts_with("**Robin**"));
        assert!(m.value.contains("\nLevel 25"));
    }

    #[test]
    fn subtitle_combines_level_and_current_game_when_both_present() {
        let snap = sample(PersonaState::InGame, Some("Dota 2"));
        assert_eq!(subtitle(&snap), "Lv 25 · in Dota 2");
    }

    #[test]
    fn subtitle_falls_back_when_level_or_game_missing() {
        let mut snap = sample(PersonaState::Online, None);
        assert_eq!(subtitle(&snap), "Lv 25 · online");
        snap.level = None;
        assert_eq!(subtitle(&snap), "online");
        snap.current_game = Some("Portal 2".into());
        assert_eq!(subtitle(&snap), "in Portal 2");
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
    fn fetcher_exposes_refresh_interval_description_and_option_schema() {
        let f = SteamPlayerSummary;
        assert!(f.refresh_interval() > 0);
        assert!(!f.description().is_empty());
        assert_eq!(f.option_schemas().len(), 1);
        assert_eq!(f.option_schemas()[0].name, "steam_id");
        assert_eq!(f.shapes().len(), SHAPES.len());
    }

    #[test]
    fn cache_key_is_name_prefixed_and_varies_with_options() {
        let f = SteamPlayerSummary;
        let base = f.cache_key(&ctx(None));
        assert!(base.starts_with("steam_player_summary-"));

        let opts = toml::Value::try_from(serde_json::json!({"steam_id": "123"})).unwrap();
        assert_ne!(f.cache_key(&ctx(Some(opts))), base);
    }

    #[test]
    fn options_struct_rejects_unknown_keys() {
        let raw = toml::Value::try_from(serde_json::json!({"unknown": 1})).unwrap();
        assert!(parse_options::<Options>(Some(&raw)).is_err());
    }

    #[test]
    fn persona_state_from_code_maps_each_status_code_when_not_in_game() {
        assert_eq!(PersonaState::from_code(1, false), PersonaState::Online);
        assert_eq!(PersonaState::from_code(2, false), PersonaState::Busy);
        assert_eq!(PersonaState::from_code(3, false), PersonaState::Away);
        assert_eq!(PersonaState::from_code(4, false), PersonaState::Snooze);
        assert_eq!(
            PersonaState::from_code(5, false),
            PersonaState::LookingToTrade
        );
        assert_eq!(
            PersonaState::from_code(6, false),
            PersonaState::LookingToPlay
        );
        assert_eq!(PersonaState::from_code(0, false), PersonaState::Offline);
        assert_eq!(PersonaState::from_code(99, false), PersonaState::Offline);
    }

    #[test]
    fn text_body_wraps_the_headline() {
        let snap = sample(PersonaState::Online, None);
        let Body::Text(t) = text_body(&snap) else {
            panic!("expected text");
        };
        assert_eq!(t.value, headline(&snap));
    }

    #[test]
    fn text_block_body_carries_the_lines() {
        let snap = sample(PersonaState::Online, None);
        let Body::TextBlock(b) = text_block_body(&snap) else {
            panic!("expected text_block");
        };
        assert_eq!(b.lines, lines(&snap));
        assert_eq!(b.lines[0], "Robin");
    }

    #[test]
    fn lines_reports_playing_game_instead_of_last_seen_when_in_game() {
        let snap = sample(PersonaState::InGame, Some("Half-Life"));
        let ls = lines(&snap);
        assert!(ls.iter().any(|l| l == "Playing: Half-Life"));
        assert!(!ls.iter().any(|l| l.starts_with("Last seen:")));
    }

    #[test]
    fn entries_body_adds_playing_row_when_in_a_game() {
        let snap = sample(PersonaState::InGame, Some("Team Fortress 2"));
        let Body::Entries(e) = entries_body(&snap) else {
            panic!("expected entries");
        };
        let playing = e.items.iter().find(|i| i.key == "playing").unwrap();
        assert_eq!(playing.value.as_deref(), Some("Team Fortress 2"));
        assert!(e.items.iter().any(|i| i.key == "last seen"));
    }

    #[test]
    fn badge_body_uses_state_label_when_not_in_a_game() {
        let snap = sample(PersonaState::Away, None);
        let Body::Badge(b) = badge_body(&snap) else {
            panic!("expected badge");
        };
        assert_eq!(b.status, Status::Warn);
        assert_eq!(b.label, "away");
    }

    #[test]
    fn format_last_logoff_falls_back_to_raw_ts_for_out_of_range_input() {
        let s = format_last_logoff(i64::MAX);
        assert_eq!(s, format!("ts:{}", i64::MAX));
    }

    fn no_avatar(state: PersonaState, game: Option<&str>) -> Snapshot {
        Snapshot {
            avatar_url: None,
            ..sample(state, game)
        }
    }

    #[tokio::test]
    async fn render_body_dispatches_each_shape_to_its_body_variant() {
        let snap = no_avatar(PersonaState::Online, None);
        assert!(matches!(
            render_body(&snap, Shape::Text).await,
            Body::Text(_)
        ));
        assert!(matches!(
            render_body(&snap, Shape::TextBlock).await,
            Body::TextBlock(_)
        ));
        assert!(matches!(
            render_body(&snap, Shape::MarkdownTextBlock).await,
            Body::MarkdownTextBlock(_)
        ));
        assert!(matches!(
            render_body(&snap, Shape::LinkedTextBlock).await,
            Body::LinkedTextBlock(_)
        ));
        assert!(matches!(
            render_body(&snap, Shape::Entries).await,
            Body::Entries(_)
        ));
        assert!(matches!(
            render_body(&snap, Shape::Badge).await,
            Body::Badge(_)
        ));
    }

    #[tokio::test]
    async fn render_body_image_shapes_resolve_without_network_when_avatar_absent() {
        let snap = no_avatar(PersonaState::Online, None);
        assert!(matches!(
            render_body(&snap, Shape::Image).await,
            Body::Image(_)
        ));
        let Body::Image(img) = render_body(&snap, Shape::Image).await else {
            panic!("expected image");
        };
        assert!(img.path.is_empty());

        let Body::ImageLinkedList(list) = render_body(&snap, Shape::ImageLinkedList).await else {
            panic!("expected image_linked_list");
        };
        assert_eq!(list.items.len(), 1);
        assert!(list.items[0].thumbnail_path.is_none());
        assert_eq!(list.items[0].title, "Robin");
    }

    #[tokio::test]
    async fn render_body_unlisted_shape_falls_back_to_text() {
        // `Shape::Ratio` is not in SHAPES, so it exercises the `_ =>` catch-all arm.
        let snap = no_avatar(PersonaState::Online, None);
        assert!(matches!(
            render_body(&snap, Shape::Ratio).await,
            Body::Text(_)
        ));
    }

    #[test]
    fn player_summaries_response_parses_first_player_and_defaults_missing_fields() {
        let raw =
            r#"{"response":{"players":[{"steamid":"1","personaname":"Gabe","personastate":1}]}}"#;
        let parsed: PlayerSummariesResponse = serde_json::from_str(raw).unwrap();
        let player = &parsed.response.players[0];
        assert_eq!(player.personaname, "Gabe");
        assert_eq!(player.personastate, 1);
        assert!(player.profileurl.is_empty());
        assert!(player.avatarfull.is_empty());
        assert!(player.lastlogoff.is_none());
        assert!(player.gameid.is_none());
        assert!(player.gameextrainfo.is_none());
    }

    #[test]
    fn player_summaries_body_defaults_players_to_empty_when_absent() {
        let parsed: PlayerSummariesResponse = serde_json::from_str(r#"{"response":{}}"#).unwrap();
        assert!(parsed.response.players.is_empty());
    }

    #[test]
    fn steam_level_response_reads_level_and_defaults_to_none_when_absent() {
        let with_level: SteamLevelResponse =
            serde_json::from_str(r#"{"response":{"player_level":42}}"#).unwrap();
        assert_eq!(with_level.response.player_level, Some(42));

        let without: SteamLevelResponse = serde_json::from_str(r#"{"response":{}}"#).unwrap();
        assert!(without.response.player_level.is_none());
    }

    /// One-shot local HTTP server that answers a single request with a tiny PNG, so the avatar
    /// download path can be exercised without reaching the real network.
    fn serve_png_once() -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let body: &[u8] = b"\x89PNG\r\n\x1a\navatar-bytes";
            let header = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: image/png\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes()).unwrap();
            stream.write_all(body).unwrap();
            stream.flush().unwrap();
        });
        (format!("http://{addr}/avatar.png"), handle)
    }

    /// RAII guard: points `SPLASHBOARD_HOME` at a temp dir and restores it on drop, so a
    /// panic mid-test can't leak the override into later tests sharing the process env.
    struct SplashHomeGuard(Option<String>);

    impl SplashHomeGuard {
        fn set(path: &std::path::Path) -> Self {
            let previous = std::env::var("SPLASHBOARD_HOME").ok();
            unsafe { std::env::set_var("SPLASHBOARD_HOME", path) };
            Self(previous)
        }
    }

    impl Drop for SplashHomeGuard {
        fn drop(&mut self) {
            unsafe {
                match self.0.take() {
                    Some(value) => std::env::set_var("SPLASHBOARD_HOME", value),
                    None => std::env::remove_var("SPLASHBOARD_HOME"),
                }
            }
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn image_bodies_resolve_avatar_through_thumbnail_cache() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let _home = SplashHomeGuard::set(tmp.path());

        let (url, server) = serve_png_once();
        let snap = Snapshot {
            avatar_url: Some(url),
            ..sample(PersonaState::Online, None)
        };

        // `image_linked_body` performs the one network download; `image_body` then resolves the
        // same avatar URL from the on-disk cache without a second request.
        let Body::ImageLinkedList(list) = render_body(&snap, Shape::ImageLinkedList).await else {
            panic!("expected image_linked_list");
        };
        server.join().unwrap();
        let Body::Image(img) = render_body(&snap, Shape::Image).await else {
            panic!("expected image");
        };

        let thumb = list.items[0]
            .thumbnail_path
            .as_deref()
            .expect("avatar should resolve to a cached path");
        assert!(thumb.ends_with(".png"));
        assert_eq!(list.items[0].title, "Robin");
        assert_eq!(img.path, thumb);
    }
}
