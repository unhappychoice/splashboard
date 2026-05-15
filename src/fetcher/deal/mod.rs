//! `deal_*` family — gaming and (future) retail deal feeds.
//!
//! Each sibling normalises its upstream into the shared [`common::DealRow`] vocabulary so the
//! same renderer chain (`list_links`, `list_cards`, `chart_bar`, `status_badge`, …) handles
//! every variant. Batch 1 ships three gaming-focused sources; the `deal_*` prefix leaves room
//! for non-gaming retail siblings without further renaming.

mod common;
mod free_games;
mod games;
mod steam_daily;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(free_games::FreeGamesFetcher),
        Arc::new(steam_daily::SteamDailyFetcher),
        Arc::new(games::GamesFetcher),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetchers_entry_point_registers_the_batch_under_deal_prefix() {
        let names: Vec<String> = fetchers().iter().map(|f| f.name().to_string()).collect();
        assert!(names.contains(&"deal_free_games".to_string()));
        assert!(names.contains(&"deal_steam_daily".to_string()));
        assert!(names.contains(&"deal_games".to_string()));
        for name in &names {
            assert!(
                name.starts_with("deal_"),
                "{name} must use the `deal_` family prefix"
            );
        }
    }
}
