//! GitLab fetchers. All classify as `Safety::Safe`: the API host is config-fixed at trust time
//! via the `host` option (default `gitlab.com`, validated against a hostname-only allowlist),
//! and the auth token (`GITLAB_TOKEN`) only ever leaves to that pinned host. Mirrors the
//! `github_*` family contract; differences from GitHub are documented in
//! [`client`] and [`common`].
//!
//! Auth: `GITLAB_TOKEN` env var (Personal Access Token with `read_api` scope is enough for the
//! shipped fetchers). Missing auth surfaces as a placeholder payload, not a panic.

use std::sync::Arc;

pub(crate) mod client;
pub(crate) mod common;
pub(crate) mod items;
mod my_mrs;

use super::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![Arc::new(my_mrs::GitlabMyMrs)]
}
