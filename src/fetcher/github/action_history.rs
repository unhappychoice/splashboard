//! `github_action_history` — recent workflow runs mapped either to a pass/fail number series
//! (`NumberSeries`, 1 = success / 0 = anything else — oldest first, feeds `sparkline`) or a
//! timeline of the most recent N runs. `Network`.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    Body, NumberSeriesData, Payload, Status, TimelineData, TimelineEvent,
};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{
    RepoSlug, cache_key, parse_options, parse_timestamp, payload, placeholder, resolve_repo,
};

const SHAPES: &[Shape] = &[Shape::NumberSeries, Shape::Timeline];
const DEFAULT_LIMIT: u32 = 30;

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
        type_hint: "integer (1..=100)",
        required: false,
        default: Some("30"),
        description: "Number of recent runs to return.",
    },
    OptionSchema {
        name: "branch",
        type_hint: "string",
        required: false,
        default: None,
        description: "Branch filter. Omit for runs across all branches.",
    },
];

pub struct GithubActionHistory;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub branch: Option<String>,
}

#[async_trait]
impl Fetcher for GithubActionHistory {
    fn name(&self) -> &str {
        "github_action_history"
    }
    fn safety(&self) -> Safety {
        Safety::Network
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
            Shape::NumberSeries => {
                samples::number_series(&[1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 1, 0, 1, 1])
            }
            Shape::Timeline => samples::timeline(&[
                (1_776_000_000, "#4235 main", Some("passing")),
                (1_775_800_000, "#4234 feat/a", Some("failing")),
                (1_775_600_000, "#4233 main", Some("passing")),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return Ok(placeholder(&msg)),
        };
        let slug = match resolve_repo(opts.repo.as_deref()) {
            Ok(s) => s,
            Err(e) => return Ok(placeholder(&e.to_string())),
        };
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 100);
        let mut path = format!(
            "/repos/{}/{}/actions/runs?per_page={limit}",
            slug.owner, slug.name
        );
        if let Some(branch) = opts.branch.as_deref() {
            path.push_str(&format!("&branch={branch}"));
        }
        let res: RunsResponse = rest_get(&path).await?;
        Ok(payload(render_body(
            &res.workflow_runs,
            ctx.shape.unwrap_or(Shape::NumberSeries),
        )))
    }
}

#[derive(Debug, Deserialize)]
struct RunsResponse {
    #[serde(default)]
    workflow_runs: Vec<WorkflowRun>,
}

#[derive(Debug, Deserialize)]
struct WorkflowRun {
    #[serde(default)]
    run_number: u64,
    #[serde(default)]
    status: String,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    head_branch: Option<String>,
    #[serde(default)]
    updated_at: String,
}

fn render_body(runs: &[WorkflowRun], shape: Shape) -> Body {
    match shape {
        Shape::Timeline => Body::Timeline(TimelineData {
            events: runs
                .iter()
                .map(|r| TimelineEvent {
                    timestamp: parse_timestamp(&r.updated_at),
                    title: format!(
                        "#{} {}",
                        r.run_number,
                        r.head_branch.as_deref().unwrap_or("?")
                    ),
                    detail: Some(label(r)),
                    status: Some(status_of(r)),
                })
                .collect(),
        }),
        _ => Body::NumberSeries(NumberSeriesData {
            values: runs
                .iter()
                .rev()
                .map(|r| {
                    if matches!(r.conclusion.as_deref(), Some("success")) {
                        1
                    } else {
                        0
                    }
                })
                .collect(),
        }),
    }
}

fn status_of(r: &WorkflowRun) -> Status {
    if r.status == "completed" {
        match r.conclusion.as_deref() {
            Some("success") => Status::Ok,
            Some("failure") | Some("timed_out") | Some("startup_failure") => Status::Error,
            _ => Status::Warn,
        }
    } else {
        Status::Warn
    }
}

fn label(r: &WorkflowRun) -> String {
    if r.status == "completed" {
        r.conclusion.clone().unwrap_or_else(|| "completed".into())
    } else {
        r.status.clone()
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
