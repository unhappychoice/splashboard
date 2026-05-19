//! `lastfm_charts` — Last.fm's global "right now" chart: top artists or top tracks across
//! every scrobble, world-wide.
//!
//! Reads `chart.getTopArtists` or `chart.getTopTracks` depending on `kind`. No user-specific
//! data, no `period` — `chart.*` endpoints return the global ranking as-of the request
//! moment. Companion to the shipped `crypto_trending` / `huggingface_trending` /
//! `wikipedia_trending` / `reddit_trending` family ("what the world is listening to" axis).

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::lastfm::client;
use crate::fetcher::lastfm::common::{ImageEntry, best_image, parse_count};
use crate::fetcher::lastfm::top::{
    TopRow, bars_body, entries_body, headline, image_linked_body, linked_text_body, markdown_body,
    text_block_body, text_body,
};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;

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
        name: "kind",
        type_hint: "\"artists\" | \"tracks\"",
        required: false,
        default: Some("\"artists\""),
        description: "Which Last.fm chart to rank — global top artists or global top tracks.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=50)",
        required: false,
        default: Some("10"),
        description: "Number of chart entries to display.",
    },
];

const DEFAULT_LIMIT: u32 = 10;
const MIN_LIMIT: u32 = 1;
const MAX_LIMIT: u32 = 50;

pub struct LastfmCharts;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub kind: Option<Kind>,
    pub limit: Option<u32>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Artists,
    Tracks,
}

impl Kind {
    fn method(self) -> &'static str {
        match self {
            Self::Artists => "chart.getTopArtists",
            Self::Tracks => "chart.getTopTracks",
        }
    }

    fn empty_label(self) -> &'static str {
        match self {
            Self::Artists => "no chart artists",
            Self::Tracks => "no chart tracks",
        }
    }
}

#[async_trait]
impl Fetcher for LastfmCharts {
    fn name(&self) -> &str {
        "lastfm_charts"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Last.fm's global chart of top artists or top tracks across every scrobble — \"what the world is listening to right now\". No user binding, no rolling window. The Last.fm slot in the `*_trending` family alongside `crypto_trending` / `huggingface_trending`."
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
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        sample_body(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let kind = opts.kind.unwrap_or_default();
        let limit = opts
            .limit
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(MIN_LIMIT, MAX_LIMIT) as usize;
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);

        let rows = fetch_rows(kind, limit).await?;
        Ok(payload(render_body(&rows, kind, shape).await))
    }
}

async fn fetch_rows(kind: Kind, limit: usize) -> Result<Vec<TopRow>, FetchError> {
    let limit_str = limit.to_string();
    let params = [("limit", limit_str.as_str())];
    match kind {
        Kind::Artists => {
            let raw: ChartArtistsResponse = client::get_json(kind.method(), &params).await?;
            Ok(raw
                .artists
                .artist
                .into_iter()
                .enumerate()
                .map(|(i, a)| artist_row(a, i))
                .collect())
        }
        Kind::Tracks => {
            let raw: ChartTracksResponse = client::get_json(kind.method(), &params).await?;
            Ok(raw
                .tracks
                .track
                .into_iter()
                .enumerate()
                .map(|(i, t)| track_row(t, i))
                .collect())
        }
    }
}

fn artist_row(raw: RawChartArtist, index: usize) -> TopRow {
    TopRow {
        rank: index + 1,
        primary: raw.name,
        secondary: None,
        playcount: parse_count(&raw.playcount),
        url: raw.url,
        image_url: best_image(&raw.image),
    }
}

fn track_row(raw: RawChartTrack, index: usize) -> TopRow {
    TopRow {
        rank: index + 1,
        primary: raw.name,
        secondary: raw.artist.name.filter(|s| !s.is_empty()),
        playcount: parse_count(&raw.playcount),
        url: raw.url,
        image_url: best_image(&raw.image),
    }
}

async fn render_body(rows: &[TopRow], kind: Kind, shape: Shape) -> Body {
    let empty = kind.empty_label();
    match shape {
        Shape::ImageLinkedList => image_linked_body(rows).await,
        Shape::LinkedTextBlock => linked_text_body(rows, empty),
        Shape::TextBlock => text_block_body(rows, empty),
        Shape::MarkdownTextBlock => markdown_body(rows, empty),
        Shape::Entries => entries_body(rows),
        Shape::Bars => bars_body(rows),
        _ => text_body(rows, empty),
    }
}

fn sample_body(shape: Shape) -> Option<Body> {
    use crate::samples;
    Some(match shape {
        Shape::Text => samples::text(&headline(&sample_rows(), "no chart artists")),
        Shape::TextBlock => samples::text_block(&[
            "#1 Taylor Swift  12.4M plays",
            "#2 The Weeknd  8.2M plays",
            "#3 Drake  7.1M plays",
        ]),
        Shape::MarkdownTextBlock => samples::markdown(
            "- **#1 Taylor Swift** — 12.4M plays\n- **#2 The Weeknd** — 8.2M plays\n- **#3 Drake** — 7.1M plays",
        ),
        Shape::LinkedTextBlock => samples::linked_text_block(&[
            (
                "#1 Taylor Swift  12.4M plays",
                Some("https://www.last.fm/music/Taylor+Swift"),
            ),
            (
                "#2 The Weeknd  8.2M plays",
                Some("https://www.last.fm/music/The+Weeknd"),
            ),
        ]),
        Shape::ImageLinkedList => samples::image_linked_list(&[
            (
                "#1 Taylor Swift",
                Some("https://www.last.fm/music/Taylor+Swift"),
                None,
                Some("12.4M plays"),
            ),
            (
                "#2 The Weeknd",
                Some("https://www.last.fm/music/The+Weeknd"),
                None,
                Some("8.2M plays"),
            ),
        ]),
        Shape::Entries => samples::entries(&[
            ("#1 Taylor Swift", "12.4M plays"),
            ("#2 The Weeknd", "8.2M plays"),
        ]),
        Shape::Bars => samples::bars(&[
            ("Taylor Swift", 12_400_000),
            ("The Weeknd", 8_200_000),
            ("Drake", 7_100_000),
        ]),
        _ => return None,
    })
}

fn sample_rows() -> Vec<TopRow> {
    vec![
        TopRow {
            rank: 1,
            primary: "Taylor Swift".into(),
            secondary: None,
            playcount: 12_400_000,
            url: "https://www.last.fm/music/Taylor+Swift".into(),
            image_url: None,
        },
        TopRow {
            rank: 2,
            primary: "The Weeknd".into(),
            secondary: None,
            playcount: 8_200_000,
            url: "https://www.last.fm/music/The+Weeknd".into(),
            image_url: None,
        },
    ]
}

#[derive(Deserialize)]
struct ChartArtistsResponse {
    artists: ChartArtistsInner,
}

#[derive(Deserialize)]
struct ChartArtistsInner {
    #[serde(default)]
    artist: Vec<RawChartArtist>,
}

#[derive(Deserialize)]
struct RawChartArtist {
    name: String,
    #[serde(default)]
    playcount: String,
    url: String,
    #[serde(default)]
    image: Vec<ImageEntry>,
}

#[derive(Deserialize)]
struct ChartTracksResponse {
    tracks: ChartTracksInner,
}

#[derive(Deserialize)]
struct ChartTracksInner {
    #[serde(default)]
    track: Vec<RawChartTrack>,
}

#[derive(Deserialize)]
struct RawChartTrack {
    name: String,
    #[serde(default)]
    playcount: String,
    artist: ArtistObj,
    url: String,
    #[serde(default)]
    image: Vec<ImageEntry>,
}

#[derive(Deserialize)]
struct ArtistObj {
    #[serde(default)]
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn ctx(options: Option<&str>, shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "lastfm-charts".into(),
            timeout: Duration::from_secs(1),
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            ..Default::default()
        }
    }

    #[test]
    fn kind_default_is_artists() {
        assert_eq!(Kind::default(), Kind::Artists);
    }

    #[test]
    fn kind_methods_map_to_lastfm_endpoint_names() {
        assert_eq!(Kind::Artists.method(), "chart.getTopArtists");
        assert_eq!(Kind::Tracks.method(), "chart.getTopTracks");
    }

    #[test]
    fn kind_deserialises_from_lowercase_string() {
        let raw: toml::Value = toml::from_str("kind = \"tracks\"").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.kind, Some(Kind::Tracks));
    }

    #[test]
    fn options_default_to_none() {
        let opts = Options::default();
        assert!(opts.kind.is_none());
        assert!(opts.limit.is_none());
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("bogus = 1").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn artist_row_assigns_position_rank_and_no_secondary() {
        let json = r##"{
            "artists": {
                "artist": [
                    {
                        "name": "Taylor Swift",
                        "playcount": "12400000",
                        "url": "https://www.last.fm/music/Taylor+Swift",
                        "image": [{"#text": "https://example.com/m.png", "size": "mega"}]
                    }
                ]
            }
        }"##;
        let raw: ChartArtistsResponse = serde_json::from_str(json).unwrap();
        let rows: Vec<TopRow> = raw
            .artists
            .artist
            .into_iter()
            .enumerate()
            .map(|(i, a)| artist_row(a, i))
            .collect();
        assert_eq!(rows[0].rank, 1);
        assert_eq!(rows[0].primary, "Taylor Swift");
        assert!(rows[0].secondary.is_none());
        assert_eq!(rows[0].playcount, 12_400_000);
        assert_eq!(
            rows[0].image_url.as_deref(),
            Some("https://example.com/m.png")
        );
    }

    #[test]
    fn track_row_pulls_artist_into_secondary() {
        let json = r##"{
            "tracks": {
                "track": [
                    {
                        "name": "Anti-Hero",
                        "playcount": "5400000",
                        "artist": {"name": "Taylor Swift"},
                        "url": "https://www.last.fm/music/Taylor+Swift/_/Anti-Hero",
                        "image": []
                    }
                ]
            }
        }"##;
        let raw: ChartTracksResponse = serde_json::from_str(json).unwrap();
        let rows: Vec<TopRow> = raw
            .tracks
            .track
            .into_iter()
            .enumerate()
            .map(|(i, t)| track_row(t, i))
            .collect();
        assert_eq!(rows[0].rank, 1);
        assert_eq!(rows[0].primary, "Anti-Hero");
        assert_eq!(rows[0].secondary.as_deref(), Some("Taylor Swift"));
        assert_eq!(rows[0].playcount, 5_400_000);
    }

    #[test]
    fn empty_label_differs_per_kind() {
        assert_eq!(Kind::Artists.empty_label(), "no chart artists");
        assert_eq!(Kind::Tracks.empty_label(), "no chart tracks");
    }

    #[test]
    fn fetcher_exposes_catalog_metadata_and_samples() {
        let fetcher = LastfmCharts;
        assert_eq!(fetcher.name(), "lastfm_charts");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        for shape in SHAPES {
            assert!(
                fetcher.sample_body(*shape).is_some(),
                "missing sample for {shape:?}"
            );
        }
        // Charts deliberately drops Ratio / NumberSeries / Badge / Timeline — chart endpoints
        // don't carry user-binding semantics those shapes are meaningful for.
        for missing in [
            Shape::Ratio,
            Shape::NumberSeries,
            Shape::Badge,
            Shape::Timeline,
        ] {
            assert!(fetcher.sample_body(missing).is_none());
        }
    }

    #[test]
    fn cache_key_partitions_by_kind_and_shape() {
        let fetcher = LastfmCharts;
        let artists = fetcher.cache_key(&ctx(
            Some("kind = \"artists\""),
            Some(Shape::LinkedTextBlock),
        ));
        let tracks = fetcher.cache_key(&ctx(
            Some("kind = \"tracks\""),
            Some(Shape::LinkedTextBlock),
        ));
        let bars = fetcher.cache_key(&ctx(Some("kind = \"artists\""), Some(Shape::Bars)));
        assert_ne!(artists, tracks);
        assert_ne!(artists, bars);
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_options_before_network() {
        let err = LastfmCharts
            .fetch(&ctx(Some("bogus = true"), Some(Shape::Text)))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("unknown field"));
    }
}
