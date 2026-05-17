//! `gitlab_pipeline_status` — current pipeline state for a GitLab project as a single badge,
//! optional text line, or `Entries` rollup (status / branch / sha / duration). Uses
//! `/api/v4/projects/{id}/pipelines?per_page=1[&ref=<branch>]`.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{BadgeData, Body, EntriesData, Entry, Payload, Status, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::{default_host, rest_get};
use super::common::{ProjectPath, cache_key, parse_options, payload, resolve_project};
use super::my_mrs::resolve_host;

const SHAPES: &[Shape] = &[Shape::Badge, Shape::Text, Shape::Entries];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "host",
        type_hint: "hostname",
        required: false,
        default: Some("gitlab.com"),
        description: "GitLab instance host. Use this for self-hosted GitLab.",
    },
    OptionSchema {
        name: "project",
        type_hint: "\"group/name\" or \"group/sub/name\"",
        required: false,
        default: Some("git remote of cwd"),
        description: "Project to query. Falls back to the current directory's GitLab remote.",
    },
    OptionSchema {
        name: "branch",
        type_hint: "string",
        required: false,
        default: None,
        description: "Branch (ref) to filter the latest pipeline by. Omit for the most recent pipeline on any branch.",
    },
];

pub struct GitlabPipelineStatus;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
}

#[async_trait]
impl Fetcher for GitlabPipelineStatus {
    fn name(&self) -> &str {
        "gitlab_pipeline_status"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Latest CI pipeline run for a GitLab project as a pass/fail badge, short text line, or status rollup."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 5
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
        cache_key(self.name(), ctx, &cache_extra(ctx))
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Badge => samples::badge(Status::Ok, "main · passing"),
            Shape::Text => samples::text("main · passing"),
            Shape::Entries => Body::Entries(EntriesData {
                items: vec![
                    entry("status", Some(Status::Ok), "passing"),
                    entry("branch", None, "main"),
                    entry("sha", None, "abc123de"),
                    entry("duration", None, "3m 12s"),
                ],
            }),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let host = resolve_host(opts.host.as_deref())?;
        let project = resolve_project(opts.project.as_deref(), &host)?;
        let mut path = format!("/projects/{}/pipelines?per_page=1", project.encoded());
        if let Some(branch) = opts.branch.as_deref() {
            path.push_str(&format!("&ref={branch}"));
        }
        let pipelines: Vec<Pipeline> = rest_get(&host, &path).await?;
        let shape = ctx.shape.unwrap_or(Shape::Badge);
        let Some(latest) = pipelines.into_iter().next() else {
            return Ok(payload(Body::Badge(BadgeData {
                status: Status::Warn,
                label: "no runs".into(),
            })));
        };
        Ok(payload(render_body(&latest, shape)))
    }
}

#[derive(Debug, Deserialize)]
struct Pipeline {
    #[serde(default)]
    status: String,
    #[serde(default, rename = "ref")]
    branch: Option<String>,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    duration: Option<u64>,
}

fn render_body(p: &Pipeline, shape: Shape) -> Body {
    let (status, label_word) = classify(&p.status);
    let branch = p.branch.as_deref().unwrap_or("?");
    match shape {
        Shape::Text => Body::Text(TextData {
            value: format!("{branch} · {label_word}"),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: vec![
                entry("status", Some(status), label_word),
                entry("branch", None, branch),
                entry("sha", None, &short_sha(p.sha.as_deref())),
                entry("duration", None, &format_duration(p.duration)),
            ],
        }),
        _ => Body::Badge(BadgeData {
            status,
            label: format!("{branch} · {label_word}"),
        }),
    }
}

fn classify(raw: &str) -> (Status, &'static str) {
    match raw {
        "success" => (Status::Ok, "passing"),
        "failed" => (Status::Error, "failing"),
        "canceled" => (Status::Warn, "cancelled"),
        "skipped" => (Status::Warn, "skipped"),
        "manual" => (Status::Warn, "manual"),
        "running" | "pending" | "created" | "waiting_for_resource" | "preparing" => {
            (Status::Warn, "running")
        }
        _ => (Status::Warn, "unknown"),
    }
}

fn entry(key: &str, status: Option<Status>, value: &str) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value.into()),
        status,
    }
}

fn short_sha(sha: Option<&str>) -> String {
    sha.unwrap_or("?").chars().take(8).collect::<String>()
}

fn format_duration(seconds: Option<u64>) -> String {
    let Some(s) = seconds else {
        return "?".into();
    };
    let m = s / 60;
    let r = s % 60;
    if m == 0 {
        format!("{r}s")
    } else {
        format!("{m}m {r:02}s")
    }
}

fn cache_extra(ctx: &FetchContext) -> String {
    let host = ctx
        .options
        .as_ref()
        .and_then(|v| v.get("host"))
        .and_then(|v| v.as_str())
        .unwrap_or(default_host())
        .to_string();
    let project = ctx
        .options
        .as_ref()
        .and_then(|v| v.get("project"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| {
            resolve_project(None, &host)
                .ok()
                .map(|p: ProjectPath| p.as_str().to_string())
        })
        .unwrap_or_default();
    let branch = ctx
        .options
        .as_ref()
        .and_then(|v| v.get("branch"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    format!("{host}|{project}|{branch}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pipeline(status: &str) -> Pipeline {
        Pipeline {
            status: status.into(),
            branch: Some("main".into()),
            sha: Some("abc123def456".into()),
            duration: Some(192),
        }
    }

    #[test]
    fn options_deserialize_all_fields() {
        let opts: Options =
            toml::from_str("host = \"git.example.org\"\nproject = \"g/p\"\nbranch = \"main\"")
                .unwrap();
        assert_eq!(opts.host.as_deref(), Some("git.example.org"));
        assert_eq!(opts.project.as_deref(), Some("g/p"));
        assert_eq!(opts.branch.as_deref(), Some("main"));
        assert!(toml::from_str::<Options>("bogus = 1").is_err());
    }

    #[test]
    fn shapes_table_lists_the_three_supported_variants() {
        let fetcher = GitlabPipelineStatus;
        assert_eq!(
            fetcher.shapes(),
            &[Shape::Badge, Shape::Text, Shape::Entries]
        );
        assert_eq!(fetcher.default_shape(), Shape::Badge);
    }

    #[test]
    fn classify_maps_each_documented_state() {
        for (raw, expected) in [
            ("success", Status::Ok),
            ("failed", Status::Error),
            ("canceled", Status::Warn),
            ("skipped", Status::Warn),
            ("manual", Status::Warn),
            ("running", Status::Warn),
            ("pending", Status::Warn),
            ("created", Status::Warn),
            ("waiting_for_resource", Status::Warn),
            ("preparing", Status::Warn),
            ("totally_unknown", Status::Warn),
        ] {
            assert_eq!(classify(raw).0, expected, "raw={raw}");
        }
    }

    #[test]
    fn render_badge_uses_branch_and_label_word() {
        let Body::Badge(b) = render_body(&pipeline("success"), Shape::Badge) else {
            panic!("expected badge");
        };
        assert_eq!(b.status, Status::Ok);
        assert_eq!(b.label, "main · passing");
    }

    #[test]
    fn render_text_falls_back_to_question_mark_for_missing_branch() {
        let mut p = pipeline("success");
        p.branch = None;
        let Body::Text(t) = render_body(&p, Shape::Text) else {
            panic!("expected text");
        };
        assert_eq!(t.value, "? · passing");
    }

    #[test]
    fn render_entries_carries_status_branch_sha_duration() {
        let Body::Entries(e) = render_body(&pipeline("failed"), Shape::Entries) else {
            panic!("expected entries");
        };
        assert_eq!(e.items[0].key, "status");
        assert_eq!(e.items[0].status, Some(Status::Error));
        assert_eq!(e.items[0].value.as_deref(), Some("failing"));
        assert_eq!(e.items[2].value.as_deref(), Some("abc123de"));
        assert_eq!(e.items[3].value.as_deref(), Some("3m 12s"));
    }

    #[test]
    fn short_sha_truncates_long_hexes_and_falls_back() {
        assert_eq!(short_sha(Some("abcdef1234567890")), "abcdef12");
        assert_eq!(short_sha(None), "?");
    }

    #[test]
    fn format_duration_renders_seconds_only_or_minutes_seconds() {
        assert_eq!(format_duration(Some(45)), "45s");
        assert_eq!(format_duration(Some(192)), "3m 12s");
        assert_eq!(format_duration(Some(60)), "1m 00s");
        assert_eq!(format_duration(None), "?");
    }

    #[test]
    fn sample_entries_carries_the_four_rollup_rows() {
        let Some(Body::Entries(e)) = GitlabPipelineStatus.sample_body(Shape::Entries) else {
            panic!("expected entries");
        };
        let keys: Vec<&str> = e.items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, vec!["status", "branch", "sha", "duration"]);
    }
}
