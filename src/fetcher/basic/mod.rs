//! `basic_*` family — config-only realtime fetchers, one per `Shape`. Each sibling lets users
//! author the widget's payload inline in TOML (`[widget.options]`) without writing a fetcher.
//! Pairs with `basic_static` (Text / TextBlock / MarkdownTextBlock, shipped) and
//! `basic_read_store` (file-based escape hatch, shipped). All are `Safety::Safe` (no I/O) and
//! `RealtimeFetcher` (pure config → payload, recomputed every frame).

use std::sync::Arc;

use super::RealtimeFetcher;

pub mod common;
pub mod links;

pub use links::BasicLinks;

pub fn realtime_fetchers() -> Vec<Arc<dyn RealtimeFetcher>> {
    vec![Arc::new(BasicLinks)]
}
