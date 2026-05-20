//! `lastfm_top_artists` — a Last.fm user's top artists for a rolling window.
//!
//! Reads `user.getTopArtists`. The `period` option (default `"7day"`) chooses the window
//! width — same set of values Last.fm exposes on the profile page tabs. Rendering goes
//! through the shared [`super::top`] surface so the sibling stays focused on parsing.

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
        description: "Last.fm username whose top artists to read.",
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
        description: "Number of artists to display.",
    },
];

const DEFAULT_LIMIT: u32 = 10;
const MIN_LIMIT: u32 = 1;
const MAX_LIMIT: u32 = 50;

pub struct LastfmTopArtists;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub user: Option<String>,
    pub period: Option<Period>,
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for LastfmTopArtists {
    fn name(&self) -> &str {
        "lastfm_top_artists"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Top artists for a Last.fm user over a rolling window (`7day` default, also `1month` / `3month` / `6month` / `12month` / `overall`). Mirrors the profile page's Top Artists tab."
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
                    "lastfm_top_artists: `user = \"<lastfm name>\"` is required".into(),
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
    let raw: TopArtistsResponse = client::get_json(
        "user.getTopArtists",
        &[
            ("user", user),
            ("period", period.as_param()),
            ("limit", &limit_str),
        ],
    )
    .await?;
    Ok(raw
        .topartists
        .artist
        .into_iter()
        .enumerate()
        .map(|(i, a)| into_row(a, i))
        .collect())
}

fn into_row(raw: RawArtist, index: usize) -> TopRow {
    let rank = raw
        .attr
        .as_ref()
        .and_then(|a| a.rank.parse::<usize>().ok())
        .unwrap_or(index + 1);
    TopRow {
        rank,
        primary: raw.name,
        secondary: None,
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
    format!("no top artists ({})", period.label())
}

fn sample_body(shape: Shape) -> Option<Body> {
    use crate::samples;
    let rows = sample_rows();
    Some(match shape {
        Shape::Text => samples::text(&headline(&rows, "no top artists")),
        Shape::TextBlock => samples::text_block(&[
            "#1 Coldplay  3.2k plays",
            "#2 Adam Levine  1.8k plays",
            "#3 Beach House  850 plays",
        ]),
        Shape::MarkdownTextBlock => samples::markdown(
            "- **#1 Coldplay** — 3.2k plays\n- **#2 Adam Levine** — 1.8k plays\n- **#3 Beach House** — 850 plays",
        ),
        Shape::LinkedTextBlock => samples::linked_text_block(&[
            (
                "#1 Coldplay  3.2k plays",
                Some("https://www.last.fm/music/Coldplay"),
            ),
            (
                "#2 Adam Levine  1.8k plays",
                Some("https://www.last.fm/music/Adam+Levine"),
            ),
        ]),
        Shape::ImageLinkedList => samples::image_linked_list(&[
            (
                "#1 Coldplay",
                Some("https://www.last.fm/music/Coldplay"),
                None,
                Some("3.2k plays"),
            ),
            (
                "#2 Adam Levine",
                Some("https://www.last.fm/music/Adam+Levine"),
                None,
                Some("1.8k plays"),
            ),
        ]),
        Shape::Entries => samples::entries(&[
            ("#1 Coldplay", "3.2k plays"),
            ("#2 Adam Levine", "1.8k plays"),
        ]),
        Shape::Ratio => samples::ratio(0.45, "Coldplay"),
        Shape::NumberSeries => {
            samples::number_series(&[3_200, 1_800, 850, 720, 480, 320, 210, 180, 150, 120])
        }
        Shape::Bars => samples::bars(&[
            ("Coldplay", 3_200),
            ("Adam Levine", 1_800),
            ("Beach House", 850),
        ]),
        Shape::Badge => samples::badge(Status::Ok, "#1 Coldplay"),
        _ => return None,
    })
}

fn sample_rows() -> Vec<TopRow> {
    vec![
        TopRow {
            rank: 1,
            primary: "Coldplay".into(),
            secondary: None,
            playcount: 3_200,
            url: "https://www.last.fm/music/Coldplay".into(),
            image_url: None,
        },
        TopRow {
            rank: 2,
            primary: "Adam Levine".into(),
            secondary: None,
            playcount: 1_800,
            url: "https://www.last.fm/music/Adam+Levine".into(),
            image_url: None,
        },
    ]
}

#[derive(Deserialize)]
struct TopArtistsResponse {
    topartists: TopArtistsInner,
}

#[derive(Deserialize)]
struct TopArtistsInner {
    #[serde(default)]
    artist: Vec<RawArtist>,
}

#[derive(Deserialize)]
struct RawArtist {
    name: String,
    #[serde(default)]
    playcount: String,
    url: String,
    #[serde(default)]
    image: Vec<ImageEntry>,
    #[serde(rename = "@attr", default)]
    attr: Option<RankAttr>,
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
            widget_id: "lastfm-top-artists".into(),
            timeout: Duration::from_secs(1),
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            ..Default::default()
        }
    }

    fn parse_artists(json: &str) -> Vec<TopRow> {
        let raw: TopArtistsResponse = serde_json::from_str(json).unwrap();
        raw.topartists
            .artist
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
            toml::from_str("user = \"rj\"\nperiod = \"3month\"\nlimit = 25").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.user.as_deref(), Some("rj"));
        assert_eq!(opts.period, Some(Period::ThreeMonth));
        assert_eq!(opts.limit, Some(25));
    }

    #[test]
    fn into_row_uses_attr_rank_when_present() {
        let json = r##"{
            "topartists": {
                "artist": [
                    {
                        "name": "Artist",
                        "playcount": "42",
                        "url": "https://www.last.fm/music/Artist",
                        "image": [
                            {"#text": "https://example.com/m.png", "size": "mega"}
                        ],
                        "@attr": {"rank": "1"}
                    }
                ]
            }
        }"##;
        let rows = parse_artists(json);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].rank, 1);
        assert_eq!(rows[0].primary, "Artist");
        assert!(rows[0].secondary.is_none());
        assert_eq!(rows[0].playcount, 42);
        assert_eq!(
            rows[0].image_url.as_deref(),
            Some("https://example.com/m.png")
        );
    }

    #[test]
    fn into_row_falls_back_to_position_when_rank_missing() {
        let json = r##"{
            "topartists": {
                "artist": [
                    {"name": "A", "playcount": "1", "url": "https://x", "image": []},
                    {"name": "B", "playcount": "2", "url": "https://x", "image": []}
                ]
            }
        }"##;
        let rows = parse_artists(json);
        assert_eq!(rows[0].rank, 1);
        assert_eq!(rows[1].rank, 2);
    }

    #[test]
    fn into_row_collapses_unparseable_playcount_to_zero() {
        let json = r##"{
            "topartists": {
                "artist": [
                    {"name": "A", "playcount": "n/a", "url": "https://x", "image": []}
                ]
            }
        }"##;
        let rows = parse_artists(json);
        assert_eq!(rows[0].playcount, 0);
    }

    #[test]
    fn empty_label_includes_period_text() {
        assert!(empty_label(Period::SevenDay).contains("7d"));
        assert!(empty_label(Period::Overall).contains("overall"));
    }

    #[test]
    fn fetcher_exposes_catalog_metadata_and_samples() {
        let fetcher = LastfmTopArtists;
        assert_eq!(fetcher.name(), "lastfm_top_artists");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        for shape in SHAPES {
            assert!(
                fetcher.sample_body(*shape).is_some(),
                "missing sample for {shape:?}"
            );
        }
        // Timeline isn't part of this fetcher's shape list — sample must be None.
        assert!(fetcher.sample_body(Shape::Timeline).is_none());
    }

    #[test]
    fn cache_key_partitions_by_period_and_shape() {
        let fetcher = LastfmTopArtists;
        let seven = fetcher.cache_key(&ctx(
            Some("user = \"rj\"\nperiod = \"7day\""),
            Some(Shape::LinkedTextBlock),
        ));
        let month = fetcher.cache_key(&ctx(
            Some("user = \"rj\"\nperiod = \"1month\""),
            Some(Shape::LinkedTextBlock),
        ));
        let bars = fetcher.cache_key(&ctx(
            Some("user = \"rj\"\nperiod = \"7day\""),
            Some(Shape::Bars),
        ));
        assert_ne!(seven, month);
        assert_ne!(seven, bars);
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
        let err = LastfmTopArtists
            .fetch(&ctx(None, Some(Shape::Text)))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("`user ="));
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_options_before_network() {
        let err = LastfmTopArtists
            .fetch(&ctx(Some("user = \"rj\"\nbogus = true"), Some(Shape::Text)))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("unknown field"));
    }
}
