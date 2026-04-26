//! `hackernews_*` fetcher family. Splits into:
//!
//! - `top` — story listings (top / new / best / ask / show / job)
//!
//! Shared HTTP client and base URLs live in `client`.

pub mod client;
pub mod top;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![Arc::new(top::HackernewsTopFetcher)]
}
