//! Claude (and Claude Code) fetchers.
//!
//! - [`code_usage::ClaudeCodeUsage`] aggregates the local JSONL files Claude Code writes for
//!   every session, with zero configuration and no network.
//! - [`subscription::ClaudeSubscription`] reads the OAuth token Claude Code stores in
//!   `~/.claude/.credentials.json` and reports Max-plan utilisation against an undocumented
//!   `api.anthropic.com` endpoint.
//!
//! Both are `Safety::Safe` — `code_usage` is local-only, `subscription` only ever talks to the
//! hardcoded `api.anthropic.com` host (same model as the `github_*` / `lastfm_*` families).

mod code_usage;
mod common;
mod subscription;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(code_usage::ClaudeCodeUsage),
        Arc::new(subscription::ClaudeSubscription),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetchers_entry_point_registers_each_member_under_claude_prefix() {
        let names: Vec<String> = fetchers().iter().map(|f| f.name().to_string()).collect();
        for name in &names {
            assert!(
                name.starts_with("claude_"),
                "{name} must use the `claude_` family prefix"
            );
        }
        assert!(names.contains(&"claude_code_usage".to_string()));
        assert!(names.contains(&"claude_subscription".to_string()));
    }
}
