//! Git / VCS fetchers backed by `gix`. All read-only (Safety::Safe) against the repo rooted at
//! the process CWD (walk-up via `gix::discover`). Each fetcher is multi-shape: they accept one
//! text variant (`Text` for single-string summaries, `TextBlock` for multi-row output) plus one
//! or two structural shapes that carry the real data.
//!
//! Cache keys mix the discovered repo root so running splashboard in two different projects
//! doesn't pollute each other's `git_status` cache entries.

use std::path::PathBuf;
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::payload::{Body, Payload, TextBlockData, TextData};

use super::{FetchContext, FetchError, Fetcher};

mod age;
mod blame_heatmap;
mod churn;
mod commits_activity;
mod contributors;
mod latest_tag;
mod recent_commits;
mod repo_name;
mod stash_count;
mod status;
mod worktrees;

#[cfg(test)]
pub(crate) mod test_support;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(status::GitStatus),
        Arc::new(stash_count::GitStashCount),
        Arc::new(recent_commits::GitRecentCommits),
        Arc::new(contributors::GitContributors),
        Arc::new(commits_activity::GitCommitsActivity),
        Arc::new(latest_tag::GitLatestTag),
        Arc::new(worktrees::GitWorktrees),
        Arc::new(blame_heatmap::GitBlameHeatmap),
        Arc::new(repo_name::GitRepoName),
        Arc::new(age::GitAge),
        Arc::new(churn::GitChurn),
    ]
}

pub(crate) fn open_repo() -> Result<gix::Repository, FetchError> {
    let cwd = std::env::current_dir().map_err(fail)?;
    gix::discover(&cwd).map_err(fail)
}

pub(crate) fn payload(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}

pub(crate) fn text_body(value: impl Into<String>) -> Body {
    Body::Text(TextData {
        value: value.into(),
    })
}

pub(crate) fn text_block_body(values: Vec<String>) -> Body {
    Body::TextBlock(TextBlockData { lines: values })
}

pub(crate) fn fail<E: std::fmt::Display>(e: E) -> FetchError {
    FetchError::Failed(e.to_string())
}

/// Cache key that mixes the discovered repo root in. Falls back to `cwd` when discovery fails so
/// two repos don't collide. Shape is included so `git_status` as Entries vs Badge gets separate
/// cache slots.
pub(crate) fn repo_cache_key(name: &str, ctx: &FetchContext) -> String {
    let root = discover_root().unwrap_or_else(|| PathBuf::from(""));
    let shape = ctx.shape.map(|s| s.as_str()).unwrap_or("default");
    let raw = format!(
        "{}|{}|{}|{}",
        name,
        root.display(),
        shape,
        ctx.format.as_deref().unwrap_or("")
    );
    let digest = Sha256::digest(raw.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{name}-{hex}")
}

fn discover_root() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let repo = gix::discover(&cwd).ok()?;
    repo.git_dir().parent().map(PathBuf::from)
}
