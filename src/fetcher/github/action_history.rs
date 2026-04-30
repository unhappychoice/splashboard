//! `github_action_history` — recent workflow runs mapped either to a pass/fail number series
//! (`NumberSeries`, 1 = success / 0 = anything else — oldest first, feeds `sparkline`) or a
//! timeline of the most recent N runs.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    Body, NumberSeriesData, Payload, PointSeries, PointSeriesData, Status, TimelineData,
    TimelineEvent,
};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{RepoSlug, cache_key, parse_options, parse_timestamp, payload, resolve_repo};

const SHAPES: &[Shape] = &[Shape::NumberSeries, Shape::Timeline, Shape::PointSeries];
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
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Recent CI workflow runs as a pass/fail sparkline (`NumberSeries`), a timeline of the last N runs, or a duration scatter plot of `(run_number, seconds)` for spotting CI slowdowns (`PointSeries`). Use `github_action_status` instead for just the current main-branch state."
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
            Shape::PointSeries => Body::PointSeries(PointSeriesData {
                series: vec![PointSeries {
                    name: "ci duration (s)".into(),
                    points: vec![
                        (4233.0, 142.0),
                        (4234.0, 168.0),
                        (4235.0, 138.0),
                        (4236.0, 220.0),
                        (4237.0, 145.0),
                    ],
                }],
            }),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let slug = resolve_repo(opts.repo.as_deref())?;
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
    created_at: String,
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
        Shape::PointSeries => Body::PointSeries(PointSeriesData {
            series: vec![PointSeries {
                name: "ci duration (s)".into(),
                points: runs
                    .iter()
                    .rev()
                    .filter_map(|r| duration_seconds(r).map(|s| (r.run_number as f64, s)))
                    .collect(),
            }],
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

fn duration_seconds(r: &WorkflowRun) -> Option<f64> {
    let start = parse_timestamp(&r.created_at);
    let end = parse_timestamp(&r.updated_at);
    if start <= 0 || end <= 0 || end < start {
        return None;
    }
    Some((end - start) as f64)
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

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::time::Duration;

    use super::*;

    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        restore: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn set(pairs: &[(&'static str, Option<&str>)]) -> Self {
            let lock = crate::paths::TEST_ENV_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let restore = pairs
                .iter()
                .map(|(key, value)| {
                    let previous = std::env::var(key).ok();
                    match value {
                        Some(value) => unsafe { std::env::set_var(key, value) },
                        None => unsafe { std::env::remove_var(key) },
                    }
                    (*key, previous)
                })
                .collect();
            Self {
                _lock: lock,
                restore,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            self.restore.iter().for_each(|(key, value)| match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            });
        }
    }

    fn ctx(options: Option<&str>, shape: Option<Shape>, format: Option<&str>) -> FetchContext {
        FetchContext {
            widget_id: "action-history".into(),
            format: format.map(str::to_string),
            timeout: Duration::from_secs(1),
            file_format: None,
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            timezone: None,
            locale: None,
        }
    }

    fn run_async<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn run(num: u64, created: &str, updated: &str, conclusion: Option<&str>) -> WorkflowRun {
        WorkflowRun {
            run_number: num,
            status: "completed".into(),
            conclusion: conclusion.map(String::from),
            head_branch: Some("main".into()),
            created_at: created.into(),
            updated_at: updated.into(),
        }
    }

    fn workflow_run(
        num: u64,
        status: &str,
        conclusion: Option<&str>,
        branch: Option<&str>,
    ) -> WorkflowRun {
        WorkflowRun {
            run_number: num,
            status: status.into(),
            conclusion: conclusion.map(String::from),
            head_branch: branch.map(String::from),
            created_at: "2026-04-22T10:00:00Z".into(),
            updated_at: "2026-04-22T10:02:30Z".into(),
        }
    }

    #[test]
    fn options_default_to_none() {
        let opts = Options::default();
        assert!(opts.repo.is_none());
        assert!(opts.limit.is_none());
        assert!(opts.branch.is_none());
    }

    #[test]
    fn options_deserialize_repo_limit_and_branch() {
        let raw: toml::Value =
            toml::from_str("repo = \"owner/name\"\nlimit = 7\nbranch = \"main\"").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.repo.as_deref(), Some("owner/name"));
        assert_eq!(opts.limit, Some(7));
        assert_eq!(opts.branch.as_deref(), Some("main"));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value =
            toml::from_str("repo = \"owner/name\"\nlimit = 7\nbogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn fetcher_metadata_and_samples_cover_supported_shapes() {
        let fetcher = GithubActionHistory;
        assert_eq!(fetcher.name(), "github_action_history");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("Recent CI workflow runs"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::NumberSeries);

        let names = fetcher
            .option_schemas()
            .iter()
            .map(|schema| schema.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["repo", "limit", "branch"]);

        let Some(Body::NumberSeries(number_series)) = fetcher.sample_body(Shape::NumberSeries)
        else {
            panic!("expected number series sample");
        };
        assert_eq!(number_series.values[0], 1);
        assert_eq!(number_series.values[2], 0);

        let Some(Body::Timeline(timeline)) = fetcher.sample_body(Shape::Timeline) else {
            panic!("expected timeline sample");
        };
        assert_eq!(timeline.events[0].title, "#4235 main");
        assert_eq!(timeline.events[1].detail.as_deref(), Some("failing"));

        let Some(Body::PointSeries(point_series)) = fetcher.sample_body(Shape::PointSeries) else {
            panic!("expected point series sample");
        };
        assert_eq!(point_series.series.len(), 1);
        assert_eq!(point_series.series[0].name, "ci duration (s)");
        assert_eq!(point_series.series[0].points[3], (4236.0, 220.0));
        assert!(fetcher.sample_body(Shape::Text).is_none());
    }

    #[test]
    fn cache_key_is_stable_and_changes_with_repo_shape_and_format() {
        let fetcher = GithubActionHistory;
        let base = ctx(
            Some("repo = \"owner/name\"\nlimit = 5"),
            Some(Shape::NumberSeries),
            Some("compact"),
        );
        let same = ctx(
            Some("repo = \"owner/name\"\nlimit = 5"),
            Some(Shape::NumberSeries),
            Some("compact"),
        );
        let other_repo = ctx(
            Some("repo = \"other/name\"\nlimit = 5"),
            Some(Shape::NumberSeries),
            Some("compact"),
        );
        let other_shape = ctx(
            Some("repo = \"owner/name\"\nlimit = 5"),
            Some(Shape::Timeline),
            Some("compact"),
        );
        let other_format = ctx(
            Some("repo = \"owner/name\"\nlimit = 5"),
            Some(Shape::NumberSeries),
            Some("verbose"),
        );

        assert_eq!(fetcher.cache_key(&base), fetcher.cache_key(&same));
        assert_ne!(fetcher.cache_key(&base), fetcher.cache_key(&other_repo));
        assert_ne!(fetcher.cache_key(&base), fetcher.cache_key(&other_shape));
        assert_ne!(fetcher.cache_key(&base), fetcher.cache_key(&other_format));
    }

    #[test]
    fn duration_seconds_returns_delta_when_both_timestamps_parse() {
        let r = run(
            10,
            "2026-04-22T10:00:00Z",
            "2026-04-22T10:02:30Z",
            Some("success"),
        );
        assert_eq!(duration_seconds(&r), Some(150.0));
    }

    #[test]
    fn duration_seconds_returns_none_for_missing_or_inverted_timestamps() {
        let bad = run(11, "", "2026-04-22T10:02:30Z", None);
        assert!(duration_seconds(&bad).is_none());
        let inverted = run(12, "2026-04-22T10:02:30Z", "2026-04-22T10:00:00Z", None);
        assert!(duration_seconds(&inverted).is_none());
    }

    #[test]
    fn status_and_label_cover_success_error_and_in_progress_states() {
        let success = workflow_run(7, "completed", Some("success"), Some("main"));
        let failure = workflow_run(8, "completed", Some("startup_failure"), Some("release"));
        let neutral = workflow_run(9, "completed", None, Some("main"));
        let queued = workflow_run(10, "queued", Some("failure"), None);

        assert_eq!(status_of(&success), Status::Ok);
        assert_eq!(label(&success), "success");
        assert_eq!(status_of(&failure), Status::Error);
        assert_eq!(label(&failure), "startup_failure");
        assert_eq!(status_of(&neutral), Status::Warn);
        assert_eq!(label(&neutral), "completed");
        assert_eq!(status_of(&queued), Status::Warn);
        assert_eq!(label(&queued), "queued");
    }

    #[test]
    fn number_series_body_reverses_runs_and_marks_only_successes() {
        let runs = vec![
            run(
                3,
                "2026-04-22T10:04:00Z",
                "2026-04-22T10:05:00Z",
                Some("success"),
            ),
            run(
                2,
                "2026-04-22T10:02:00Z",
                "2026-04-22T10:03:00Z",
                Some("failure"),
            ),
            run(1, "2026-04-22T10:00:00Z", "2026-04-22T10:01:00Z", None),
        ];
        let Body::NumberSeries(data) = render_body(&runs, Shape::NumberSeries) else {
            panic!("expected number series");
        };
        assert_eq!(data.values, vec![0, 0, 1]);
    }

    #[test]
    fn timeline_body_uses_branch_fallback_label_and_status() {
        let runs = vec![
            workflow_run(10, "queued", Some("failure"), None),
            workflow_run(9, "completed", None, Some("main")),
            workflow_run(8, "completed", Some("timed_out"), Some("release")),
        ];
        let Body::Timeline(data) = render_body(&runs, Shape::Timeline) else {
            panic!("expected timeline");
        };
        assert_eq!(data.events.len(), 3);
        assert_eq!(data.events[0].title, "#10 ?");
        assert_eq!(data.events[0].detail.as_deref(), Some("queued"));
        assert_eq!(data.events[0].status, Some(Status::Warn));
        assert_eq!(data.events[1].detail.as_deref(), Some("completed"));
        assert_eq!(data.events[1].status, Some(Status::Warn));
        assert_eq!(data.events[2].title, "#8 release");
        assert_eq!(data.events[2].status, Some(Status::Error));
        assert_eq!(data.events[2].timestamp, 1_776_852_150);
    }

    #[test]
    fn point_series_body_filters_out_runs_without_duration() {
        let runs = vec![
            run(
                1,
                "2026-04-22T10:00:00Z",
                "2026-04-22T10:01:40Z",
                Some("success"),
            ),
            run(2, "", "", None),
            run(
                3,
                "2026-04-22T10:00:00Z",
                "2026-04-22T10:03:00Z",
                Some("success"),
            ),
        ];
        let body = render_body(&runs, Shape::PointSeries);
        let Body::PointSeries(d) = body else {
            panic!("expected point series");
        };
        assert_eq!(d.series.len(), 1);
        let pts = &d.series[0].points;
        assert_eq!(pts.len(), 2);
        // Newest-first input is reversed → expect oldest first by run_number.
        assert_eq!(pts[0].0, 3.0);
        assert_eq!(pts[0].1, 180.0);
        assert_eq!(pts[1].0, 1.0);
        assert_eq!(pts[1].1, 100.0);
    }

    #[test]
    fn fetch_rejects_invalid_options_before_repo_or_auth_lookup() {
        let fetcher = GithubActionHistory;
        let err =
            run_async(fetcher.fetch(&ctx(Some("limit = \"many\""), Some(Shape::Timeline), None)))
                .expect_err("invalid options should fail");
        let FetchError::Failed(message) = err else {
            panic!("expected fetch failure");
        };
        assert!(message.contains("invalid options"));
    }

    #[test]
    fn fetch_rejects_invalid_repo_before_auth_lookup() {
        let fetcher = GithubActionHistory;
        let err = run_async(fetcher.fetch(&ctx(
            Some("repo = \"broken\"\nbranch = \"main\""),
            Some(Shape::PointSeries),
            None,
        )))
        .expect_err("invalid repo should fail");
        let FetchError::Failed(message) = err else {
            panic!("expected fetch failure");
        };
        assert_eq!(message, "invalid repo option: \"broken\"");
    }

    #[test]
    fn fetch_without_token_surfaces_auth_error_after_processing_repo_branch_and_limit() {
        let _guard = EnvGuard::set(&[("GH_TOKEN", None), ("GITHUB_TOKEN", None)]);
        let fetcher = GithubActionHistory;
        let err = run_async(fetcher.fetch(&ctx(
            Some("repo = \"owner/name\"\nlimit = 500\nbranch = \"release\""),
            Some(Shape::PointSeries),
            Some("compact"),
        )))
        .expect_err("missing token should fail");
        let FetchError::Failed(message) = err else {
            panic!("expected fetch failure");
        };
        assert_eq!(message, "GH_TOKEN / GITHUB_TOKEN not set");
    }
}
