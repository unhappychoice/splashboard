//! `lastfm_scrobbles_today` — today's scrobbles for a Last.fm user, plus the currently-playing
//! track when one is in flight.
//!
//! Reads `user.getRecentTracks` with `from = midnight UTC today`. The currently-playing track
//! (signalled by `@attr.nowplaying = "true"` on the most recent entry) is split out from the
//! today-count rollup so the headline can choose between "♪ now playing" and a count summary
//! depending on whether the user is actively listening when the splash renders.
//!
//! Refresh interval is short (5 minutes) so the splash reflects "what I'm playing now" within
//! a coffee break, but not so short that we hammer the API on a fresh `cd`.

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::lastfm::client;
use crate::fetcher::lastfm::common::{ImageEntry, best_image};
use crate::fetcher::thumbnails;
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Body, EntriesData, Entry, ImageLinkedItem, ImageLinkedListData, LinkedLine,
    LinkedTextBlockData, MarkdownTextBlockData, Payload, Status, TextBlockData, TextData,
    TimelineData, TimelineEvent,
};
use crate::render::Shape;

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::ImageLinkedList,
    Shape::TextBlock,
    Shape::Text,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Badge,
    Shape::Timeline,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "user",
        type_hint: "string",
        required: true,
        default: None,
        description: "Last.fm username whose scrobbles to read.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=50)",
        required: false,
        default: Some("10"),
        description: "Maximum number of recently scrobbled tracks to show (excluding the currently-playing slot).",
    },
];

const DEFAULT_LIMIT: u32 = 10;
const MIN_LIMIT: u32 = 1;
const MAX_LIMIT: u32 = 50;

pub struct LastfmScrobblesToday;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub user: Option<String>,
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for LastfmScrobblesToday {
    fn name(&self) -> &str {
        "lastfm_scrobbles_today"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Today's scrobbles for a Last.fm user, with the currently-playing track surfaced when one is in flight. Reads `user.getRecentTracks` since UTC midnight; intentionally bounded to today so the cached splash doesn't drift across day boundaries."
    }
    fn refresh_interval(&self) -> u64 {
        5 * 60
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
        sample_body(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let user = opts
            .user
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                FetchError::Failed(
                    "lastfm_scrobbles_today: `user = \"<lastfm name>\"` is required".into(),
                )
            })?;
        let limit = opts
            .limit
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(MIN_LIMIT, MAX_LIMIT) as usize;
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);

        let snapshot = fetch_snapshot(user, limit).await?;
        let body = render_body(&snapshot, shape).await;
        Ok(payload(body))
    }
}

/// Parsed view of the response the rest of the rendering code reads from.
#[derive(Debug, Clone, Default)]
struct Snapshot {
    /// Tracks scrobbled since midnight UTC today, newest first. Excludes the nowplaying entry.
    tracks: Vec<TrackRow>,
    now_playing: Option<TrackRow>,
}

#[derive(Debug, Clone)]
struct TrackRow {
    name: String,
    artist: String,
    album: Option<String>,
    url: String,
    image_url: Option<String>,
    timestamp: Option<i64>,
}

impl Snapshot {
    fn today_count(&self) -> u64 {
        self.tracks.len() as u64
    }
}

async fn fetch_snapshot(user: &str, limit: usize) -> Result<Snapshot, FetchError> {
    let from = start_of_today_utc().to_string();
    // The nowplaying slot, when present, doesn't count toward `limit` from the API's perspective
    // — request `limit + 1` so a continuously listening user doesn't lose a row to it.
    let cap = (limit + 1).min((MAX_LIMIT + 1) as usize).to_string();
    let raw: RecentTracksResponse = client::get_json(
        "user.getRecentTracks",
        &[("user", user), ("from", &from), ("limit", &cap)],
    )
    .await?;
    let mut snap = parse_recent(raw);
    // Defensive cap — Last.fm sometimes returns one extra row past `limit` regardless of the
    // nowplaying split.
    snap.tracks.truncate(limit);
    Ok(snap)
}

fn start_of_today_utc() -> i64 {
    let today = Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap();
    chrono::DateTime::<Utc>::from_naive_utc_and_offset(today, Utc).timestamp()
}

fn parse_recent(raw: RecentTracksResponse) -> Snapshot {
    let mut snap = Snapshot::default();
    for raw_track in raw.recenttracks.track {
        let is_now = raw_track
            .attr
            .as_ref()
            .and_then(|a| a.nowplaying.as_deref())
            == Some("true");
        let row = TrackRow {
            name: raw_track.name,
            artist: raw_track.artist.text,
            album: raw_track.album.map(|a| a.text).filter(|s| !s.is_empty()),
            url: raw_track.url,
            image_url: best_image(&raw_track.image),
            timestamp: raw_track.date.and_then(|d| d.uts.parse().ok()),
        };
        if is_now {
            snap.now_playing = Some(row);
        } else {
            snap.tracks.push(row);
        }
    }
    snap
}

async fn render_body(snap: &Snapshot, shape: Shape) -> Body {
    match shape {
        Shape::ImageLinkedList => image_linked_body(snap).await,
        Shape::LinkedTextBlock => linked_text_body(snap),
        Shape::TextBlock => text_block_body(snap),
        Shape::MarkdownTextBlock => markdown_body(snap),
        Shape::Entries => entries_body(snap),
        Shape::Badge => badge_body(snap),
        Shape::Timeline => timeline_body(snap),
        _ => text_body(snap),
    }
}

fn text_body(snap: &Snapshot) -> Body {
    Body::Text(TextData {
        value: headline(snap),
    })
}

fn headline(snap: &Snapshot) -> String {
    if let Some(nowp) = &snap.now_playing {
        format!("♪ {} — {}", nowp.name, nowp.artist)
    } else if snap.today_count() == 1 {
        "1 scrobble today".into()
    } else if snap.today_count() > 0 {
        format!("{} scrobbles today", snap.today_count())
    } else {
        "quiet day".into()
    }
}

fn text_block_body(snap: &Snapshot) -> Body {
    Body::TextBlock(TextBlockData {
        lines: text_lines(snap),
    })
}

fn text_lines(snap: &Snapshot) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(nowp) = &snap.now_playing {
        lines.push(format!("♪ {} — {}", nowp.name, nowp.artist));
    }
    lines.extend(snap.tracks.iter().map(format_track_row));
    if lines.is_empty() {
        lines.push("quiet day".into());
    }
    lines
}

fn format_track_row(t: &TrackRow) -> String {
    let time = t
        .timestamp
        .map(format_time_label)
        .unwrap_or_else(|| "--:--".into());
    format!("{time}  {} — {}", t.name, t.artist)
}

fn format_time_label(ts: i64) -> String {
    chrono::DateTime::<Utc>::from_timestamp(ts, 0)
        .map(|dt| dt.format("%H:%M").to_string())
        .unwrap_or_else(|| "--:--".into())
}

fn markdown_body(snap: &Snapshot) -> Body {
    let mut lines = Vec::new();
    if let Some(nowp) = &snap.now_playing {
        lines.push(format!("- ▶ **{}** — {}", nowp.name, nowp.artist));
    }
    lines.extend(
        snap.tracks
            .iter()
            .map(|t| format!("- **{}** — {}", t.name, t.artist)),
    );
    if lines.is_empty() {
        lines.push("_quiet day_".into());
    }
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: lines.join("\n"),
    })
}

fn linked_text_body(snap: &Snapshot) -> Body {
    let mut items = Vec::new();
    if let Some(nowp) = &snap.now_playing {
        items.push(LinkedLine {
            text: format!("♪ {} — {}", nowp.name, nowp.artist),
            url: Some(nowp.url.clone()),
        });
    }
    items.extend(snap.tracks.iter().map(|t| LinkedLine {
        text: format_track_row(t),
        url: Some(t.url.clone()),
    }));
    if items.is_empty() {
        items.push(LinkedLine {
            text: "quiet day".into(),
            url: None,
        });
    }
    Body::LinkedTextBlock(LinkedTextBlockData { items })
}

async fn image_linked_body(snap: &Snapshot) -> Body {
    let rows = collect_rows_for_image(snap);
    let urls: Vec<Option<String>> = rows.iter().map(|(t, _, _)| t.image_url.clone()).collect();
    let paths = thumbnails::download_many(&urls).await;
    let items = rows
        .into_iter()
        .zip(paths)
        .map(|((t, title, subtitle), path)| ImageLinkedItem {
            title,
            url: Some(t.url.clone()),
            thumbnail_path: path.map(|p| p.to_string_lossy().into_owned()),
            subtitle,
        })
        .collect();
    Body::ImageLinkedList(ImageLinkedListData { items })
}

/// Produce `(track, title, subtitle)` rows in display order; nowplaying first.
fn collect_rows_for_image(snap: &Snapshot) -> Vec<(&TrackRow, String, Option<String>)> {
    let mut rows = Vec::new();
    if let Some(nowp) = &snap.now_playing {
        rows.push((
            nowp,
            format!("♪ {} — {}", nowp.name, nowp.artist),
            Some("now playing".into()),
        ));
    }
    for t in &snap.tracks {
        let subtitle = match t.timestamp {
            Some(ts) => Some(format!("{} · {}", format_time_label(ts), t.artist)),
            None => Some(t.artist.clone()),
        };
        rows.push((t, t.name.clone(), subtitle));
    }
    rows
}

fn entries_body(snap: &Snapshot) -> Body {
    let mut items = Vec::new();
    if let Some(nowp) = &snap.now_playing {
        items.push(Entry {
            key: nowp.name.clone(),
            value: Some(format!("▶ {}", nowp.artist)),
            status: Some(Status::Ok),
        });
    }
    items.extend(snap.tracks.iter().map(|t| Entry {
        key: t.name.clone(),
        value: Some(t.artist.clone()),
        status: None,
    }));
    if items.is_empty() {
        items.push(Entry {
            key: "today".into(),
            value: Some("no scrobbles yet".into()),
            status: None,
        });
    }
    Body::Entries(EntriesData { items })
}

fn badge_body(snap: &Snapshot) -> Body {
    let (status, label) = if let Some(nowp) = &snap.now_playing {
        (Status::Ok, format!("♪ {}", nowp.name))
    } else if snap.today_count() == 1 {
        (Status::Ok, "1 today".into())
    } else if snap.today_count() > 0 {
        (Status::Ok, format!("{} today", snap.today_count()))
    } else {
        (Status::Warn, "quiet day".into())
    };
    Body::Badge(BadgeData { status, label })
}

fn timeline_body(snap: &Snapshot) -> Body {
    let mut events = Vec::new();
    if let Some(nowp) = &snap.now_playing {
        events.push(TimelineEvent {
            timestamp: Utc::now().timestamp(),
            title: format!("♪ {} — {}", nowp.name, nowp.artist),
            detail: nowp.album.clone(),
            status: Some(Status::Ok),
        });
    }
    events.extend(snap.tracks.iter().map(|t| TimelineEvent {
        timestamp: t.timestamp.unwrap_or(0),
        title: format!("{} — {}", t.name, t.artist),
        detail: t.album.clone(),
        status: None,
    }));
    Body::Timeline(TimelineData { events })
}

fn sample_body(shape: Shape) -> Option<Body> {
    use crate::samples;
    Some(match shape {
        Shape::Text => samples::text("♪ Strawberry Swing — Coldplay"),
        Shape::TextBlock => samples::text_block(&[
            "♪ Strawberry Swing — Coldplay",
            "14:32  Lost Stars — Adam Levine",
            "14:28  Bloom — Beach House",
        ]),
        Shape::MarkdownTextBlock => samples::markdown(
            "- ▶ **Strawberry Swing** — Coldplay\n- **Lost Stars** — Adam Levine\n- **Bloom** — Beach House",
        ),
        Shape::LinkedTextBlock => samples::linked_text_block(&[
            (
                "♪ Strawberry Swing — Coldplay",
                Some("https://www.last.fm/music/Coldplay/_/Strawberry+Swing"),
            ),
            (
                "14:32  Lost Stars — Adam Levine",
                Some("https://www.last.fm/music/Adam+Levine/_/Lost+Stars"),
            ),
        ]),
        Shape::ImageLinkedList => samples::image_linked_list(&[
            (
                "♪ Strawberry Swing — Coldplay",
                Some("https://www.last.fm/music/Coldplay/_/Strawberry+Swing"),
                None,
                Some("now playing"),
            ),
            (
                "Lost Stars",
                Some("https://www.last.fm/music/Adam+Levine/_/Lost+Stars"),
                None,
                Some("14:32 · Adam Levine"),
            ),
        ]),
        Shape::Entries => samples::entries(&[
            ("Strawberry Swing", "▶ Coldplay"),
            ("Lost Stars", "Adam Levine"),
        ]),
        Shape::Badge => samples::badge(Status::Ok, "♪ Strawberry Swing"),
        Shape::Timeline => samples::timeline(&[
            (
                1_700_000_000,
                "♪ Strawberry Swing — Coldplay",
                Some("Viva la Vida"),
            ),
            (1_699_999_500, "Lost Stars — Adam Levine", None),
        ]),
        _ => return None,
    })
}

#[derive(Deserialize)]
struct RecentTracksResponse {
    recenttracks: RecentTracksInner,
}

#[derive(Deserialize)]
struct RecentTracksInner {
    #[serde(default)]
    track: Vec<RawTrack>,
}

#[derive(Deserialize)]
struct RawTrack {
    name: String,
    artist: ArtistObj,
    #[serde(default)]
    album: Option<AlbumObj>,
    url: String,
    #[serde(default)]
    image: Vec<ImageEntry>,
    #[serde(default)]
    date: Option<DateObj>,
    #[serde(rename = "@attr", default)]
    attr: Option<TrackAttr>,
}

#[derive(Deserialize)]
struct ArtistObj {
    #[serde(rename = "#text", default)]
    text: String,
}

#[derive(Deserialize)]
struct AlbumObj {
    #[serde(rename = "#text", default)]
    text: String,
}

#[derive(Deserialize)]
struct DateObj {
    uts: String,
}

#[derive(Deserialize)]
struct TrackAttr {
    #[serde(default)]
    nowplaying: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn ctx(options: Option<&str>, shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "lastfm-today".into(),
            timeout: Duration::from_secs(1),
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            ..Default::default()
        }
    }

    fn track(name: &str, artist: &str, ts: Option<i64>) -> TrackRow {
        TrackRow {
            name: name.into(),
            artist: artist.into(),
            album: None,
            url: format!("https://www.last.fm/music/{artist}/_/{name}"),
            image_url: None,
            timestamp: ts,
        }
    }

    fn snapshot(now: Option<TrackRow>, tracks: Vec<TrackRow>) -> Snapshot {
        Snapshot {
            now_playing: now,
            tracks,
        }
    }

    fn parse_recent_from(json: &str) -> Snapshot {
        let raw: RecentTracksResponse = serde_json::from_str(json).unwrap();
        parse_recent(raw)
    }

    #[test]
    fn options_default_to_none() {
        let opts = Options::default();
        assert!(opts.user.is_none());
        assert!(opts.limit.is_none());
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("bogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn options_deserialize_full() {
        let raw: toml::Value = toml::from_str("user = \"rj\"\nlimit = 5").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.user.as_deref(), Some("rj"));
        assert_eq!(opts.limit, Some(5));
    }

    #[test]
    fn parse_recent_splits_nowplaying_from_history() {
        let json = r##"{
            "recenttracks": {
                "track": [
                    {
                        "name": "Live Track",
                        "artist": {"#text": "Live Artist"},
                        "url": "https://www.last.fm/track/live",
                        "image": [],
                        "@attr": {"nowplaying": "true"}
                    },
                    {
                        "name": "Played Track",
                        "artist": {"#text": "Played Artist"},
                        "url": "https://www.last.fm/track/played",
                        "image": [],
                        "date": {"uts": "1700000000"}
                    }
                ]
            }
        }"##;
        let snap = parse_recent_from(json);
        let nowp = snap.now_playing.expect("nowplaying must be detected");
        assert_eq!(nowp.name, "Live Track");
        assert_eq!(snap.tracks.len(), 1);
        assert_eq!(snap.tracks[0].name, "Played Track");
        assert_eq!(snap.tracks[0].timestamp, Some(1_700_000_000));
    }

    #[test]
    fn parse_recent_handles_empty_track_list() {
        let snap = parse_recent_from(r#"{"recenttracks": {"track": []}}"#);
        assert!(snap.now_playing.is_none());
        assert_eq!(snap.tracks.len(), 0);
    }

    #[test]
    fn parse_recent_treats_nowplaying_false_as_history() {
        let json = r##"{
            "recenttracks": {
                "track": [
                    {
                        "name": "Track",
                        "artist": {"#text": "Artist"},
                        "url": "https://www.last.fm/x",
                        "image": [],
                        "date": {"uts": "1700000000"},
                        "@attr": {"nowplaying": "false"}
                    }
                ]
            }
        }"##;
        let snap = parse_recent_from(json);
        assert!(snap.now_playing.is_none());
        assert_eq!(snap.tracks.len(), 1);
    }

    #[test]
    fn parse_recent_picks_best_image() {
        let json = r##"{
            "recenttracks": {
                "track": [
                    {
                        "name": "T",
                        "artist": {"#text": "A"},
                        "url": "https://www.last.fm/x",
                        "image": [
                            {"#text": "https://example.com/small.png", "size": "small"},
                            {"#text": "https://example.com/mega.png", "size": "mega"}
                        ],
                        "date": {"uts": "1700000000"}
                    }
                ]
            }
        }"##;
        let snap = parse_recent_from(json);
        assert_eq!(
            snap.tracks[0].image_url.as_deref(),
            Some("https://example.com/mega.png")
        );
    }

    #[test]
    fn headline_prefers_now_playing_over_count() {
        let snap = snapshot(
            Some(track("Live", "Artist", None)),
            vec![track("Old", "Artist", Some(0))],
        );
        assert!(headline(&snap).starts_with("♪ Live"));
    }

    #[test]
    fn headline_singular_for_one_scrobble() {
        let snap = snapshot(None, vec![track("Only", "Artist", Some(0))]);
        assert_eq!(headline(&snap), "1 scrobble today");
    }

    #[test]
    fn headline_pluralises_for_many_scrobbles() {
        let snap = snapshot(
            None,
            (0..3)
                .map(|i| track(&format!("T{i}"), "A", Some(0)))
                .collect(),
        );
        assert_eq!(headline(&snap), "3 scrobbles today");
    }

    #[test]
    fn headline_reports_quiet_day_when_empty() {
        let snap = snapshot(None, vec![]);
        assert_eq!(headline(&snap), "quiet day");
    }

    #[test]
    fn format_track_row_uses_dashes_when_timestamp_missing() {
        let row = format_track_row(&track("Song", "Artist", None));
        assert!(row.starts_with("--:--"));
        assert!(row.contains("Song"));
        assert!(row.contains("Artist"));
    }

    #[test]
    fn format_time_label_renders_hour_minute_utc() {
        // 2023-04-22 10:15:30 UTC -> "10:15"
        assert_eq!(format_time_label(1_682_158_530), "10:15");
    }

    #[test]
    fn linked_text_body_falls_back_to_quiet_day_on_empty() {
        let body = linked_text_body(&Snapshot::default());
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked_text_block");
        };
        assert_eq!(b.items.len(), 1);
        assert_eq!(b.items[0].text, "quiet day");
        assert!(b.items[0].url.is_none());
    }

    #[test]
    fn linked_text_body_carries_track_urls() {
        let snap = snapshot(None, vec![track("Song", "Artist", Some(1_700_000_000))]);
        let Body::LinkedTextBlock(b) = linked_text_body(&snap) else {
            panic!("expected linked_text_block");
        };
        assert_eq!(b.items.len(), 1);
        assert!(b.items[0].text.contains("Song"));
        assert!(b.items[0].url.as_deref().unwrap().contains("Song"));
    }

    #[test]
    fn linked_text_body_prepends_now_playing_row() {
        let snap = snapshot(
            Some(track("Live", "Artist", None)),
            vec![track("Old", "Artist", Some(1_700_000_000))],
        );
        let Body::LinkedTextBlock(b) = linked_text_body(&snap) else {
            panic!("expected linked_text_block");
        };
        assert_eq!(b.items.len(), 2);
        assert!(b.items[0].text.starts_with("♪ Live"));
    }

    #[test]
    fn text_block_body_includes_quiet_day_when_no_tracks() {
        let body = text_block_body(&Snapshot::default());
        let Body::TextBlock(b) = body else {
            panic!("expected text_block");
        };
        assert_eq!(b.lines, vec!["quiet day".to_string()]);
    }

    #[test]
    fn markdown_body_renders_bullets() {
        let snap = snapshot(None, vec![track("Song", "Artist", Some(0))]);
        let Body::MarkdownTextBlock(m) = markdown_body(&snap) else {
            panic!("expected markdown");
        };
        assert!(m.value.contains("**Song**"));
        assert!(m.value.starts_with("- "));
    }

    #[test]
    fn markdown_body_uses_italic_quiet_day_when_empty() {
        let Body::MarkdownTextBlock(m) = markdown_body(&Snapshot::default()) else {
            panic!("expected markdown");
        };
        assert_eq!(m.value, "_quiet day_");
    }

    #[test]
    fn entries_body_tags_now_playing_with_ok_status() {
        let snap = snapshot(Some(track("Live", "Artist", None)), vec![]);
        let Body::Entries(e) = entries_body(&snap) else {
            panic!("expected entries");
        };
        assert_eq!(e.items[0].key, "Live");
        assert_eq!(e.items[0].status, Some(Status::Ok));
    }

    #[test]
    fn entries_body_falls_back_when_empty() {
        let Body::Entries(e) = entries_body(&Snapshot::default()) else {
            panic!("expected entries");
        };
        assert_eq!(e.items.len(), 1);
        assert_eq!(e.items[0].key, "today");
    }

    #[test]
    fn badge_body_warns_on_quiet_day() {
        let Body::Badge(b) = badge_body(&Snapshot::default()) else {
            panic!("expected badge");
        };
        assert_eq!(b.status, Status::Warn);
        assert_eq!(b.label, "quiet day");
    }

    #[test]
    fn badge_body_uses_singular_label_for_one_scrobble() {
        let snap = snapshot(None, vec![track("Only", "A", Some(0))]);
        let Body::Badge(b) = badge_body(&snap) else {
            panic!("expected badge");
        };
        assert_eq!(b.label, "1 today");
        assert_eq!(b.status, Status::Ok);
    }

    #[test]
    fn badge_body_promotes_now_playing_to_label() {
        let snap = snapshot(Some(track("Live", "Artist", None)), vec![]);
        let Body::Badge(b) = badge_body(&snap) else {
            panic!("expected badge");
        };
        assert!(b.label.starts_with("♪ Live"));
    }

    #[test]
    fn timeline_body_orders_nowplaying_first() {
        let snap = snapshot(
            Some(track("Live", "Artist", None)),
            vec![track("Old", "Artist", Some(1_700_000_000))],
        );
        let Body::Timeline(t) = timeline_body(&snap) else {
            panic!("expected timeline");
        };
        assert_eq!(t.events.len(), 2);
        assert!(t.events[0].title.starts_with("♪ Live"));
        assert_eq!(t.events[1].timestamp, 1_700_000_000);
    }

    #[test]
    fn collect_rows_for_image_subtitles_fall_back_to_artist_when_no_timestamp() {
        let snap = snapshot(None, vec![track("Song", "Artist", None)]);
        let rows = collect_rows_for_image(&snap);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].2.as_deref(), Some("Artist"));
    }

    #[test]
    fn fetcher_exposes_catalog_metadata_and_samples() {
        let fetcher = LastfmScrobblesToday;
        assert_eq!(fetcher.name(), "lastfm_scrobbles_today");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
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
        let fetcher = LastfmScrobblesToday;
        let a = fetcher.cache_key(&ctx(Some("user = \"rj\""), Some(Shape::LinkedTextBlock)));
        let b = fetcher.cache_key(&ctx(Some("user = \"rj\""), Some(Shape::Badge)));
        let c = fetcher.cache_key(&ctx(Some("user = \"other\""), Some(Shape::LinkedTextBlock)));
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[tokio::test]
    async fn fetch_requires_user_option() {
        let err = LastfmScrobblesToday
            .fetch(&ctx(None, Some(Shape::Text)))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("`user ="));
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_options_before_network() {
        let err = LastfmScrobblesToday
            .fetch(&ctx(Some("user = \"rj\"\nbogus = true"), Some(Shape::Text)))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("unknown field"));
    }

    #[tokio::test]
    async fn fetch_rejects_blank_user() {
        let err = LastfmScrobblesToday
            .fetch(&ctx(Some("user = \"   \""), Some(Shape::Text)))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("`user ="));
    }

    fn rich_snapshot() -> Snapshot {
        snapshot(
            Some(track("Live", "Now Artist", None)),
            vec![
                track("First", "Artist A", Some(1_682_158_530)),
                track("Second", "Artist B", None),
            ],
        )
    }

    #[tokio::test]
    async fn render_body_dispatches_each_shape_to_its_builder() {
        let snap = rich_snapshot();
        for &shape in SHAPES {
            let body = render_body(&snap, shape).await;
            assert_eq!(crate::render::shape_of(&body), shape, "shape {shape:?}");
        }
    }

    #[tokio::test]
    async fn render_body_falls_back_to_text_for_unsupported_shape() {
        let body = render_body(&rich_snapshot(), Shape::Ratio).await;
        let Body::Text(t) = body else {
            panic!("expected text fallback");
        };
        assert!(t.value.starts_with("♪ Live"));
    }

    #[test]
    fn text_body_wraps_the_headline() {
        let snap = snapshot(None, vec![track("Only", "A", Some(0))]);
        let Body::Text(t) = text_body(&snap) else {
            panic!("expected text");
        };
        assert_eq!(t.value, "1 scrobble today");
    }

    #[test]
    fn text_block_body_lists_now_playing_then_history() {
        let Body::TextBlock(b) = text_block_body(&rich_snapshot()) else {
            panic!("expected text_block");
        };
        assert_eq!(b.lines.len(), 3);
        assert!(b.lines[0].starts_with("♪ Live"));
        assert!(b.lines[1].starts_with("10:15"));
        assert!(b.lines[1].contains("First"));
        assert!(b.lines[2].starts_with("--:--"));
    }

    #[test]
    fn markdown_body_prepends_now_playing_bullet() {
        let Body::MarkdownTextBlock(m) = markdown_body(&rich_snapshot()) else {
            panic!("expected markdown");
        };
        assert!(m.value.starts_with("- ▶ **Live**"));
        assert!(m.value.contains("- **First** — Artist A"));
    }

    #[test]
    fn entries_body_lists_history_tracks_without_status() {
        let snap = snapshot(None, vec![track("Hist", "Artist", Some(0))]);
        let Body::Entries(e) = entries_body(&snap) else {
            panic!("expected entries");
        };
        assert_eq!(e.items.len(), 1);
        assert_eq!(e.items[0].key, "Hist");
        assert_eq!(e.items[0].value.as_deref(), Some("Artist"));
        assert!(e.items[0].status.is_none());
    }

    #[test]
    fn collect_rows_for_image_orders_now_playing_first_and_timestamps_history() {
        let snap = rich_snapshot();
        let rows = collect_rows_for_image(&snap);
        assert_eq!(rows.len(), 3);
        assert!(rows[0].1.starts_with("♪ Live"));
        assert_eq!(rows[0].2.as_deref(), Some("now playing"));
        assert_eq!(rows[1].1, "First");
        assert_eq!(rows[1].2.as_deref(), Some("10:15 · Artist A"));
        assert_eq!(rows[2].2.as_deref(), Some("Artist B"));
    }

    #[tokio::test]
    async fn image_linked_body_builds_rows_with_empty_thumbnails_offline() {
        let Body::ImageLinkedList(d) = image_linked_body(&rich_snapshot()).await else {
            panic!("expected image_linked_list");
        };
        assert_eq!(d.items.len(), 3);
        assert!(d.items[0].title.starts_with("♪ Live"));
        assert_eq!(d.items[0].subtitle.as_deref(), Some("now playing"));
        assert!(d.items.iter().all(|i| i.thumbnail_path.is_none()));
    }
}
