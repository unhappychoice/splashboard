//! `basic_*` family — config-only realtime fetchers, one per `Shape`. Each sibling lets users
//! author the widget's payload inline in TOML (`[widget.options]`) without writing a fetcher.
//! Pairs with `basic_static` (Text / TextBlock / MarkdownTextBlock, shipped) and
//! `basic_read_store` (file-based escape hatch, shipped). All are `Safety::Safe` (no I/O) and
//! `RealtimeFetcher` (pure config → payload, recomputed every frame).

use std::sync::Arc;

use super::RealtimeFetcher;

pub mod badge;
pub mod bars;
pub mod common;
pub mod entries;
pub mod image;
pub mod links;
pub mod numbers;
pub mod points;
pub mod ratio;

pub use badge::BasicBadge;
pub use bars::BasicBars;
pub use entries::BasicEntries;
pub use image::BasicImage;
pub use links::BasicLinks;
pub use numbers::BasicNumbers;
pub use points::BasicPoints;
pub use ratio::BasicRatio;

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
    ]
}
