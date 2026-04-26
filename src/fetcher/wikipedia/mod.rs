//! `wikipedia_*` fetcher family. Splits into:
//!
//! - `on_this_day` — historical events for today's MM/DD via `feed/onthisday`.
//! - `featured` — today's Featured Article (TFA) via `feed/featured`.
//! - `random` — a random page summary via `page/random/summary`.
//!
//! Shared HTTP client, base URL builder, page-summary type, and shape-aware rendering for
//! single-article fetchers live in `client`.

pub mod client;
pub mod featured;
pub mod on_this_day;
pub mod random;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(on_this_day::WikipediaOnThisDayFetcher),
        Arc::new(featured::WikipediaFeaturedFetcher),
        Arc::new(random::WikipediaRandomFetcher),
    ]
}
