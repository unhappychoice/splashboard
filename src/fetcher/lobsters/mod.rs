//! `lobsters_*` fetcher family. Targets `lobste.rs` — fixed host, no auth, so every fetcher
//! in the family stays Safety::Safe regardless of what config supplies.
//!
//! - `top` — story listings (hottest / newest / active), optionally filtered by tag.
//!
//! Shared HTTP client and base URLs live in `client`.

pub mod client;
pub mod top;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![Arc::new(top::LobstersTopFetcher)]
}
