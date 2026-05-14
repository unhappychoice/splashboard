//! `basic_*` family — config-only realtime fetchers, one per `Shape`. Each sibling lets users
//! author the widget's payload inline in TOML (`[widget.options]`) without writing a fetcher.
//! Pairs with `basic_static` (Text / TextBlock / MarkdownTextBlock, shipped) and
//! `basic_read_store` (file-based escape hatch, shipped). All are `Safety::Safe` (no I/O) and
//! `RealtimeFetcher` (pure config → payload, recomputed every frame).

use std::sync::Arc;

use super::RealtimeFetcher;

pub mod badge;
pub mod bars;
pub mod calendar;
pub mod common;
pub mod entries;
pub mod heatmap;
pub mod image;
pub mod links;
pub mod numbers;
pub mod points;
pub mod ratio;
pub mod timeline;

pub use badge::BasicBadge;
pub use bars::BasicBars;
pub use calendar::BasicCalendar;
pub use entries::BasicEntries;
pub use heatmap::BasicHeatmap;
pub use image::BasicImage;
pub use links::BasicLinks;
pub use numbers::BasicNumbers;
pub use points::BasicPoints;
pub use ratio::BasicRatio;
pub use timeline::BasicTimeline;

pub fn realtime_fetchers() -> Vec<Arc<dyn RealtimeFetcher>> {
    vec![
        Arc::new(BasicLinks),
        Arc::new(BasicImage),
        Arc::new(BasicBadge),
        Arc::new(BasicRatio),
        Arc::new(BasicBars),
        Arc::new(BasicEntries),
        Arc::new(BasicNumbers),
        Arc::new(BasicPoints),
        Arc::new(BasicTimeline),
        Arc::new(BasicCalendar),
        Arc::new(BasicHeatmap),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetcher::{FetchContext, Safety};
    use crate::payload::Body;
    use crate::render::{Shape, shape_of};

    const ALL_SHAPES: &[Shape] = &[
        Shape::Text,
        Shape::TextBlock,
        Shape::MarkdownTextBlock,
        Shape::LinkedTextBlock,
        Shape::ImageLinkedList,
        Shape::Entries,
        Shape::Ratio,
        Shape::NumberSeries,
        Shape::PointSeries,
        Shape::Bars,
        Shape::Image,
        Shape::Calendar,
        Shape::Heatmap,
        Shape::Badge,
        Shape::Timeline,
        Shape::Error,
    ];

    /// The family exists to make every payload `Shape` emittable from config alone. The three
    /// text shapes (`Text` / `TextBlock` / `MarkdownTextBlock`) are already owned by the shipped
    /// `basic_static`, and `Error` is the placeholder pseudo-shape — every *other* shape must be
    /// reachable through some fetcher's `shapes()` here. Note `ImageLinkedList` rides along on
    /// `basic_links` (same "list of links" data, thumbnail variant) rather than getting its own
    /// fetcher, so this checks `shapes()` membership, not `default_shape()`.
    #[test]
    fn family_makes_every_non_text_payload_shape_reachable() {
        let fetchers = realtime_fetchers();
        let names: Vec<&str> = fetchers.iter().map(|f| f.name()).collect();
        for expected in [
            "basic_links",
            "basic_image",
            "basic_badge",
            "basic_ratio",
            "basic_bars",
            "basic_entries",
            "basic_numbers",
            "basic_points",
            "basic_timeline",
            "basic_calendar",
            "basic_heatmap",
        ] {
            assert!(
                names.contains(&expected),
                "{expected} missing from realtime_fetchers()"
            );
        }

        let owned_by_basic_static = [
            Shape::Text,
            Shape::TextBlock,
            Shape::MarkdownTextBlock,
            Shape::Error,
        ];
        for &shape in ALL_SHAPES {
            if owned_by_basic_static.contains(&shape) {
                continue;
            }
            assert!(
                fetchers.iter().any(|f| f.shapes().contains(&shape)),
                "no basic_* fetcher can emit {shape:?}"
            );
        }
    }

    /// Contract sweep over the whole family: `Safe`, `basic_` prefix, `default_shape` is a
    /// member of `shapes()`, and `sample_body` returns `Some` with a matching `Body` for every
    /// declared shape and `None` for every undeclared one.
    #[test]
    fn every_fetcher_satisfies_the_realtime_contract() {
        for f in realtime_fetchers() {
            let name = f.name();
            assert!(name.starts_with("basic_"), "{name} missing basic_ prefix");
            assert_eq!(f.safety(), Safety::Safe, "{name} must be Safe");
            assert!(!f.shapes().is_empty(), "{name} declares no shapes");
            assert!(
                f.shapes().contains(&f.default_shape()),
                "{name} default_shape() not in shapes()"
            );
            for &shape in ALL_SHAPES {
                let declared = f.shapes().contains(&shape);
                match f.sample_body(shape) {
                    Some(body) => {
                        assert!(
                            declared,
                            "{name} sample_body({shape:?}) is Some for an undeclared shape"
                        );
                        assert_eq!(
                            shape_of(&body),
                            shape,
                            "{name} sample_body({shape:?}) returned a mismatched Body"
                        );
                    }
                    None => assert!(
                        !declared,
                        "{name} declares {shape:?} but sample_body returned None"
                    ),
                }
            }
        }
    }

    /// Every fetcher's `Options` struct is `#[serde(deny_unknown_fields)]`, so a typo'd key
    /// must surface the shared two-line placeholder rather than being silently ignored.
    #[test]
    fn every_fetcher_rejects_unknown_option_keys() {
        let bogus: toml::Value =
            toml::from_str("definitely_not_a_real_option = true").expect("valid toml");
        for f in realtime_fetchers() {
            let ctx = FetchContext {
                options: Some(bogus.clone()),
                ..Default::default()
            };
            match f.compute(&ctx).body {
                Body::TextBlock(d) => assert!(
                    d.lines
                        .first()
                        .is_some_and(|line| line.contains("invalid options")),
                    "{} unknown-key placeholder missing 'invalid options': {:?}",
                    f.name(),
                    d.lines
                ),
                other => panic!(
                    "{} should render a placeholder on an unknown option key, got {other:?}",
                    f.name()
                ),
            }
        }
    }
}
