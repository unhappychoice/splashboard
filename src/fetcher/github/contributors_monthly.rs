//! `github_contributors_monthly` — who's shipping this month, ranked by commit count. Uses the
//! stats endpoint, which returns 202 while GitHub computes the aggregate; we surface a
//! transient placeholder rather than blocking. `Network`.

use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Bar, BarsData, Body, EntriesData, Entry, Payload};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::{http, resolve_token};
use super::common::{RepoSlug, cache_key, parse_options, payload, placeholder, resolve_repo};

const SHAPES: &[Shape] = &[Shape::Bars, Shape::Entries];
const DEFAULT_LIMIT: u32 = 10;

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
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Maximum number of contributors to show.",
    },
    OptionSchema {
        name: "days",
        type_hint: "integer (7..=365)",
        required: false,
        default: Some("30"),
        description: "Window size (in days) the monthly total is summed over.",
    },
];

pub struct GithubContributorsMonthly;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub days: Option<u32>,
}

#[async_trait]
impl Fetcher for GithubContributorsMonthly {
    fn name(&self) -> &str {
        "github_contributors_monthly"
    }
    fn safety(&self) -> Safety {
        Safety::Network
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Bars
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
            Shape::Bars => samples::bars(&[("alice", 42), ("bob", 27), ("charlie", 11)]),
            Shape::Entries => samples::entries(&[
                ("alice", "42"),
                ("bob", "27"),
                ("charlie", "11"),
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
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 30) as usize;
        let days = opts.days.unwrap_or(30).clamp(7, 365) as i64;
        match fetch_stats(&slug).await? {
            StatsOutcome::Computing => Ok(placeholder(
                "github: stats warming up (refresh in a minute)",
            )),
            StatsOutcome::Ready(stats) => {
                let rows = top_contributors(&stats, days, limit);
                Ok(payload(render_body(&rows, ctx.shape.unwrap_or(Shape::Bars))))
            }
        }
    }
}

enum StatsOutcome {
    Computing,
    Ready(Vec<Contributor>),
}

#[derive(Debug, Deserialize)]
struct Contributor {
    author: Option<Author>,
    #[serde(default)]
    weeks: Vec<Week>,
}

#[derive(Debug, Deserialize)]
struct Author {
    login: String,
}

#[derive(Debug, Deserialize)]
struct Week {
    /// Unix seconds marking the start of the week (Sunday).
    w: i64,
    /// Commits during the week.
    #[serde(default)]
    c: u64,
}

/// `/stats/contributors` responds with 202 the first time while GitHub computes the aggregate.
/// Returning `StatsOutcome::Computing` lets the caller render a warming-up placeholder instead
/// of blocking — the next refresh picks up the ready data.
async fn fetch_stats(slug: &RepoSlug) -> Result<StatsOutcome, FetchError> {
    let token = resolve_token()?;
    let url = format!(
        "https://api.github.com/repos/{}/{}/stats/contributors",
        slug.owner, slug.name
    );
    let res = http()
        .get(&url)
        .bearer_auth(&token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("github stats: {e}")))?;
    let status = res.status();
    if status.as_u16() == 202 {
        return Ok(StatsOutcome::Computing);
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("github stats body: {e}")))?;
    if !status.is_success() {
        return Err(FetchError::Failed(format!("github stats {status}")));
    }
    let list: Vec<Contributor> = serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("github stats parse: {e}")))?;
    Ok(StatsOutcome::Ready(list))
}

fn top_contributors(stats: &[Contributor], days: i64, limit: usize) -> Vec<(String, u64)> {
    let cutoff = Utc::now().timestamp() - Duration::days(days).num_seconds();
    let mut rows: Vec<(String, u64)> = stats
        .iter()
        .filter_map(|c| {
            let login = c.author.as_ref()?.login.clone();
            let commits: u64 = c.weeks.iter().filter(|w| w.w >= cutoff).map(|w| w.c).sum();
            (commits > 0).then_some((login, commits))
        })
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1));
    rows.truncate(limit);
    rows
}

fn render_body(rows: &[(String, u64)], shape: Shape) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: rows
                .iter()
                .map(|(n, c)| Entry {
                    key: n.clone(),
                    value: Some(c.to_string()),
                    status: None,
                })
                .collect(),
        }),
        _ => Body::Bars(BarsData {
            bars: rows
                .iter()
                .map(|(n, c)| Bar {
                    label: n.clone(),
                    value: *c,
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
