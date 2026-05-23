//! `gitlab_repo_issues` — open issues against one specific GitLab project.

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::forge_items::{self, LIST_SHAPES};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::{default_host, rest_get};
use super::common::{ProjectPath, cache_key, parse_options, payload, resolve_project};
use super::items::{GitlabIssueLike, sample_rows, to_forge_rows};
use super::my_mrs::resolve_host;

const DEFAULT_LIMIT: u32 = 10;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "host",
        type_hint: "hostname",
        required: false,
        default: Some("gitlab.com"),
        description: "GitLab instance host. Use this for self-hosted GitLab.",
    },
    OptionSchema {
        name: "project",
        type_hint: "\"group/name\" or \"group/sub/name\"",
        required: false,
        default: Some("git remote of cwd"),
        description: "Project to query. Falls back to the current directory's GitLab remote.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Maximum number of issues to show.",
    },
];

pub struct GitlabRepoIssues;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for GitlabRepoIssues {
    fn name(&self) -> &str {
        "gitlab_repo_issues"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Open issues against one specific GitLab project. Mirrors `github_repo_issues` for GitLab."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 10
    }
    fn shapes(&self) -> &[Shape] {
        LIST_SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        cache_key(self.name(), ctx, &cache_extra(ctx))
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        let rows = sample_rows(&[
            (
                "#75",
                "Heatmap renderer eats narrow columns",
                Some("https://gitlab.com/g/splashboard/-/issues/75"),
                3,
                1_774_000_000,
            ),
            (
                "#71",
                "gitlab_pipeline_status: respect branch option",
                Some("https://gitlab.com/g/splashboard/-/issues/71"),
                1,
                1_773_800_000,
            ),
        ]);
        forge_items::dispatch_sample(&rows, shape, "open issue", "open issues")
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let host = resolve_host(opts.host.as_deref())?;
        let project = resolve_project(opts.project.as_deref(), &host)?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 30);
        let path = format!(
            "/projects/{}/issues?state=opened&per_page={limit}&order_by=updated_at",
            project.encoded()
        );
        let items: Vec<GitlabIssueLike> = rest_get(&host, &path).await?;
        let rows = to_forge_rows(&items, false);
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let body = forge_items::dispatch_rows_async(rows, shape, "open issue", "open issues").await;
        Ok(payload(body))
    }
}

fn cache_extra(ctx: &FetchContext) -> String {
    let host = ctx
        .options
        .as_ref()
        .and_then(|v| v.get("host"))
        .and_then(|v| v.as_str())
        .unwrap_or(default_host())
        .to_string();
    let project = ctx
        .options
        .as_ref()
        .and_then(|v| v.get("project"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| {
            resolve_project(None, &host)
                .ok()
                .map(|p: ProjectPath| p.as_str().to_string())
        })
        .unwrap_or_default();
    format!("{host}|{project}")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn options_deserialize_and_reject_unknown_keys() {
        let opts: Options =
            toml::from_str("host = \"git.example.org\"\nproject = \"g/p\"\nlimit = 3").unwrap();
        assert_eq!(opts.host.as_deref(), Some("git.example.org"));
        assert_eq!(opts.project.as_deref(), Some("g/p"));
        assert_eq!(opts.limit, Some(3));
        assert!(toml::from_str::<Options>("bogus = 1").is_err());
    }

    #[test]
    fn shapes_table_matches_shared_list() {
        let fetcher = GitlabRepoIssues;
        assert_eq!(fetcher.shapes(), LIST_SHAPES);
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
    }

    #[test]
    fn sample_body_covers_every_list_shape() {
        let fetcher = GitlabRepoIssues;
        for shape in LIST_SHAPES {
            assert!(fetcher.sample_body(*shape).is_some(), "missing {shape:?}");
        }
        assert!(fetcher.sample_body(Shape::Ratio).is_none());
    }

    #[test]
    fn metadata_methods_have_content() {
        let fetcher = GitlabRepoIssues;
        assert_eq!(fetcher.name(), "gitlab_repo_issues");
        assert!(fetcher.description().contains("GitLab"));
        assert_eq!(fetcher.refresh_interval(), 60 * 10);
    }

    #[test]
    fn option_schemas_lists_host_project_and_limit() {
        let names: Vec<&str> = GitlabRepoIssues
            .option_schemas()
            .iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["host", "project", "limit"]);
    }

    #[test]
    fn cache_key_is_name_prefixed_and_varies_with_project_option() {
        let fetcher = GitlabRepoIssues;
        let a = fetcher.cache_key(&FetchContext {
            options: Some(toml::from_str("project = \"a/b\"").unwrap()),
            ..Default::default()
        });
        let b = fetcher.cache_key(&FetchContext {
            options: Some(toml::from_str("project = \"a/c\"").unwrap()),
            ..Default::default()
        });
        assert!(a.starts_with("gitlab_repo_issues-"));
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_varies_with_host_option() {
        let fetcher = GitlabRepoIssues;
        let a = fetcher.cache_key(&FetchContext {
            options: Some(toml::from_str("host = \"git.one.org\"\nproject = \"a/b\"").unwrap()),
            ..Default::default()
        });
        let b = fetcher.cache_key(&FetchContext {
            options: Some(toml::from_str("host = \"git.two.org\"\nproject = \"a/b\"").unwrap()),
            ..Default::default()
        });
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_with_no_options_falls_back_to_remote_resolution() {
        // No `project` option: `cache_extra` walks the cwd git remote, which here is a GitHub
        // repo, so the gitlab.com lookup yields nothing and the key still resolves cleanly.
        let key = GitlabRepoIssues.cache_key(&FetchContext::default());
        assert!(key.starts_with("gitlab_repo_issues-"));
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_option_keys_before_network_request() {
        let err = GitlabRepoIssues
            .fetch(&FetchContext {
                options: Some(toml::from_str("bogus = 1").unwrap()),
                timeout: Duration::from_secs(1),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(m) if m.contains("invalid options")
        ));
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_host_before_network_request() {
        let err = GitlabRepoIssues
            .fetch(&FetchContext {
                options: Some(toml::from_str("host = \"https://evil.example\"").unwrap()),
                timeout: Duration::from_secs(1),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(m) if m.contains("invalid gitlab host")
        ));
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_project_before_network_request() {
        let err = GitlabRepoIssues
            .fetch(&FetchContext {
                options: Some(toml::from_str("project = \"not a path\"").unwrap()),
                timeout: Duration::from_secs(1),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(m) if m.contains("invalid project option")
        ));
    }
}
