//! `github_repo_prs` — open pull requests for a specific repo. Repo comes from the `repo`
//! option or (fallback) the git remote of the current directory.

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
        description: "Repository to list PRs for. Falls back to the current directory's github remote.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Maximum number of PRs to show.",
    },
];

pub struct GithubRepoPrs;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for GithubRepoPrs {
    fn name(&self) -> &str {
        "github_repo_prs"
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
            Shape::TextBlock => samples::text_block(&[
                "#54 feat(docs): generate widget catalogue",
                "#51 feat(fetcher): split clock options",
            ]),
            Shape::Entries => samples::entries(&[
                ("#54", "feat(docs): widget catalogue"),
                ("#51", "feat(fetcher): split clock options"),
            ]),
            Shape::Timeline => samples::timeline(&[
                (1_774_000_000, "#54", Some("feat(docs): widget catalogue")),
                (
                    1_773_800_000,
                    "#51",
                    Some("feat(fetcher): split clock options"),
                ),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let slug = resolve_repo(opts.repo.as_deref())?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 30);
        let path = format!(
            "/repos/{}/{}/pulls?state=open&sort=updated&direction=desc&per_page={limit}",
            slug.owner, slug.name
        );
        let items: Vec<IssueItem> = rest_get(&path).await?;
        Ok(payload(render_items(
            &items,
            ctx.shape.unwrap_or(Shape::TextBlock),
            false,
        )))
    }
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
