//! Codex (OpenAI Codex CLI) fetchers.
//!
//! - [`usage::CodexUsage`] aggregates the local `rollout-*.jsonl` files Codex CLI writes for
//!   every session, with zero configuration and no network.
//!
//! `Safety::Safe` — every read is rooted at `~/.codex/sessions/` (or the `CODEX_HOME`
//! override) and no network connection is opened.

mod common;
mod usage;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![Arc::new(usage::CodexUsage)]
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
    }
}
