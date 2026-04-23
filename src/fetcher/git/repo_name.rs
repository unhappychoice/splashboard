//! `git_repo_name` — repo name as Text, suitable for hero rendering. Derived from the
//! `origin` remote (or the first configured remote) parsed as `owner/name`; falls back to the
//! cwd's directory name when no github remote exists so a local-only clone still has a header.

use async_trait::async_trait;

use crate::payload::{Body, Payload, TextData};
use crate::render::Shape;

use super::super::github::common::slug_from_url;
use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{payload, repo_cache_key, text_body};

pub struct GitRepoName;

#[async_trait]
impl Fetcher for GitRepoName {
    fn name(&self) -> &str {
        "git_repo_name"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Text]
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        repo_cache_key(self.name(), ctx)
    }
    fn sample_body(&self, _shape: Shape) -> Option<Body> {
        Some(Body::Text(TextData {
            value: "splashboard".into(),
        }))
    }
    async fn fetch(&self, _ctx: &FetchContext) -> Result<Payload, FetchError> {
        Ok(payload(text_body(
            detect_name().unwrap_or_else(|| "project".into()),
        )))
    }
}

fn detect_name() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    remote_name(&cwd).or_else(|| cwd.file_name().map(|s| s.to_string_lossy().into_owned()))
}

fn remote_name(cwd: &std::path::Path) -> Option<String> {
    let repo = gix::discover(cwd).ok()?;
    let names: Vec<_> = repo.remote_names().into_iter().collect();
    let preferred = names
        .iter()
        .find(|n| n.as_ref() == "origin")
        .or_else(|| names.first())?;
    let remote = repo.find_remote(preferred.as_ref()).ok()?;
    let url = remote.url(gix::remote::Direction::Fetch)?;
    slug_from_url(&url.to_bstring().to_string()).map(|s| s.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_is_non_empty() {
        let sample = GitRepoName.sample_body(Shape::Text).unwrap();
        match sample {
            Body::Text(d) => assert!(!d.value.is_empty()),
            _ => panic!("expected text sample"),
        }
    }
}
