//! `gitlab_repo_mrs` — open merge requests against one specific GitLab project. Falls back to
//! the git remote of the cwd when `project` is unset.

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
        description: "Maximum number of MRs to show.",
    },
];

pub struct GitlabRepoMrs;

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
impl Fetcher for GitlabRepoMrs {
    fn name(&self) -> &str {
        "gitlab_repo_mrs"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Open merge requests against one specific GitLab project. Use `gitlab_my_mrs` instead for MRs you authored across every project."
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
                "!54",
                "feat(docs): generate widget catalogue",
                Some("https://gitlab.com/g/splashboard/-/merge_requests/54"),
                7,
                1_774_000_000,
            ),
            (
                "!51",
                "feat(fetcher): split clock options",
                Some("https://gitlab.com/g/splashboard/-/merge_requests/51"),
                2,
                1_773_800_000,
            ),
        ]);
        forge_items::dispatch_sample(&rows, shape, "open MR", "open MRs")
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let host = resolve_host(opts.host.as_deref())?;
        let project = resolve_project(opts.project.as_deref(), &host)?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 30);
        let path = format!(
            "/projects/{}/merge_requests?state=opened&per_page={limit}&order_by=updated_at",
            project.encoded()
        );
        let items: Vec<GitlabIssueLike> = rest_get(&host, &path).await?;
        let rows = to_forge_rows(&items, false);
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let body = forge_items::dispatch_rows_async(rows, shape, "open MR", "open MRs").await;
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
        let fetcher = GitlabRepoMrs;
        assert_eq!(fetcher.shapes(), LIST_SHAPES);
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
    }

    #[test]
    fn sample_body_covers_every_list_shape() {
        let fetcher = GitlabRepoMrs;
        for shape in LIST_SHAPES {
            assert!(fetcher.sample_body(*shape).is_some(), "missing {shape:?}");
        }
        assert!(fetcher.sample_body(Shape::Ratio).is_none());
    }

    #[test]
    fn cache_key_changes_with_project_option() {
        let fetcher = GitlabRepoMrs;
        let a = fetcher.cache_key(&FetchContext {
            options: Some(toml::from_str("project = \"a/b\"").unwrap()),
            ..Default::default()
        });
        let b = fetcher.cache_key(&FetchContext {
            options: Some(toml::from_str("project = \"a/c\"").unwrap()),
            ..Default::default()
        });
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_project_before_network_request() {
        let err = GitlabRepoMrs
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
