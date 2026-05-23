//! `deal_steam_daily` — Steam's official daily / weekend / midweek deal RSS feed.
//!
//! Safety::Safe — host hardcoded at `store.steampowered.com`. The discount percent is parsed
//! out of the entry title (`"Daily Deal - The Beast Inside, 25% Off"`); titles that don't
//! match the canonical pattern still render but without a `discount_pct` populated.

use std::sync::OnceLock;

use async_trait::async_trait;
use feed_rs::model::Entry;
use regex::Regex;
use serde::Deserialize;
use url::Url;

use super::common::{self, DealRow, MAX_ROWS};
use crate::fetcher::feed;
use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;
use crate::samples;

const FEED_URL: &str = "https://store.steampowered.com/feeds/daily_deals.xml";
const NAME: &str = "deal_steam_daily";
const STORE: &str = "Steam";

const DEFAULT_COUNT: u32 = 10;
const MIN_COUNT: u32 = 1;

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Text,
    Shape::Entries,
    Shape::Bars,
    Shape::ImageLinkedList,
    Shape::Badge,
    Shape::Timeline,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "count",
    type_hint: "integer (1..=20)",
    required: false,
    default: Some("10"),
    description: "Maximum number of deals to display.",
}];

pub struct SteamDailyFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    count: Option<u32>,
}

#[async_trait]
impl Fetcher for SteamDailyFetcher {
    fn name(&self) -> &str {
        NAME
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Steam's official daily deals RSS feed — currently-running individual game discounts curated by Valve. Each entry parses into `[Steam] <Game>  <N>% off`. Complements `deal_games` (CheapShark) which spans every store; this one mirrors Steam's storefront priority order."
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
        cache_key(NAME, ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        sample_body_for(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let count = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_ROWS as u32) as usize;
        let url = Url::parse(FEED_URL)
            .map_err(|e| FetchError::Failed(format!("deal_steam_daily: bad feed url: {e}")))?;
        let bytes = feed::fetch_bytes(&url, NAME).await?;
        let parsed = feed::parse_feed(&bytes, NAME)?;
        let rows: Vec<DealRow> = parsed
            .entries
            .iter()
            .take(count)
            .map(entry_to_row)
            .collect();
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let body = match shape {
            Shape::ImageLinkedList => common::image_linked_body(&rows).await,
            other => common::body_for_shape(&rows, other)
                .unwrap_or_else(|| common::linked_text_block_body(&rows)),
        };
        Ok(payload(body))
    }
}

fn entry_to_row(entry: &Entry) -> DealRow {
    let raw_title = entry
        .title
        .as_ref()
        .map(|t| feed::collapse_whitespace(&t.content))
        .unwrap_or_default();
    let (title, discount_pct) = parse_title(&raw_title);
    let link =
        feed::link_for(entry).unwrap_or_else(|| String::from("https://store.steampowered.com"));
    DealRow {
        title: if title.is_empty() {
            "(unnamed deal)".into()
        } else {
            title
        },
        image_url: feed::thumbnail_url_for(entry),
        sale_price: None,
        original_price: None,
        discount_pct,
        store: Some(STORE.into()),
        link,
        published: entry.published.or(entry.updated),
    }
}

/// Steam titles use a handful of marketing prefixes (`Daily Deal`, `Weekend Deal`,
/// `Midweek Madness`, `Weeklong Deal`, `Free Weekend`, `Special Promotion`, `Publisher Sale`).
/// We strip whichever prefix matches and pull the trailing `, X% Off` clause when present.
fn parse_title(raw: &str) -> (String, Option<u32>) {
    static TITLE_RE: OnceLock<Regex> = OnceLock::new();
    let re = TITLE_RE.get_or_init(|| {
        Regex::new(
            r"^(?P<prefix>[A-Za-z][A-Za-z ]+?)\s*-\s*(?P<title>.+?)(?:,\s*(?P<pct>\d{1,3})% Off!?)?$",
        )
        .unwrap()
    });
    match re.captures(raw) {
        Some(c) => {
            let title = c["title"].trim().to_string();
            let pct = c.name("pct").and_then(|m| m.as_str().parse::<u32>().ok());
            (title, pct.map(|p| p.min(100)))
        }
        None => (raw.to_string(), None),
    }
}

fn sample_body_for(shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::LinkedTextBlock => samples::linked_text_block(&[
            (
                "[Steam] Portal 2  75% off",
                Some("https://store.steampowered.com/app/620/Portal_2/"),
            ),
            (
                "[Steam] Hades  50% off",
                Some("https://store.steampowered.com/app/1145360/Hades/"),
            ),
            (
                "[Steam] Stardew Valley  40% off",
                Some("https://store.steampowered.com/app/413150/Stardew_Valley/"),
            ),
        ]),
        Shape::TextBlock => samples::text_block(&[
            "[Steam] Portal 2  75% off",
            "[Steam] Hades  50% off",
            "[Steam] Stardew Valley  40% off",
        ]),
        Shape::MarkdownTextBlock => samples::markdown(
            "- [[Steam] Portal 2  75% off](https://store.steampowered.com/app/620/Portal_2/)\n- [[Steam] Hades  50% off](https://store.steampowered.com/app/1145360/Hades/)",
        ),
        Shape::Text => samples::text("[Steam] Portal 2  75% off"),
        Shape::Entries => samples::entries(&[
            ("Portal 2", "75% off"),
            ("Hades", "50% off"),
            ("Stardew Valley", "40% off"),
        ]),
        Shape::Bars => samples::bars(&[("Portal 2", 75), ("Hades", 50), ("Stardew Valley", 40)]),
        Shape::Badge => samples::badge(crate::payload::Status::Ok, "75% off"),
        Shape::Timeline => samples::timeline(&[
            (1_745_625_600, "Portal 2", Some("Steam · 75% off")),
            (1_745_539_200, "Hades", Some("Steam · 50% off")),
        ]),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use feed_rs::model::{Link, Text};

    fn text(content: &str) -> Text {
        Text {
            content_type: "text/plain".parse().unwrap(),
            src: None,
            content: content.into(),
        }
    }

    fn link(href: &str) -> Link {
        Link {
            href: href.into(),
            rel: None,
            media_type: None,
            href_lang: None,
            title: None,
            length: None,
        }
    }

    #[test]
    fn parse_title_extracts_daily_deal_discount() {
        let (title, pct) = parse_title("Daily Deal - The Beast Inside, 25% Off");
        assert_eq!(title, "The Beast Inside");
        assert_eq!(pct, Some(25));
    }

    #[test]
    fn parse_title_handles_weekend_deal_with_exclamation() {
        let (title, pct) = parse_title("Weekend Deal - Portal 2, 75% Off!");
        assert_eq!(title, "Portal 2");
        assert_eq!(pct, Some(75));
    }

    #[test]
    fn parse_title_handles_midweek_madness_three_word_prefix() {
        let (title, pct) = parse_title("Midweek Madness - Game Of The Year, 50% Off");
        assert_eq!(title, "Game Of The Year");
        assert_eq!(pct, Some(50));
    }

    #[test]
    fn parse_title_returns_none_pct_when_clause_absent() {
        let (title, pct) = parse_title("Free Weekend - Some Multiplayer Game");
        assert_eq!(title, "Some Multiplayer Game");
        assert!(pct.is_none());
    }

    #[test]
    fn parse_title_clamps_implausible_percentages() {
        let (_title, pct) = parse_title("Daily Deal - Bug, 250% Off");
        assert_eq!(pct, Some(100));
    }

    #[test]
    fn parse_title_falls_back_to_raw_when_pattern_misses() {
        let (title, pct) = parse_title("just a random string");
        assert_eq!(title, "just a random string");
        assert!(pct.is_none());
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("count = 5\nbogus = 1").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn fetcher_exposes_safety_safe_and_steam_store_attribution() {
        let f = SteamDailyFetcher;
        assert_eq!(f.name(), NAME);
        assert_eq!(f.safety(), Safety::Safe);
        assert_eq!(f.default_shape(), Shape::LinkedTextBlock);
    }

    #[test]
    fn sample_body_covers_every_supported_shape() {
        let f = SteamDailyFetcher;
        for shape in [
            Shape::LinkedTextBlock,
            Shape::TextBlock,
            Shape::MarkdownTextBlock,
            Shape::Text,
            Shape::Entries,
            Shape::Bars,
            Shape::Badge,
            Shape::Timeline,
        ] {
            assert!(
                f.sample_body(shape).is_some(),
                "sample missing for {shape:?}"
            );
        }
        assert!(f.sample_body(Shape::ImageLinkedList).is_none());
        assert!(f.sample_body(Shape::Heatmap).is_none());
    }

    #[test]
    fn entry_to_row_parses_steam_title_link_and_store() {
        let entry = Entry {
            title: Some(text("Daily Deal - Portal 2, 75% Off")),
            links: vec![link("https://store.steampowered.com/app/620/Portal_2/")],
            published: Some(Utc.with_ymd_and_hms(2026, 4, 26, 12, 0, 0).unwrap()),
            ..Default::default()
        };
        let row = entry_to_row(&entry);
        assert_eq!(row.title, "Portal 2");
        assert_eq!(row.discount_pct, Some(75));
        assert_eq!(row.store.as_deref(), Some(STORE));
        assert_eq!(row.link, "https://store.steampowered.com/app/620/Portal_2/");
        assert_eq!(row.published, entry.published);
        assert!(row.image_url.is_none());
        assert!(row.sale_price.is_none());
        assert!(row.original_price.is_none());
    }

    #[test]
    fn entry_to_row_names_titleless_entries_unnamed_deal() {
        let row = entry_to_row(&Entry::default());
        assert_eq!(row.title, "(unnamed deal)");
        assert!(row.discount_pct.is_none());
    }

    #[test]
    fn entry_to_row_falls_back_to_steam_store_url_when_link_missing() {
        let entry = Entry {
            title: Some(text("Weekend Deal - Hades, 50% Off")),
            ..Default::default()
        };
        let row = entry_to_row(&entry);
        assert_eq!(row.link, "https://store.steampowered.com");
        assert_eq!(row.title, "Hades");
        assert_eq!(row.discount_pct, Some(50));
    }

    #[test]
    fn entry_to_row_uses_updated_timestamp_when_published_absent() {
        let updated = Utc.with_ymd_and_hms(2026, 5, 1, 9, 0, 0).unwrap();
        let entry = Entry {
            title: Some(text("Midweek Madness - Celeste, 60% Off")),
            updated: Some(updated),
            ..Default::default()
        };
        let row = entry_to_row(&entry);
        assert_eq!(row.published, Some(updated));
        assert_eq!(row.title, "Celeste");
    }

    #[test]
    fn cache_key_is_name_prefixed_and_varies_with_options() {
        let f = SteamDailyFetcher;
        let bare = f.cache_key(&FetchContext::default());
        assert!(bare.starts_with(NAME), "got: {bare}");
        let with_opts = f.cache_key(&FetchContext {
            options: Some(toml::from_str("count = 5").unwrap()),
            ..Default::default()
        });
        assert!(with_opts.starts_with(NAME), "got: {with_opts}");
        assert_ne!(bare, with_opts);
    }
}
