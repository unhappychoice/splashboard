//! `lastfm_top_albums` — a Last.fm user's top albums for a rolling window.
//!
//! Reads `user.getTopAlbums`. Same `period` / `limit` contract as the other `top_*` siblings;
//! rows carry the artist as the secondary label so the shared rendering surface produces
//! `"Album — Artist"` titles automatically.

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::lastfm::client;
use crate::fetcher::lastfm::common::{ImageEntry, best_image, parse_count};
use crate::fetcher::lastfm::top::{
    Period, TopRow, badge_body, bars_body, entries_body, headline, image_linked_body,
    linked_text_body, markdown_body, number_series_body, ratio_body, text_block_body, text_body,
};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload, Status};
use crate::render::Shape;

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::ImageLinkedList,
    Shape::TextBlock,
    Shape::Text,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Ratio,
    Shape::NumberSeries,
    Shape::Bars,
    Shape::Badge,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "user",
        type_hint: "string",
        required: true,
        default: None,
        description: "Last.fm username whose top albums to read.",
    },
    OptionSchema {
        name: "period",
        type_hint: "\"7day\" | \"1month\" | \"3month\" | \"6month\" | \"12month\" | \"overall\"",
        required: false,
        default: Some("\"7day\""),
        description: "Rolling window for the ranking — same set Last.fm exposes on the profile page tabs.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=50)",
        required: false,
        default: Some("10"),
        description: "Number of albums to display.",
    },
];

const DEFAULT_LIMIT: u32 = 10;
const MIN_LIMIT: u32 = 1;
const MAX_LIMIT: u32 = 50;

pub struct LastfmTopAlbums;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub user: Option<String>,
    pub period: Option<Period>,
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for LastfmTopAlbums {
    fn name(&self) -> &str {
        "lastfm_top_albums"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Top albums for a Last.fm user over a rolling window (`7day` default, also `1month` / `3month` / `6month` / `12month` / `overall`). Mirrors the profile page's Top Albums tab. ImageLinkedList shape carries the album artwork from Last.fm's image set."
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
                    "lastfm_top_albums: `user = \"<lastfm name>\"` is required".into(),
                )
            })?;
        let period = opts.period.unwrap_or_default();
        let limit = opts
            .limit
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(MIN_LIMIT, MAX_LIMIT) as usize;
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);

        let rows = fetch_rows(user, period, limit).await?;
        Ok(payload(render_body(&rows, period, shape).await))
    }
}

async fn fetch_rows(user: &str, period: Period, limit: usize) -> Result<Vec<TopRow>, FetchError> {
    let limit_str = limit.to_string();
    let raw: TopAlbumsResponse = client::get_json(
        "user.getTopAlbums",
        &[
            ("user", user),
            ("period", period.as_param()),
            ("limit", &limit_str),
        ],
    )
    .await?;
    Ok(raw
        .topalbums
        .album
        .into_iter()
        .enumerate()
        .map(|(i, a)| into_row(a, i))
        .collect())
}

fn into_row(raw: RawAlbum, index: usize) -> TopRow {
    let rank = raw
        .attr
        .as_ref()
        .and_then(|a| a.rank.parse::<usize>().ok())
        .unwrap_or(index + 1);
    let artist = raw.artist.name.filter(|s| !s.is_empty());
    TopRow {
        rank,
        primary: raw.name,
        secondary: artist,
        playcount: parse_count(&raw.playcount),
        url: raw.url,
        image_url: best_image(&raw.image),
    }
}

async fn render_body(rows: &[TopRow], period: Period, shape: Shape) -> Body {
    let empty = empty_label(period);
    match shape {
        Shape::ImageLinkedList => image_linked_body(rows).await,
        Shape::LinkedTextBlock => linked_text_body(rows, &empty),
        Shape::TextBlock => text_block_body(rows, &empty),
        Shape::MarkdownTextBlock => markdown_body(rows, &empty),
        Shape::Entries => entries_body(rows),
        Shape::Ratio => ratio_body(rows),
        Shape::NumberSeries => number_series_body(rows),
        Shape::Bars => bars_body(rows),
        Shape::Badge => badge_body(rows, period),
        _ => text_body(rows, &empty),
    }
}

fn empty_label(period: Period) -> String {
    format!("no top albums ({})", period.label())
}

fn sample_body(shape: Shape) -> Option<Body> {
    use crate::samples;
    let rows = sample_rows();
    Some(match shape {
        Shape::Text => samples::text(&headline(&rows, "no top albums")),
        Shape::TextBlock => samples::text_block(&[
            "#1 Viva la Vida — Coldplay  240 plays",
            "#2 Bloom — Beach House  168 plays",
            "#3 Songs About Jane — Maroon 5  142 plays",
        ]),
        Shape::MarkdownTextBlock => samples::markdown(
            "- **#1 Viva la Vida — Coldplay** — 240 plays\n- **#2 Bloom — Beach House** — 168 plays\n- **#3 Songs About Jane — Maroon 5** — 142 plays",
        ),
        Shape::LinkedTextBlock => samples::linked_text_block(&[
            (
                "#1 Viva la Vida — Coldplay  240 plays",
                Some("https://www.last.fm/music/Coldplay/Viva+la+Vida"),
            ),
            (
                "#2 Bloom — Beach House  168 plays",
                Some("https://www.last.fm/music/Beach+House/Bloom"),
            ),
        ]),
        Shape::ImageLinkedList => samples::image_linked_list(&[
            (
                "#1 Viva la Vida",
                Some("https://www.last.fm/music/Coldplay/Viva+la+Vida"),
                None,
                Some("Coldplay  ·  240 plays"),
            ),
            (
                "#2 Bloom",
                Some("https://www.last.fm/music/Beach+House/Bloom"),
                None,
                Some("Beach House  ·  168 plays"),
            ),
        ]),
        Shape::Entries => samples::entries(&[
            ("#1 Viva la Vida — Coldplay", "240 plays"),
            ("#2 Bloom — Beach House", "168 plays"),
        ]),
        Shape::Ratio => samples::ratio(0.42, "Viva la Vida — Coldplay"),
        Shape::NumberSeries => samples::number_series(&[240, 168, 142, 98, 72, 54, 41, 36, 28, 22]),
        Shape::Bars => samples::bars(&[
            ("Viva la Vida — Coldplay", 240),
            ("Bloom — Beach House", 168),
            ("Songs About Jane — Maroon 5", 142),
        ]),
        Shape::Badge => samples::badge(Status::Ok, "#1 Viva la Vida"),
        _ => return None,
    })
}

fn sample_rows() -> Vec<TopRow> {
    vec![
        TopRow {
            rank: 1,
            primary: "Viva la Vida".into(),
            secondary: Some("Coldplay".into()),
            playcount: 240,
            url: "https://www.last.fm/music/Coldplay/Viva+la+Vida".into(),
            image_url: None,
        },
        TopRow {
            rank: 2,
            primary: "Bloom".into(),
            secondary: Some("Beach House".into()),
            playcount: 168,
            url: "https://www.last.fm/music/Beach+House/Bloom".into(),
            image_url: None,
        },
    ]
}

#[derive(Deserialize)]
struct TopAlbumsResponse {
    topalbums: TopAlbumsInner,
}

#[derive(Deserialize)]
struct TopAlbumsInner {
    #[serde(default)]
    album: Vec<RawAlbum>,
}

#[derive(Deserialize)]
struct RawAlbum {
    name: String,
    #[serde(default)]
    playcount: String,
    artist: ArtistObj,
    url: String,
    #[serde(default)]
    image: Vec<ImageEntry>,
    #[serde(rename = "@attr", default)]
    attr: Option<RankAttr>,
}

#[derive(Deserialize)]
struct ArtistObj {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct RankAttr {
    #[serde(default)]
    rank: String,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn ctx(options: Option<&str>, shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "lastfm-top-albums".into(),
            timeout: Duration::from_secs(1),
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            ..Default::default()
        }
    }

    fn parse_albums(json: &str) -> Vec<TopRow> {
        let raw: TopAlbumsResponse = serde_json::from_str(json).unwrap();
        raw.topalbums
            .album
            .into_iter()
            .enumerate()
            .map(|(i, a)| into_row(a, i))
            .collect()
    }

    #[test]
    fn options_default_to_none() {
        let opts = Options::default();
        assert!(opts.user.is_none());
        assert!(opts.period.is_none());
        assert!(opts.limit.is_none());
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("bogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn options_deserialise_full_form() {
        let raw: toml::Value =
            toml::from_str("user = \"rj\"\nperiod = \"6month\"\nlimit = 10").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.user.as_deref(), Some("rj"));
        assert_eq!(opts.period, Some(Period::SixMonth));
        assert_eq!(opts.limit, Some(10));
    }

    #[test]
    fn into_row_pulls_artist_into_secondary() {
        let json = r##"{
            "topalbums": {
                "album": [
                    {
                        "name": "Album",
                        "playcount": "60",
                        "artist": {"name": "Artist"},
                        "url": "https://www.last.fm/music/Artist/Album",
                        "image": [{"#text": "https://example.com/m.png", "size": "mega"}],
                        "@attr": {"rank": "1"}
                    }
                ]
            }
        }"##;
        let rows = parse_albums(json);
        assert_eq!(rows[0].rank, 1);
        assert_eq!(rows[0].primary, "Album");
        assert_eq!(rows[0].secondary.as_deref(), Some("Artist"));
        assert_eq!(rows[0].playcount, 60);
        assert_eq!(
            rows[0].image_url.as_deref(),
            Some("https://example.com/m.png")
        );
    }

    #[test]
    fn into_row_falls_back_to_position_when_rank_missing() {
        let json = r##"{
            "topalbums": {
                "album": [
                    {"name": "A", "playcount": "1", "artist": {"name": "X"}, "url": "https://x", "image": []},
                    {"name": "B", "playcount": "2", "artist": {"name": "Y"}, "url": "https://y", "image": []}
                ]
            }
        }"##;
        let rows = parse_albums(json);
        assert_eq!(rows[0].rank, 1);
        assert_eq!(rows[1].rank, 2);
    }

    #[test]
    fn empty_label_includes_period_text() {
        assert!(empty_label(Period::Overall).contains("overall"));
        assert!(empty_label(Period::TwelveMonth).contains("12m"));
    }

    #[test]
    fn fetcher_exposes_catalog_metadata_and_samples() {
        let fetcher = LastfmTopAlbums;
        assert_eq!(fetcher.name(), "lastfm_top_albums");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        for shape in SHAPES {
            assert!(
                fetcher.sample_body(*shape).is_some(),
                "missing sample for {shape:?}"
            );
        }
        assert!(fetcher.sample_body(Shape::Timeline).is_none());
    }

    #[test]
    fn cache_key_partitions_by_period_and_shape() {
        let fetcher = LastfmTopAlbums;
        let seven = fetcher.cache_key(&ctx(
            Some("user = \"rj\"\nperiod = \"7day\""),
            Some(Shape::LinkedTextBlock),
        ));
        let overall = fetcher.cache_key(&ctx(
            Some("user = \"rj\"\nperiod = \"overall\""),
            Some(Shape::LinkedTextBlock),
        ));
        assert_ne!(seven, overall);
    }

    /// `render_body` is the shape dispatcher between `fetch` and the shared `top` body builders.
    /// Drive every declared shape plus a shape outside `SHAPES` so the `_ => text_body` fallback
    /// arm is exercised. `sample_rows()` carries no `image_url`, so `ImageLinkedList` resolves its
    /// thumbnails to `None` without any network I/O.
    #[tokio::test]
    async fn render_body_dispatches_every_shape() {
        let rows = sample_rows();
        let p = Period::SevenDay;
        assert!(matches!(
            render_body(&rows, p, Shape::LinkedTextBlock).await,
            Body::LinkedTextBlock(_)
        ));
        assert!(matches!(
            render_body(&rows, p, Shape::ImageLinkedList).await,
            Body::ImageLinkedList(_)
        ));
        assert!(matches!(
            render_body(&rows, p, Shape::TextBlock).await,
            Body::TextBlock(_)
        ));
        assert!(matches!(
            render_body(&rows, p, Shape::MarkdownTextBlock).await,
            Body::MarkdownTextBlock(_)
        ));
        assert!(matches!(
            render_body(&rows, p, Shape::Entries).await,
            Body::Entries(_)
        ));
        assert!(matches!(
            render_body(&rows, p, Shape::Ratio).await,
            Body::Ratio(_)
        ));
        assert!(matches!(
            render_body(&rows, p, Shape::NumberSeries).await,
            Body::NumberSeries(_)
        ));
        assert!(matches!(
            render_body(&rows, p, Shape::Bars).await,
            Body::Bars(_)
        ));
        assert!(matches!(
            render_body(&rows, p, Shape::Badge).await,
            Body::Badge(_)
        ));
        // `Shape::Text` is not an explicit arm — it lands in the `_ => text_body` fallback.
        assert!(matches!(
            render_body(&rows, p, Shape::Text).await,
            Body::Text(_)
        ));
    }

    #[tokio::test]
    async fn fetch_requires_user_option() {
        let err = LastfmTopAlbums
            .fetch(&ctx(None, Some(Shape::Text)))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("`user ="));
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_options_before_network() {
        let err = LastfmTopAlbums
            .fetch(&ctx(Some("user = \"rj\"\nbogus = 1"), Some(Shape::Text)))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("unknown field"));
    }
}
