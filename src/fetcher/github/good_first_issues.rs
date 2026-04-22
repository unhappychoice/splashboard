//! `github_good_first_issues` — open issues labelled `good-first-issue`. Scoped to a single
//! repo when `repo` is set, otherwise a global search.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{RepoSlug, cache_key, parse_options, payload, placeholder};
use super::items::{SearchResult, render_items};

const SHAPES: &[Shape] = &[Shape::TextBlock, Shape::Entries, Shape::Timeline];
const DEFAULT_LIMIT: u32 = 10;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "repo",
        type_hint: "\"owner/name\"",
        required: false,
        default: None,
        description: "Restrict the search to one repo. Omit to search across all of GitHub.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Maximum number of issues to show.",
    },
    OptionSchema {
        name: "label",
        type_hint: "string",
        required: false,
        default: Some("\"good first issue\""),
        description: "Label to search for. Defaults to the canonical good-first-issue label.",
    },
];

pub struct GithubGoodFirstIssues;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub label: Option<String>,
}

#[async_trait]
impl Fetcher for GithubGoodFirstIssues {
    fn name(&self) -> &str {
        "github_good_first_issues"
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
                "ratatui/ratatui#999 improve docs for Paragraph",
                "tokio-rs/console#123 add quickstart section",
            ]),
            Shape::Entries => samples::entries(&[
                ("ratatui #999", "improve docs for Paragraph"),
                ("console #123", "add quickstart section"),
            ]),
            Shape::Timeline => samples::timeline(&[
                (
                    1_774_000_000,
                    "ratatui/ratatui#999",
                    Some("improve docs for Paragraph"),
                ),
                (
                    1_773_500_000,
                    "tokio-rs/console#123",
                    Some("add quickstart section"),
                ),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return Ok(placeholder(&msg)),
        };
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 30);
        let label = opts
            .label
            .as_deref()
            .unwrap_or("good first issue")
            .replace(' ', "+");
        let mut query = format!("label%3A%22{label}%22+is%3Aissue+is%3Aopen");
        let include_repo = opts.repo.is_none();
        if let Some(raw) = opts.repo.as_deref() {
            let slug = RepoSlug::parse(raw)
                .ok_or_else(|| FetchError::Failed(format!("invalid repo option: {raw:?}")))?;
            query.push_str(&format!("+repo%3A{}%2F{}", slug.owner, slug.name));
        }
        let path = format!("/search/issues?q={query}&sort=updated&per_page={limit}");
        let res: SearchResult = rest_get(&path).await?;
        Ok(payload(render_items(
            &res.items,
            ctx.shape.unwrap_or(Shape::TextBlock),
            include_repo,
        )))
    }
}

fn repo_for_key(ctx: &FetchContext) -> String {
    ctx.options
        .as_ref()
        .and_then(|v| v.get("repo"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_default()
}
