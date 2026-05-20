//! Steam fetchers — Steam Web API readouts: player profile, recently-played games, owned
//! game library, and the global most-played chart.
//!
//! Every authenticated request targets the fixed host `api.steampowered.com`. Config-provided
//! fields (`steam_id`, `count`, …) only change which resource on that host is queried; the API
//! key never leaves to an attacker-controlled origin. Classified as `Safety::Safe`, same model
//! as the `github_*` and `lastfm_*` families.
//!
//! Auth: `STEAM_API_KEY` — create one at https://steamcommunity.com/dev/apikey and put it in
//! `$HOME/.splashboard/secrets.toml`.

mod client;
mod player_summary;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![Arc::new(player_summary::SteamPlayerSummary)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetchers_entry_point_registers_the_batch_under_steam_prefix() {
        let names: Vec<String> = fetchers().iter().map(|f| f.name().to_string()).collect();
        for name in &names {
            assert!(
                name.starts_with("steam_"),
                "{name} must use the `steam_` family prefix"
            );
        }
        assert!(names.contains(&"steam_player_summary".to_string()));
    }
}
