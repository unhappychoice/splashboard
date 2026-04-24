//! `github_recent_releases` — latest releases for a repo.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    Body, EntriesData, Entry, Payload, TextBlockData, TimelineData, TimelineEvent,
};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{RepoSlug, cache_key, parse_options, parse_timestamp, payload, resolve_repo};

const SHAPES: &[Shape] = &[Shape::TextBlock, Shape::Entries, Shape::Timeline];
const DEFAULT_LIMIT: u32 = 5;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "repo",
        type_hint: "\"owner/name\"",
        required: false,
        default: Some("git remote of cwd"),
        description: "Repository to query.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=20)",
        required: false,
        default: Some("5"),
        description: "Maximum number of releases to show.",
    },
];

pub struct GithubRecentReleases;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for GithubRecentReleases {
    fn name(&self) -> &str {
        "github_recent_releases"
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
            Shape::TextBlock => samples::text_block(&["v0.3.0  2026-04-10", "v0.2.1  2026-03-05"]),
            Shape::Entries => {
                samples::entries(&[("v0.3.0", "2026-04-10"), ("v0.2.1", "2026-03-05")])
            }
            Shape::Timeline => samples::timeline(&[
                (1_776_000_000, "v0.3.0", Some("latest")),
                (1_773_800_000, "v0.2.1", None),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let slug = resolve_repo(opts.repo.as_deref())?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 20);
        let path = format!(
            "/repos/{}/{}/releases?per_page={limit}",
            slug.owner, slug.name
        );
        let items: Vec<Release> = rest_get(&path).await?;
        Ok(payload(render_body(
            &items,
            ctx.shape.unwrap_or(Shape::TextBlock),
        )))
    }
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
}

fn render_body(items: &[Release], shape: Shape) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: items
                .iter()
                .map(|r| Entry {
                    key: r.tag_name.clone(),
                    value: Some(
                        r.published_at
                            .clone()
                            .unwrap_or_default()
                            .split('T')
                            .next()
                            .unwrap_or("")
                            .into(),
                    ),
                    status: None,
                })
                .collect(),
        }),
        Shape::Timeline => Body::Timeline(TimelineData {
            events: items
                .iter()
                .map(|r| TimelineEvent {
                    timestamp: r.published_at.as_deref().map(parse_timestamp).unwrap_or(0),
                    title: r.tag_name.clone(),
                    detail: r.name.clone(),
                    status: None,
                })
                .collect(),
        }),
        _ => Body::TextBlock(TextBlockData {
            lines: items
                .iter()
                .map(|r| {
                    let date = r
                        .published_at
                        .as_deref()
                        .and_then(|d| d.split('T').next())
                        .unwrap_or("");
                    format!("{}  {date}", r.tag_name)
                })
                .collect(),
        }),
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
