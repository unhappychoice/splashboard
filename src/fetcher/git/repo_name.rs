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
    fn description(&self) -> &'static str {
        "Repository name as a single line, suitable for a hero header. Derived from the `origin` remote (parsed as `owner/name`); falls back to the working directory's basename when no remote is configured."
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
    use std::future::Future;
    use std::path::Path;
    use std::process::Command;
    use std::time::Duration;

    use super::*;
    use crate::fetcher::git::test_support::make_repo;

    fn run_git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn ctx() -> FetchContext {
        FetchContext {
            widget_id: "repo-name".into(),
            format: None,
            timeout: Duration::from_secs(1),
            file_format: None,
            shape: Some(Shape::Text),
            options: None,
            timezone: None,
            locale: None,
        }
    }

    fn run_async<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn sample_is_non_empty() {
        let sample = GitRepoName.sample_body(Shape::Text).unwrap();
        match sample {
            Body::Text(d) => assert!(!d.value.is_empty()),
            _ => panic!("expected text sample"),
        }
    }

    #[test]
    fn fetcher_metadata_matches_repo_name_contract() {
        let fetcher = GitRepoName;
        assert_eq!(fetcher.name(), "git_repo_name");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("hero header"));
        assert_eq!(fetcher.shapes(), &[Shape::Text]);
        assert_eq!(fetcher.default_shape(), Shape::Text);
        assert!(fetcher.cache_key(&ctx()).starts_with("git_repo_name-"));
    }

    #[test]
    fn remote_name_prefers_origin_when_multiple_remotes_exist() {
        let (_tmp, repo) = make_repo();
        let workdir = repo.workdir().unwrap();
        run_git(
            workdir,
            &[
                "remote",
                "add",
                "upstream",
                "https://github.com/acme/first.git",
            ],
        );
        run_git(
            workdir,
            &[
                "remote",
                "add",
                "origin",
                "git@github.com:acme/preferred.git",
            ],
        );

        assert_eq!(remote_name(workdir), Some("preferred".into()));
    }

    #[test]
    fn remote_name_uses_first_remote_when_origin_is_missing() {
        let (_tmp, repo) = make_repo();
        let workdir = repo.workdir().unwrap();
        run_git(
            workdir,
            &[
                "remote",
                "add",
                "upstream",
                "ssh://git@github.com/acme/fallback.git",
            ],
        );

        assert_eq!(remote_name(workdir), Some("fallback".into()));
    }

    #[test]
    fn remote_name_returns_none_for_non_git_repos_and_non_github_remotes() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(remote_name(tmp.path()).is_none());

        let (_repo_tmp, repo) = make_repo();
        let workdir = repo.workdir().unwrap();
        run_git(
            workdir,
            &[
                "remote",
                "add",
                "origin",
                "https://gitlab.com/acme/project.git",
            ],
        );

        assert!(remote_name(workdir).is_none());
    }

    #[test]
    fn detect_name_uses_the_workspace_remote() {
        assert_eq!(detect_name(), Some("splashboard".into()));
    }

    #[test]
    fn fetch_returns_workspace_repo_name() {
        let payload = run_async(GitRepoName.fetch(&ctx())).unwrap();
        let Body::Text(text) = payload.body else {
            panic!("expected text payload");
        };

        assert_eq!(text.value, "splashboard");
    }
}
