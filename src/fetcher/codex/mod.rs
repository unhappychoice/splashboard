//! Codex (OpenAI Codex CLI) fetchers.
//!
//! - [`usage::CodexUsage`] aggregates the local `rollout-*.jsonl` files Codex CLI writes for
//!   every session, with zero configuration and no network.
//! - [`subscription::CodexSubscription`] extracts rate-limit utilisation (5h / 7d windows +
//!   plan type) from the most recent session's last `token_count` event.
//!
//! Both are `Safety::Safe` — all reads are rooted at `~/.codex/sessions/`. No network, no
//! token leaves the host (the rate-limit data is already cached in the JSONL by Codex CLI;
//! re-fetching it would mean making an inference call, which costs money).

mod common;
mod subscription;
mod usage;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(usage::CodexUsage),
        Arc::new(subscription::CodexSubscription),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetchers_entry_point_registers_each_member_under_codex_prefix() {
        let names: Vec<String> = fetchers().iter().map(|f| f.name().to_string()).collect();
        for name in &names {
            assert!(
                name.starts_with("codex_"),
                "{name} must use the `codex_` family prefix"
            );
        }
        assert!(names.contains(&"codex_usage".to_string()));
        assert!(names.contains(&"codex_subscription".to_string()));
    }
}
