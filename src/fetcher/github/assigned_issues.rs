//! `github_assigned_issues` — open issues assigned to the authenticated user.
//! Uses `/search/issues?q=is:issue+is:open+assignee:@me`. `Safe` (fixed query).

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{cache_key, parse_options, payload};
use super::items::{SearchResult, render_items};

const SHAPES: &[Shape] = &[Shape::TextBlock, Shape::Entries, Shape::Timeline];
const DEFAULT_LIMIT: u32 = 10;

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "limit",
    type_hint: "integer (1..=30)",
    required: false,
    default: Some("10"),
    description: "Maximum number of issues to show.",
}];

pub struct GithubAssignedIssues;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for GithubAssignedIssues {
    fn name(&self) -> &str {
        "github_assigned_issues"
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
        cache_key(self.name(), ctx, "")
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::TextBlock => samples::text_block(&[
                "unhappychoice/splashboard#41 meta: widget catalog & roadmap",
                "unhappychoice/splashboard#17 theme system",
            ]),
            Shape::Entries => samples::entries(&[
                ("splashboard #41", "meta: widget catalog"),
                ("splashboard #17", "theme system"),
            ]),
            Shape::Timeline => samples::timeline(&[
                (
                    1_774_000_000,
                    "splashboard#41",
                    Some("meta: widget catalog"),
                ),
                (1_773_500_000, "splashboard#17", Some("theme system")),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 30);
        let path = format!(
            "/search/issues?q=is%3Aissue+is%3Aopen+assignee%3A%40me&per_page={limit}&sort=updated"
        );
        let res: SearchResult = rest_get(&path).await?;
        Ok(payload(render_items(
            &res.items,
            ctx.shape.unwrap_or(Shape::TextBlock),
            true,
        )))
    }
}
