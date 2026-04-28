//! `github_action_status` — current CI state for a repo, as a single badge plus optional
//! `Text` summary. Uses the latest workflow run across every workflow.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{BadgeData, Body, Payload, Status, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{RepoSlug, cache_key, parse_options, payload, resolve_repo};

const SHAPES: &[Shape] = &[Shape::Badge, Shape::Text];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "repo",
        type_hint: "\"owner/name\"",
        required: false,
        default: Some("git remote of cwd"),
        description: "Repository to query.",
    },
    OptionSchema {
        name: "branch",
        type_hint: "string",
        required: false,
        default: None,
        description: "Branch to filter the latest run by. Omit for the most recent run on any branch.",
    },
];

pub struct GithubActionStatus;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
}

#[async_trait]
impl Fetcher for GithubActionStatus {
    fn name(&self) -> &str {
        "github_action_status"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "The latest CI workflow run for a repo as a pass/fail badge or short text line. Use `github_action_history` for a series of recent runs rather than the single most recent one."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Badge
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
            Shape::Badge => samples::badge(Status::Ok, "ci passing"),
            Shape::Text => samples::text("main · passing"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let slug = resolve_repo(opts.repo.as_deref())?;
        let mut path = format!(
            "/repos/{}/{}/actions/runs?per_page=1",
            slug.owner, slug.name
        );
        if let Some(branch) = opts.branch.as_deref() {
            path.push_str(&format!("&branch={branch}"));
        }
        let res: RunsResponse = rest_get(&path).await?;
        let Some(run) = res.workflow_runs.into_iter().next() else {
            return Ok(payload(Body::Badge(BadgeData {
                status: Status::Warn,
                label: "no runs".into(),
            })));
        };
        Ok(payload(render_body(
            &run,
            ctx.shape.unwrap_or(Shape::Badge),
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
    status: String,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    head_branch: Option<String>,
}

fn render_body(run: &WorkflowRun, shape: Shape) -> Body {
    let (status, label_word) = classify(run);
    match shape {
        Shape::Text => Body::Text(TextData {
            value: format!(
                "{} · {label_word}",
                run.head_branch.as_deref().unwrap_or("?")
            ),
        }),
        _ => Body::Badge(BadgeData {
            status,
            label: format!(
                "{} · {label_word}",
                run.head_branch.as_deref().unwrap_or("?")
            ),
        }),
    }
}

fn classify(run: &WorkflowRun) -> (Status, &'static str) {
    if run.status == "completed" {
        match run.conclusion.as_deref() {
            Some("success") => (Status::Ok, "passing"),
            Some("failure") | Some("timed_out") | Some("startup_failure") => {
                (Status::Error, "failing")
            }
            Some("cancelled") => (Status::Warn, "cancelled"),
            Some("neutral") | Some("skipped") => (Status::Warn, "skipped"),
            _ => (Status::Warn, "completed"),
        }
    } else {
        (Status::Warn, "running")
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
    use std::time::Duration;

    use super::*;

    fn run(status: &str, conclusion: Option<&str>) -> WorkflowRun {
        WorkflowRun {
            status: status.into(),
            conclusion: conclusion.map(String::from),
            head_branch: Some("main".into()),
        }
    }

    fn run_with_branch(
        status: &str,
        conclusion: Option<&str>,
        head_branch: Option<&str>,
    ) -> WorkflowRun {
        WorkflowRun {
            status: status.into(),
            conclusion: conclusion.map(String::from),
            head_branch: head_branch.map(String::from),
        }
    }

    fn ctx(options: Option<&str>, shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "ci".into(),
            format: Some("compact".into()),
            timeout: Duration::from_secs(1),
            file_format: None,
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            timezone: None,
            locale: None,
        }
    }

    #[test]
    fn options_default_to_none() {
        let opts = Options::default();
        assert!(opts.repo.is_none());
        assert!(opts.branch.is_none());
    }

    #[test]
    fn options_deserialize_repo_and_branch() {
        let raw: toml::Value = toml::from_str("repo = \"foo/bar\"\nbranch = \"main\"").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.repo.as_deref(), Some("foo/bar"));
        assert_eq!(opts.branch.as_deref(), Some("main"));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("repo = \"foo/bar\"\nbogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn fetcher_metadata_and_samples_cover_supported_shapes() {
        let fetcher = GithubActionStatus;
        assert_eq!(fetcher.name(), "github_action_status");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("latest CI workflow run"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::Badge);
        assert_eq!(fetcher.option_schemas().len(), 2);

        let Some(Body::Badge(badge)) = fetcher.sample_body(Shape::Badge) else {
            panic!("expected badge sample");
        };
        assert_eq!(badge.status, Status::Ok);
        assert_eq!(badge.label, "ci passing");

        let Some(Body::Text(text)) = fetcher.sample_body(Shape::Text) else {
            panic!("expected text sample");
        };
        assert_eq!(text.value, "main · passing");
        assert!(fetcher.sample_body(Shape::Entries).is_none());
    }

    #[test]
    fn completed_success_is_ok() {
        assert_eq!(classify(&run("completed", Some("success"))).0, Status::Ok);
    }

    #[test]
    fn completed_failure_is_error() {
        assert_eq!(
            classify(&run("completed", Some("failure"))).0,
            Status::Error
        );
    }

    #[test]
    fn in_progress_is_warn() {
        assert_eq!(classify(&run("in_progress", None)).0, Status::Warn);
    }

    #[test]
    fn completed_timed_out_and_startup_failure_are_errors() {
        ["timed_out", "startup_failure"]
            .into_iter()
            .for_each(|conclusion| {
                assert_eq!(
                    classify(&run("completed", Some(conclusion))),
                    (Status::Error, "failing")
                );
            });
    }

    #[test]
    fn completed_cancelled_and_skipped_stay_warn() {
        ["cancelled", "neutral", "skipped"]
            .into_iter()
            .map(|conclusion| (conclusion, classify(&run("completed", Some(conclusion)))))
            .for_each(|(conclusion, actual)| {
                let expected = if conclusion == "cancelled" {
                    (Status::Warn, "cancelled")
                } else {
                    (Status::Warn, "skipped")
                };
                assert_eq!(actual, expected);
            });
    }

    #[test]
    fn completed_unknown_conclusion_falls_back_to_completed() {
        assert_eq!(
            classify(&run("completed", Some("action_required"))),
            (Status::Warn, "completed")
        );
        assert_eq!(
            classify(&run("completed", None)),
            (Status::Warn, "completed")
        );
    }

    #[test]
    fn render_badge_uses_branch_and_classified_label() {
        let body = render_body(&run("completed", Some("cancelled")), Shape::Badge);
        let Body::Badge(badge) = body else {
            panic!("expected badge");
        };
        assert_eq!(badge.status, Status::Warn);
        assert_eq!(badge.label, "main · cancelled");
    }

    #[test]
    fn render_text_falls_back_to_question_mark_for_missing_branch() {
        let body = render_body(
            &run_with_branch("completed", Some("success"), None),
            Shape::Text,
        );
        let Body::Text(text) = body else {
            panic!("expected text");
        };
        assert_eq!(text.value, "? · passing");
    }

    #[test]
    fn repo_for_key_prefers_explicit_repo_option() {
        assert_eq!(
            repo_for_key(&ctx(Some("repo = \"foo/bar\""), Some(Shape::Badge))),
            "foo/bar"
        );
    }

    #[test]
    fn repo_for_key_falls_back_to_resolved_repo() {
        assert_eq!(
            repo_for_key(&ctx(None, Some(Shape::Badge))),
            resolve_repo(None).unwrap().as_path()
        );
    }

    #[test]
    fn cache_key_changes_with_repo_option() {
        let fetcher = GithubActionStatus;
        let a = fetcher.cache_key(&ctx(Some("repo = \"foo/bar\""), Some(Shape::Badge)));
        let b = fetcher.cache_key(&ctx(Some("repo = \"foo/baz\""), Some(Shape::Badge)));
        assert_ne!(a, b);
    }
}
