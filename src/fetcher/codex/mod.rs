//! Codex (OpenAI Codex CLI) fetchers.
//!
//! Scaffolding for the family. Individual fetchers (`codex_usage`, `codex_subscription`,
//! etc.) live as siblings of [`common`] and register through [`fetchers`].
//!
//! `Safety::Safe` — every member of the family reads from `~/.codex/sessions/` (or the
//! `CODEX_HOME` override) and never opens a network connection.

mod common;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetchers_entry_point_starts_empty_until_members_are_registered() {
        // Commit-1 contract: the entry point exists and returns an empty vec. Member fetchers
        // are added by subsequent commits; this assertion is amended as they land.
        assert!(fetchers().is_empty());
    }
}
