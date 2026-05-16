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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::TEST_ENV_LOCK;
    use crate::render::Shape;
    use std::io;

    /// Direct unit on the shared `fail` helper so its single-line body is exercised even when
    /// every production caller happens to follow a happy path during the test run.
    #[test]
    fn fail_wraps_display_into_failed_variant() {
        let err = fail(io::Error::new(io::ErrorKind::NotFound, "missing repo"));
        assert!(matches!(err, FetchError::Failed(msg) if msg.contains("missing repo")));
    }

    /// `payload` / `text_body` / `text_block_body` share the same single-call pattern across
    /// every fetcher in this module; pin their shapes here so a future refactor that drops a
    /// helper trips a direct test instead of a diffuse failure across siblings.
    #[test]
    fn text_helpers_build_expected_shapes() {
        let p = payload(text_body("ready"));
        assert!(p.icon.is_none() && p.status.is_none() && p.format.is_none());
        assert!(matches!(p.body, Body::Text(t) if t.value == "ready"));
        let block = text_block_body(vec!["a".into(), "b".into()]);
        assert!(matches!(block, Body::TextBlock(d) if d.lines == vec!["a", "b"]));
    }

    /// `open_repo` + `repo_cache_key` both discover the workspace from the process CWD, which
    /// is the splashboard repo itself when `cargo test` runs. Asserting on the success path
    /// covers `open_repo`, `discover_root`, and `repo_cache_key`'s digest body in one call.
    /// Holds `TEST_ENV_LOCK` so it doesn't race with other env-mutating tests.
    #[test]
    fn repo_helpers_resolve_against_splashboard_workspace() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(open_repo().is_ok(), "splashboard repo must be discoverable");
        assert!(discover_root().is_some(), "expected splashboard repo root");
        let ctx_heatmap = FetchContext {
            shape: Some(Shape::Heatmap),
            ..FetchContext::default()
        };
        let ctx_text = FetchContext {
            shape: Some(Shape::Text),
            ..FetchContext::default()
        };
        let key_a = repo_cache_key("git_test", &ctx_heatmap);
        let key_b = repo_cache_key("git_test", &ctx_heatmap);
        let key_c = repo_cache_key("git_test", &ctx_text);
        assert_eq!(key_a, key_b);
        assert_ne!(key_a, key_c);
        assert!(key_a.starts_with("git_test-"));
    }
}
