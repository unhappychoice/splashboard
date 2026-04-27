//! `github_review_requests` — open PRs where the authenticated user is a requested reviewer.
//! Uses `/search/issues?q=is:pr+is:open+review-requested:@me`. `Safe` (fixed query).

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

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::TextBlock,
    Shape::Entries,
    Shape::Timeline,
];
const DEFAULT_LIMIT: u32 = 10;

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "limit",
    type_hint: "integer (1..=30)",
    required: false,
    default: Some("10"),
    description: "Maximum number of PRs to show.",
}];

pub struct GithubReviewRequests;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for GithubReviewRequests {
    fn name(&self) -> &str {
        "github_review_requests"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Open pull requests where the authenticated user is a requested reviewer, across every repo. The other side of `github_my_prs` for the personal queue."
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
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "ratatui/ratatui#1234 feat: add pie chart",
                    Some("https://github.com/ratatui/ratatui/pull/1234"),
                ),
                (
                    "tokio-rs/tokio#5678 fix: race in spawn",
                    Some("https://github.com/tokio-rs/tokio/pull/5678"),
                ),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "ratatui/ratatui#1234 feat: add pie chart",
                "tokio-rs/tokio#5678 fix: race in spawn",
            ]),
            Shape::Entries => samples::entries(&[
                ("ratatui #1234", "feat: add pie chart"),
                ("tokio #5678", "fix: race in spawn"),
            ]),
            Shape::Timeline => samples::timeline(&[
                (
                    1_774_000_000,
                    "ratatui/ratatui#1234",
                    Some("feat: add pie chart"),
                ),
                (
                    1_773_800_000,
                    "tokio-rs/tokio#5678",
                    Some("fix: race in spawn"),
                ),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 30);
        let path = format!(
            "/search/issues?q=is%3Apr+is%3Aopen+review-requested%3A%40me&per_page={limit}&sort=updated"
        );
        let res: SearchResult = rest_get(&path).await?;
        Ok(payload(render_items(
            &res.items,
            ctx.shape.unwrap_or(Shape::LinkedTextBlock),
            true,
        )))
    }
}
