//! `deal_free_games` — free games / loot offers across 8 storefronts via the LootScraper
//! aggregator (`feed.eikowagenknecht.com`).
//!
//! Safety::Safe — host is hardcoded and `platform` / `kind` pick among a closed enum, so config
//! cannot redirect the request off-host. LootScraper is third-party but the public instance has
//! shipped continuously since 2019 and absorbs the per-platform scraping complexity (Epic
//! GraphQL, Amazon Luna OAuth, Steam giveaway events, GOG/Humble/itch.io homepage scrapes)
//! that would otherwise sprawl into eight separate fetchers here.

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

const FEED_BASE: &str = "https://feed.eikowagenknecht.com";
const NAME: &str = "deal_free_games";

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

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "platform",
        type_hint: "\"all\" | \"epic\" | \"steam\" | \"amazon\" | \"gog\" | \"humble\" | \"itch\" | \"apple\" | \"google\"",
        required: false,
        default: Some("\"all\""),
        description: "Storefront to subscribe to. `all` merges every platform via LootScraper's aggregate feed.",
    },
    OptionSchema {
        name: "kind",
        type_hint: "\"game\" | \"loot\"",
        required: false,
        default: Some("\"game\""),
        description: "`game` = full game giveaways; `loot` = in-game items / Twitch drops. Only `amazon` and `steam` expose a `loot` feed; other platforms reject it.",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=20)",
        required: false,
        default: Some("10"),
        description: "Maximum number of offers to display.",
    },
];

pub struct FreeGamesFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    count: Option<u32>,
}

#[async_trait]
impl Fetcher for FreeGamesFetcher {
    fn name(&self) -> &str {
        NAME
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Free games and loot offers aggregated across Epic Games, Steam, Amazon Prime Gaming, GOG, Humble, itch.io, Apple App Store, and Google Play via the LootScraper feed. `platform = \"all\"` (default) merges every source; pick a specific platform to filter. `kind = \"loot\"` switches to the Twitch-drops / in-game-item variant (Amazon and Steam only)."
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
        let platform = parse_platform(opts.platform.as_deref())?;
        let kind = parse_kind(opts.kind.as_deref())?;
        if kind == Kind::Loot && !platform.supports_loot() {
            return Err(FetchError::Failed(format!(
                "deal_free_games: `kind = \"loot\"` is only supported on `amazon` and `steam` (got `{}`)",
                platform.config_value()
            )));
        }
        let url = Url::parse(&platform.feed_url(kind))
            .map_err(|e| FetchError::Failed(format!("deal_free_games: bad feed url: {e}")))?;
        let count = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_ROWS as u32) as usize;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platform {
    All,
    Epic,
    Steam,
    Amazon,
    Gog,
    Humble,
    Itch,
    Apple,
    Google,
}

impl Platform {
    fn config_value(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Epic => "epic",
            Self::Steam => "steam",
            Self::Amazon => "amazon",
            Self::Gog => "gog",
            Self::Humble => "humble",
            Self::Itch => "itch",
            Self::Apple => "apple",
            Self::Google => "google",
        }
    }

    fn url_slug(self) -> &'static str {
        // LootScraper's per-platform feeds use slightly different short codes from our config
        // value. Map explicitly so a config refactor can't silently break URL construction.
        match self {
            Self::All => "",
            Self::Epic => "epic",
            Self::Steam => "steam",
            Self::Amazon => "amazon",
            Self::Gog => "gog",
            Self::Humble => "humble",
            Self::Itch => "itch",
            Self::Apple => "apple",
            Self::Google => "google",
        }
    }

    fn supports_loot(self) -> bool {
        matches!(self, Self::Amazon | Self::Steam)
    }

    fn feed_url(self, kind: Kind) -> String {
        if matches!(self, Self::All) {
            format!("{FEED_BASE}/lootscraper.xml")
        } else {
            format!(
                "{FEED_BASE}/lootscraper_{slug}_{kind}.xml",
                slug = self.url_slug(),
                kind = kind.url_slug(),
            )
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Game,
    Loot,
}

impl Kind {
    fn url_slug(self) -> &'static str {
        match self {
            Self::Game => "game",
            Self::Loot => "loot",
        }
    }
}

fn parse_platform(raw: Option<&str>) -> Result<Platform, FetchError> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(Platform::All),
        Some(p) => match p.to_lowercase().as_str() {
            "all" => Ok(Platform::All),
            "epic" => Ok(Platform::Epic),
            "steam" => Ok(Platform::Steam),
            "amazon" => Ok(Platform::Amazon),
            "gog" => Ok(Platform::Gog),
            "humble" => Ok(Platform::Humble),
            "itch" => Ok(Platform::Itch),
            "apple" => Ok(Platform::Apple),
            "google" => Ok(Platform::Google),
            other => Err(FetchError::Failed(format!(
                "deal_free_games: unknown platform `{other}` (expected all/epic/steam/amazon/gog/humble/itch/apple/google)"
            ))),
        },
    }
}

fn parse_kind(raw: Option<&str>) -> Result<Kind, FetchError> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(Kind::Game),
        Some(k) => match k.to_lowercase().as_str() {
            "game" => Ok(Kind::Game),
            "loot" => Ok(Kind::Loot),
            other => Err(FetchError::Failed(format!(
                "deal_free_games: unknown kind `{other}` (expected game/loot)"
            ))),
        },
    }
}

/// LootScraper entry titles follow `"<Store> (<Kind>) - <Game>"` (e.g.
/// `"Amazon Prime (Game) - Captain Blood"`). The store / game split survives even on the
/// aggregate feed because every entry carries it. When the pattern doesn't match (older
/// item, future variation) we fall back to the raw title with no store.
fn entry_to_row(entry: &Entry) -> DealRow {
    let raw_title = entry
        .title
        .as_ref()
        .map(|t| feed::collapse_whitespace(&t.content))
        .unwrap_or_default();
    let (store, title) = split_store_and_title(&raw_title);
    let link = feed::link_for(entry).unwrap_or_else(|| String::from("https://example.com/"));
    DealRow {
        title: if title.is_empty() {
            "(unnamed offer)".into()
        } else {
            title
        },
        image_url: feed::thumbnail_url_for(entry),
        sale_price: Some("Free".into()),
        original_price: None,
        discount_pct: Some(100),
        store,
        link,
        published: entry.published.or(entry.updated),
    }
}

fn split_store_and_title(raw: &str) -> (Option<String>, String) {
    static SPLIT_RE: OnceLock<Regex> = OnceLock::new();
    let re = SPLIT_RE.get_or_init(|| {
        Regex::new(r"^(?P<store>[^(]+?)\s*\((?:Game|Loot)\)\s*-\s*(?P<title>.+)$").unwrap()
    });
    match re.captures(raw) {
        Some(c) => (
            Some(c["store"].trim().to_string()),
            c["title"].trim().to_string(),
        ),
        None => (None, raw.to_string()),
    }
}

fn sample_body_for(shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::LinkedTextBlock => samples::linked_text_block(&[
            (
                "[Epic Games] Subnautica  Free (100% off)",
                Some("https://store.epicgames.com/p/subnautica"),
            ),
            (
                "[Amazon Prime] Star Wars Battlefront II  Free (100% off)",
                Some("https://gaming.amazon.com/loot/star-wars-battlefront-2"),
            ),
            (
                "[GOG] Galactic Civilizations III  Free (100% off)",
                Some("https://www.gog.com/giveaway"),
            ),
        ]),
        Shape::TextBlock => samples::text_block(&[
            "[Epic Games] Subnautica  Free (100% off)",
            "[Amazon Prime] Star Wars Battlefront II  Free (100% off)",
            "[GOG] Galactic Civilizations III  Free (100% off)",
        ]),
        Shape::MarkdownTextBlock => samples::markdown(
            "- [[Epic Games] Subnautica  Free (100% off)](https://store.epicgames.com/p/subnautica)\n- [[Amazon Prime] Star Wars Battlefront II  Free (100% off)](https://gaming.amazon.com/loot/star-wars-battlefront-2)",
        ),
        Shape::Text => samples::text("[Epic Games] Subnautica  Free (100% off)"),
        Shape::Entries => samples::entries(&[
            ("Subnautica", "Free (100% off)"),
            ("Star Wars Battlefront II", "Free (100% off)"),
            ("Galactic Civilizations III", "Free (100% off)"),
        ]),
        Shape::Bars => samples::bars(&[
            ("Subnautica", 100),
            ("Star Wars Battlefront II", 100),
            ("Galactic Civilizations III", 100),
        ]),
        Shape::Badge => samples::badge(crate::payload::Status::Ok, "free this week"),
        Shape::Timeline => samples::timeline(&[
            (
                1_745_625_600,
                "Subnautica",
                Some("Epic Games · Free · 100% off"),
            ),
            (
                1_745_539_200,
                "Star Wars Battlefront II",
                Some("Amazon Prime · Free · 100% off"),
            ),
        ]),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use feed_rs::model::{Link, Text};

    fn opts(raw: &str) -> toml::Value {
        toml::from_str(raw).expect("test toml must parse")
    }

    fn titled_entry(title: &str) -> Entry {
        Entry {
            title: Some(Text {
                content_type: "text/plain".parse().unwrap(),
                src: None,
                content: title.into(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn parse_platform_defaults_to_all() {
        assert_eq!(parse_platform(None).unwrap(), Platform::All);
        assert_eq!(parse_platform(Some("  ")).unwrap(), Platform::All);
    }

    #[test]
    fn parse_platform_case_insensitive() {
        assert_eq!(parse_platform(Some("Epic")).unwrap(), Platform::Epic);
        assert_eq!(parse_platform(Some("STEAM")).unwrap(), Platform::Steam);
    }

    #[test]
    fn parse_platform_rejects_unknown() {
        let err = parse_platform(Some("origin")).unwrap_err();
        let FetchError::Failed(msg) = err else {
            panic!("expected Failed");
        };
        assert!(msg.contains("origin"));
        assert!(msg.contains("expected"));
    }

    #[test]
    fn parse_kind_defaults_to_game_and_rejects_unknown() {
        assert_eq!(parse_kind(None).unwrap(), Kind::Game);
        assert_eq!(parse_kind(Some("loot")).unwrap(), Kind::Loot);
        assert!(parse_kind(Some("bundle")).is_err());
    }

    #[test]
    fn feed_url_for_all_uses_aggregate_endpoint() {
        let url = Platform::All.feed_url(Kind::Game);
        assert_eq!(url, format!("{FEED_BASE}/lootscraper.xml"));
    }

    #[test]
    fn feed_url_for_specific_platform_includes_kind_slug() {
        assert_eq!(
            Platform::Epic.feed_url(Kind::Game),
            format!("{FEED_BASE}/lootscraper_epic_game.xml")
        );
        assert_eq!(
            Platform::Amazon.feed_url(Kind::Loot),
            format!("{FEED_BASE}/lootscraper_amazon_loot.xml")
        );
    }

    #[test]
    fn supports_loot_is_amazon_and_steam_only() {
        assert!(Platform::Amazon.supports_loot());
        assert!(Platform::Steam.supports_loot());
        for p in [
            Platform::Epic,
            Platform::Gog,
            Platform::Humble,
            Platform::Itch,
            Platform::Apple,
            Platform::Google,
        ] {
            assert!(!p.supports_loot(), "{:?} should not support loot", p);
        }
    }

    #[test]
    fn split_store_extracts_prefix_when_present() {
        let (store, title) = split_store_and_title("Amazon Prime (Game) - Captain Blood");
        assert_eq!(store.as_deref(), Some("Amazon Prime"));
        assert_eq!(title, "Captain Blood");
    }

    #[test]
    fn split_store_handles_loot_variant() {
        let (store, title) = split_store_and_title("Steam (Loot) - Some Cosmetic");
        assert_eq!(store.as_deref(), Some("Steam"));
        assert_eq!(title, "Some Cosmetic");
    }

    #[test]
    fn split_store_falls_back_for_unrecognised_titles() {
        let (store, title) = split_store_and_title("just a title");
        assert!(store.is_none());
        assert_eq!(title, "just a title");
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw = opts("platform = \"epic\"\nbogus = 1");
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn fetcher_exposes_safety_safe_and_default_linked_shape() {
        let f = FreeGamesFetcher;
        assert_eq!(f.name(), NAME);
        assert_eq!(f.safety(), Safety::Safe);
        assert_eq!(f.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(f.shapes(), SHAPES);
    }

    #[test]
    fn sample_body_covers_every_supported_shape() {
        let f = FreeGamesFetcher;
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
        // ImageLinkedList needs a real on-disk thumbnail path, skipped like rss.
        assert!(f.sample_body(Shape::ImageLinkedList).is_none());
        assert!(f.sample_body(Shape::Heatmap).is_none());
    }

    #[test]
    fn cache_key_varies_with_platform_option() {
        let f = FreeGamesFetcher;
        let base = FetchContext::default();
        let mut a = base.clone();
        let mut b = base.clone();
        a.options = Some(opts("platform = \"epic\""));
        b.options = Some(opts("platform = \"steam\""));
        assert_ne!(f.cache_key(&a), f.cache_key(&b));
    }

    fn fetch_err(toml: &str) -> String {
        let f = FreeGamesFetcher;
        let ctx = FetchContext {
            options: Some(opts(toml)),
            ..Default::default()
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        match rt.block_on(f.fetch(&ctx)) {
            Err(FetchError::Failed(msg)) => msg,
            other => panic!("expected FetchError::Failed, got {other:?}"),
        }
    }

    #[test]
    fn fetch_rejects_unknown_option_key() {
        // `deny_unknown_fields` on Options surfaces through `parse_options` before any network.
        assert!(fetch_err("bogus = 1").to_lowercase().contains("bogus"));
    }

    #[test]
    fn fetch_rejects_unknown_platform_before_network() {
        let msg = fetch_err("platform = \"origin\"");
        assert!(msg.contains("origin"), "msg: {msg}");
        assert!(msg.contains("unknown platform"), "msg: {msg}");
    }

    #[test]
    fn fetch_rejects_unknown_kind_before_network() {
        let msg = fetch_err("kind = \"bundle\"");
        assert!(msg.contains("bundle"), "msg: {msg}");
        assert!(msg.contains("unknown kind"), "msg: {msg}");
    }

    #[test]
    fn fetch_rejects_loot_on_non_loot_platform() {
        // Epic exposes no loot feed — the incompatibility is caught before constructing a URL.
        let msg = fetch_err("platform = \"epic\"\nkind = \"loot\"");
        assert!(msg.contains("loot"), "msg: {msg}");
        assert!(
            msg.contains("amazon") && msg.contains("steam"),
            "msg: {msg}"
        );
        assert!(msg.contains("epic"), "msg: {msg}");
    }

    #[test]
    fn entry_to_row_splits_store_and_marks_offer_free() {
        let entry = Entry {
            title: Some(Text {
                content_type: "text/plain".parse().unwrap(),
                src: None,
                content: "Epic Games (Game) - Subnautica".into(),
            }),
            published: Some(Utc.with_ymd_and_hms(2026, 4, 26, 12, 0, 0).unwrap()),
            links: vec![Link {
                href: "https://store.epicgames.com/p/subnautica".into(),
                rel: None,
                media_type: None,
                href_lang: None,
                title: None,
                length: None,
            }],
            ..Default::default()
        };
        let row = entry_to_row(&entry);
        assert_eq!(row.store.as_deref(), Some("Epic Games"));
        assert_eq!(row.title, "Subnautica");
        assert_eq!(row.link, "https://store.epicgames.com/p/subnautica");
        assert_eq!(row.sale_price.as_deref(), Some("Free"));
        assert_eq!(row.discount_pct, Some(100));
        assert_eq!(
            row.published,
            Some(Utc.with_ymd_and_hms(2026, 4, 26, 12, 0, 0).unwrap())
        );
    }

    #[test]
    fn entry_to_row_falls_back_for_titleless_linkless_entry() {
        let row = entry_to_row(&Entry::default());
        assert_eq!(row.title, "(unnamed offer)");
        assert_eq!(row.link, "https://example.com/");
        assert!(row.store.is_none());
    }

    #[test]
    fn entry_to_row_uses_updated_timestamp_when_published_absent() {
        let mut entry = titled_entry("just a title");
        entry.published = None;
        entry.updated = Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap());
        let row = entry_to_row(&entry);
        assert_eq!(row.published, entry.updated);
        // Title without the `<Store> (<Kind>) -` prefix keeps the raw text and no store.
        assert!(row.store.is_none());
        assert_eq!(row.title, "just a title");
    }

    const ALL_PLATFORMS: [Platform; 9] = [
        Platform::All,
        Platform::Epic,
        Platform::Steam,
        Platform::Amazon,
        Platform::Gog,
        Platform::Humble,
        Platform::Itch,
        Platform::Apple,
        Platform::Google,
    ];

    #[test]
    fn config_value_is_a_distinct_lowercase_token_for_every_platform() {
        let tokens: Vec<&str> = ALL_PLATFORMS.iter().map(|p| p.config_value()).collect();
        assert_eq!(
            tokens,
            [
                "all", "epic", "steam", "amazon", "gog", "humble", "itch", "apple", "google"
            ]
        );
    }

    #[test]
    fn url_slug_blanks_the_aggregate_feed_and_codes_every_storefront() {
        assert_eq!(Platform::All.url_slug(), "");
        let coded: Vec<&str> = ALL_PLATFORMS.iter().skip(1).map(|p| p.url_slug()).collect();
        assert_eq!(
            coded,
            [
                "epic", "steam", "amazon", "gog", "humble", "itch", "apple", "google"
            ]
        );
    }

    #[test]
    fn feed_url_for_every_non_aggregate_platform_embeds_its_slug() {
        for platform in ALL_PLATFORMS.iter().skip(1).copied() {
            let url = platform.feed_url(Kind::Game);
            assert_eq!(
                url,
                format!("{FEED_BASE}/lootscraper_{}_game.xml", platform.url_slug())
            );
        }
    }

    #[test]
    fn parse_platform_accepts_every_known_storefront() {
        for platform in ALL_PLATFORMS {
            let parsed = parse_platform(Some(platform.config_value())).unwrap();
            assert_eq!(parsed, platform);
        }
        // The matcher lowercases first, so a mixed-case storefront still resolves.
        assert_eq!(parse_platform(Some("GoG")).unwrap(), Platform::Gog);
    }

    #[test]
    fn parse_kind_accepts_the_explicit_game_keyword() {
        assert_eq!(parse_kind(Some("game")).unwrap(), Kind::Game);
        assert_eq!(parse_kind(Some(" GAME ")).unwrap(), Kind::Game);
    }
}
