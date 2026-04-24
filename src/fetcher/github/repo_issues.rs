//! `github_repo_issues` — open issues for a specific repo. `/repos/{o}/{r}/issues` returns PRs
//! too; we filter them out client-side (the `pull_request` marker is the canonical signal).

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{RepoSlug, cache_key, parse_options, payload, resolve_repo};
use super::items::{IssueItem, render_items};

const SHAPES: &[Shape] = &[Shape::TextBlock, Shape::Entries, Shape::Timeline];
const DEFAULT_LIMIT: u32 = 10;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "repo",
        type_hint: "\"owner/name\"",
        required: false,
        default: Some("git remote of cwd"),
        description: "Repository to list issues for. Falls back to the current directory's github remote.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Maximum number of issues to show.",
    },
];

pub struct GithubRepoIssues;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for GithubRepoIssues {
    fn name(&self) -> &str {
        "github_repo_issues"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let extra = repo_for_key(ctx);
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::TextBlock => {
                samples::text_block(&["#41 meta: widget catalog & roadmap", "#17 theme system"])
            }
            Shape::Entries => {
                samples::entries(&[("#41", "meta: widget catalog"), ("#17", "theme system")])
            }
            Shape::Timeline => samples::timeline(&[
                (1_774_000_000, "#41", Some("meta: widget catalog")),
                (1_773_500_000, "#17", Some("theme system")),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let slug = resolve_repo(opts.repo.as_deref())?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 30);
        // Over-fetch to compensate for pulls mixed into the response — the endpoint has no
        // server-side "issues only" flag. Cap at 100 (max per_page) so the fix-up never has to
        // paginate twice for a reasonable `limit`.
        let fetch_size = (limit * 2).min(100);
        let path = format!(
            "/repos/{}/{}/issues?state=open&sort=updated&direction=desc&per_page={fetch_size}",
            slug.owner, slug.name
        );
        let items: Vec<RepoIssueItem> = rest_get(&path).await?;
        let issues: Vec<IssueItem> = items
            .into_iter()
            .filter(|i| i.pull_request.is_none())
            .take(limit as usize)
            .map(|i| i.inner)
            .collect();
        Ok(payload(render_items(
            &issues,
            ctx.shape.unwrap_or(Shape::TextBlock),
            false,
        )))
    }
}

#[derive(Debug, Deserialize)]
struct RepoIssueItem {
    #[serde(default)]
    pull_request: Option<serde_json::Value>,
    #[serde(flatten)]
    inner: IssueItem,
}

fn repo_for_key(ctx: &FetchContext) -> String {
    ctx.options
        .as_ref()
        .and_then(|v| v.get("repo"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| resolve_repo(None).ok().map(|s: RepoSlug| s.as_path()))
        .unwrap_or_default()
}
