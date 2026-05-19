//! Last.fm fetchers — user listening statistics (recent scrobbles, top artists / tracks /
//! albums) plus global charts.
//!
//! Every request targets the fixed host `ws.audioscrobbler.com`. Config-provided fields
//! (`user`, `period`, …) only change which resource on that host is queried; the API key
//! never leaves to an attacker-controlled origin. Classified as `Safety::Safe`, same model
//! as the `github_*` family.
//!
//! Auth: `LASTFM_API_KEY` — create one at https://www.last.fm/api/account/create and put it
//! in `$HOME/.splashboard/secrets.toml`.

mod client;
mod common;
mod scrobbles_today;
mod top;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![Arc::new(scrobbles_today::LastfmScrobblesToday)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetchers_entry_point_registers_the_batch_under_lastfm_prefix() {
        let names: Vec<String> = fetchers().iter().map(|f| f.name().to_string()).collect();
        for name in &names {
            assert!(
                name.starts_with("lastfm_"),
                "{name} must use the `lastfm_` family prefix"
            );
        }
        assert!(names.contains(&"lastfm_scrobbles_today".to_string()));
    }
}
