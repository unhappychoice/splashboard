//! `todoist_tasks` — Todoist task snapshot with structured filters.
//!
//! Safety::Safe: every request targets Todoist's fixed REST host (`api.todoist.com`), so
//! config cannot redirect tokens to arbitrary origins.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, FixedOffset, NaiveDate};
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData, Payload, Status,
    TextBlockData, TextData, TimelineData, TimelineEvent,
};
use crate::render::Shape;
use crate::samples;
use crate::time as t;

use super::{FetchContext, FetchError, Fetcher, Safety};

const API_TASKS: &str = "https://api.todoist.com/api/v1/tasks";
const API_TASKS_FILTER: &str = "https://api.todoist.com/api/v1/tasks/filter";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_MAX_ITEMS: usize = 10;
const MAX_ITEMS_CAP: usize = 100;
const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::Text,
    Shape::TextBlock,
    Shape::Entries,
    Shape::Timeline,
    Shape::Badge,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "token",
        type_hint: "string",
        required: false,
        default: None,
        description: "Todoist API token. If omitted, `TODOIST_TOKEN` env var is used.",
    },
    OptionSchema {
        name: "filter_due",
        type_hint: "\"today\" | \"overdue\" | \"upcoming\" | \"all\"",
        required: false,
        default: Some("\"all\""),
        description: "Due-date window.",
    },
    OptionSchema {
        name: "filter_include_overdue",
        type_hint: "boolean",
        required: false,
        default: Some("true"),
        description: "When `filter_due = \"today\"`, include overdue tasks too.",
    },
    OptionSchema {
        name: "filter_projects",
        type_hint: "array of strings",
        required: false,
        default: None,
        description: "Project names matched as an OR clause (`#ProjectA | #ProjectB`).",
    },
    OptionSchema {
        name: "filter_labels",
        type_hint: "array of strings",
        required: false,
        default: None,
        description: "Label names matched as an OR clause (`@urgent | @backend`).",
    },
    OptionSchema {
        name: "filter_priorities",
        type_hint: "array of integers (1..=4)",
        required: false,
        default: None,
        description: "Priority filter matched as an OR clause (`p1 | p2`).",
    },
    OptionSchema {
        name: "max_items",
        type_hint: "integer (1..=100)",
        required: false,
        default: Some("10"),
        description: "Maximum rendered tasks for list-like shapes (TextBlock/Entries/Timeline).",
    },
];

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![Arc::new(TodoistTasks)]
}

pub struct TodoistTasks;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    #[serde(alias = "due")]
    filter_due: Option<DueWindow>,
    #[serde(default)]
    #[serde(alias = "include_overdue")]
    filter_include_overdue: Option<bool>,
    #[serde(default)]
    #[serde(alias = "projects")]
    filter_projects: Option<Vec<String>>,
    #[serde(default)]
    #[serde(alias = "labels")]
    filter_labels: Option<Vec<String>>,
    #[serde(default)]
    #[serde(alias = "priorities")]
    filter_priorities: Option<Vec<u8>>,
    #[serde(default)]
    max_items: Option<usize>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum DueWindow {
    Today,
    Overdue,
    Upcoming,
    #[default]
    All,
}

#[derive(Debug, Deserialize)]
struct ApiTask {
    id: String,
    content: String,
    priority: u8,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    due: Option<ApiDue>,
}

#[derive(Debug, Deserialize)]
struct ApiTasksResponse {
    results: Vec<ApiTask>,
}

#[derive(Debug, Deserialize)]
struct ApiDue {
    date: String,
    #[serde(default)]
    datetime: Option<String>,
}

#[derive(Debug)]
struct TaskView {
    content: String,
    priority: u8,
    url: Option<String>,
    due_label: Option<String>,
    due_sort_key: Option<i64>,
    timeline_ts: Option<i64>,
    is_overdue: bool,
}

#[async_trait]
impl Fetcher for TodoistTasks {
    fn name(&self) -> &str {
        "todoist_tasks"
    }

    fn safety(&self) -> Safety {
        Safety::Safe
    }

    fn description(&self) -> &'static str {
        "Todoist task snapshot, sorted by due date and priority, with structured filters for due window, projects, labels, and priority. `Badge` summarises overdue/total counts; list shapes show each task with its due label and priority and link back to the Todoist app."
    }

    fn shapes(&self) -> &[Shape] {
        SHAPES
    }

    fn default_shape(&self) -> Shape {
        Shape::LinkedTextBlock
    }

    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }

    fn cache_key(&self, ctx: &FetchContext) -> String {
        cache_key(self.name(), ctx)
    }

    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "overdue · P4 Fix flaky CI",
                    Some("https://app.todoist.com/app/task/123"),
                ),
                (
                    "today · P3 Draft release notes",
                    Some("https://app.todoist.com/app/task/456"),
                ),
            ]),
            Shape::Text => samples::text("todo 5 tasks (2 overdue)"),
            Shape::TextBlock => samples::text_block(&[
                "overdue · P4 Fix flaky CI",
                "today · P3 Draft release notes",
            ]),
            Shape::Entries => samples::entries(&[
                ("Fix flaky CI", "overdue · P4"),
                ("Draft release notes", "today · P3"),
            ]),
            Shape::Timeline => samples::timeline(&[
                (1_776_630_000, "Fix flaky CI", Some("overdue · P4")),
                (1_776_716_400, "Draft release notes", Some("today · P3")),
            ]),
            Shape::Badge => Body::Badge(BadgeData {
                status: Status::Error,
                label: "todo 5 (2 overdue)".into(),
            }),
            _ => return None,
        })
    }

    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref())?;
        let token = resolve_token(opts.token.as_deref())?;
        let tasks = fetch_tasks(&token, build_filter(&opts)).await?;
        let now = t::now_in(ctx.timezone.as_deref());
        let mut views: Vec<TaskView> = tasks.into_iter().map(|t| to_task_view(t, &now)).collect();
        views.sort_by(cmp_views);
        let max_items = opts
            .max_items
            .unwrap_or(DEFAULT_MAX_ITEMS)
            .clamp(1, MAX_ITEMS_CAP);
        Ok(payload(render_body(
            &views,
            ctx.shape.unwrap_or(Shape::TextBlock),
            max_items,
        )))
    }
}

fn cache_key(name: &str, ctx: &FetchContext) -> String {
    let shape = ctx.shape.map(|s| s.as_str()).unwrap_or("default");
    let opts = ctx
        .options
        .as_ref()
        .map(toml::Value::to_string)
        .unwrap_or_default();
    let raw = format!(
        "{}|{}|{}|{}",
        name,
        shape,
        ctx.format.as_deref().unwrap_or(""),
        opts
    );
    let digest = Sha256::digest(raw.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{name}-{hex}")
}

async fn fetch_tasks(token: &str, filter: Option<String>) -> Result<Vec<ApiTask>, FetchError> {
    let (url, req) = match filter.as_deref() {
        Some(q) => (
            API_TASKS_FILTER,
            http()
                .get(API_TASKS_FILTER)
                .bearer_auth(token)
                .query(&[("query", q)]),
        ),
        None => (API_TASKS, http().get(API_TASKS).bearer_auth(token)),
    };
    let res = req
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("todoist request failed: {e}")))?;
    let status = res.status();
    let body = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("todoist response body: {e}")))?;
    if !status.is_success() {
        return Err(FetchError::Failed(todoist_error(status, &body)));
    }
    serde_json::from_slice::<ApiTasksResponse>(&body)
        .map(|v| v.results)
        .map_err(|e| FetchError::Failed(format!("todoist json parse ({url}): {e}")))
}

fn todoist_error(status: reqwest::StatusCode, body: &[u8]) -> String {
    #[derive(Deserialize)]
    struct ApiError {
        #[serde(default)]
        error: Option<String>,
        #[serde(default)]
        message: Option<String>,
    }
    let text = serde_json::from_slice::<ApiError>(body)
        .ok()
        .and_then(|e| e.error.or(e.message))
        .or_else(|| {
            std::str::from_utf8(body)
                .ok()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
        });
    text.map(|t| format!("todoist {status}: {t}"))
        .unwrap_or_else(|| format!("todoist {status}"))
}

fn build_filter(opts: &Options) -> Option<String> {
    let due = due_clause(
        opts.filter_due.unwrap_or_default(),
        opts.filter_include_overdue.unwrap_or(true),
    );
    let projects = scoped_or_clause('#', opts.filter_projects.as_deref());
    let labels = scoped_or_clause('@', opts.filter_labels.as_deref());
    let priorities = priorities_clause(opts.filter_priorities.as_deref());
    let clauses: Vec<String> = [due, projects, labels, priorities]
        .into_iter()
        .flatten()
        .collect();
    (!clauses.is_empty()).then(|| clauses.join(" & "))
}

fn due_clause(due: DueWindow, include_overdue: bool) -> Option<String> {
    match due {
        DueWindow::Today => Some(if include_overdue {
            "(today | overdue)".into()
        } else {
            "today".into()
        }),
        DueWindow::Overdue => Some("overdue".into()),
        DueWindow::Upcoming => Some("7 days & !today".into()),
        DueWindow::All => None,
    }
}

fn scoped_or_clause(prefix: char, values: Option<&[String]>) -> Option<String> {
    let terms: Vec<String> = values
        .unwrap_or(&[])
        .iter()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| format!("{prefix}{}", escape_filter_atom(v)))
        .collect();
    or_clause(&terms)
}

fn priorities_clause(values: Option<&[u8]>) -> Option<String> {
    let terms: Vec<String> = values
        .unwrap_or(&[])
        .iter()
        .filter(|v| (1u8..=4u8).contains(v))
        .map(|v| format!("p{v}"))
        .collect();
    or_clause(&terms)
}

fn or_clause(values: &[String]) -> Option<String> {
    match values.len() {
        0 => None,
        1 => Some(values[0].clone()),
        _ => Some(format!("({})", values.join(" | "))),
    }
}

fn escape_filter_atom(value: &str) -> String {
    let plain = value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if plain {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('"', "\\\""))
    }
}

fn to_task_view(task: ApiTask, now: &DateTime<FixedOffset>) -> TaskView {
    let created_ts = task
        .created_at
        .as_deref()
        .and_then(parse_rfc3339_timestamp)
        .unwrap_or_else(|| now.timestamp());
    let (due_label, due_sort_key, timeline_ts, is_overdue) = task
        .due
        .as_ref()
        .map(|d| parse_due(d, now))
        .unwrap_or((None, None, None, false));
    TaskView {
        content: task.content,
        priority: task.priority.clamp(1, 4),
        url: task_link(&task.id),
        due_label,
        due_sort_key,
        timeline_ts: timeline_ts.or(Some(created_ts)),
        is_overdue,
    }
}

fn parse_due(
    due: &ApiDue,
    now: &DateTime<FixedOffset>,
) -> (Option<String>, Option<i64>, Option<i64>, bool) {
    let tz = *now.offset();
    if let Some(dt) = due.datetime.as_deref().and_then(parse_rfc3339_timestamp) {
        let today = now.date_naive();
        let date_label = DateTime::from_timestamp(dt, 0)
            .map(|d| d.with_timezone(&tz).date_naive())
            .map(|d| due_label_from_date(d, today))
            .unwrap_or_else(|| "scheduled".into());
        return (Some(date_label), Some(dt), Some(dt), dt < now.timestamp());
    }
    NaiveDate::parse_from_str(&due.date, "%Y-%m-%d")
        .ok()
        .map(|date| {
            let ts = date
                .and_hms_opt(12, 0, 0)
                .map(|v| v.and_utc().timestamp())
                .unwrap_or(0);
            (
                Some(due_label_from_date(date, now.date_naive())),
                Some(ts),
                Some(ts),
                date < now.date_naive(),
            )
        })
        .unwrap_or((None, None, None, false))
}

fn due_label_from_date(date: NaiveDate, today: NaiveDate) -> String {
    if date < today {
        "overdue".into()
    } else if date == today {
        "today".into()
    } else if date == today.succ_opt().unwrap_or(today) {
        "tomorrow".into()
    } else {
        format!("{:02}-{:02}", date.month(), date.day())
    }
}

fn parse_rfc3339_timestamp(raw: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.timestamp())
}

fn cmp_views(a: &TaskView, b: &TaskView) -> std::cmp::Ordering {
    a.due_sort_key
        .unwrap_or(i64::MAX)
        .cmp(&b.due_sort_key.unwrap_or(i64::MAX))
        .then_with(|| b.priority.cmp(&a.priority))
        .then_with(|| a.content.cmp(&b.content))
}

fn render_body(tasks: &[TaskView], shape: Shape, max_items: usize) -> Body {
    let overdue_count = tasks.iter().filter(|t| t.is_overdue).count();
    match shape {
        Shape::Text => Body::Text(TextData {
            value: format!("todo {} tasks ({} overdue)", tasks.len(), overdue_count),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: tasks
                .iter()
                .take(max_items)
                .map(|t| LinkedLine {
                    text: task_line(t),
                    url: t.url.clone(),
                })
                .collect(),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: tasks
                .iter()
                .take(max_items)
                .map(|t| Entry {
                    key: t.content.clone(),
                    value: Some(task_meta(t)),
                    status: None,
                })
                .collect(),
        }),
        Shape::Timeline => Body::Timeline(TimelineData {
            events: tasks
                .iter()
                .filter_map(|t| {
                    t.timeline_ts.map(|ts| TimelineEvent {
                        timestamp: ts,
                        title: t.content.clone(),
                        detail: Some(task_meta(t)),
                        status: t.is_overdue.then_some(Status::Error),
                    })
                })
                .take(max_items)
                .collect(),
        }),
        Shape::Badge => Body::Badge(BadgeData {
            status: badge_status(tasks.len(), overdue_count),
            label: format!("todo {} ({} overdue)", tasks.len(), overdue_count),
        }),
        _ => Body::TextBlock(TextBlockData {
            lines: tasks.iter().take(max_items).map(task_line).collect(),
        }),
    }
}

fn badge_status(total: usize, overdue: usize) -> Status {
    if overdue > 0 {
        Status::Error
    } else if total > 0 {
        Status::Warn
    } else {
        Status::Ok
    }
}

fn task_line(task: &TaskView) -> String {
    format!("{} {}", task_meta(task), task.content)
}

fn task_meta(task: &TaskView) -> String {
    let due = task.due_label.as_deref().unwrap_or("no due");
    format!("{due} · P{}", task.priority)
}

fn task_link(id: &str) -> Option<String> {
    (!id.is_empty()).then(|| format!("https://app.todoist.com/app/task/{id}"))
}

fn parse_options(raw: Option<&toml::Value>) -> Result<Options, FetchError> {
    raw.map(|v| {
        v.clone()
            .try_into::<Options>()
            .map_err(|e| FetchError::Failed(format!("invalid options: {e}")))
    })
    .unwrap_or_else(|| Ok(Options::default()))
}

fn resolve_token(config_token: Option<&str>) -> Result<String, FetchError> {
    config_token
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .or_else(|| std::env::var("TODOIST_TOKEN").ok())
        .ok_or_else(|| {
            FetchError::Failed("todoist token missing: set options.token or TODOIST_TOKEN".into())
        })
}

fn http() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .gzip(true)
            .build()
            .expect("reqwest client should build with default config")
    })
}

fn payload(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(raw: &str) -> Options {
        let value: toml::Value = toml::from_str(raw).unwrap();
        parse_options(Some(&value)).unwrap()
    }

    fn ctx(shape: Option<Shape>, raw: Option<&str>) -> FetchContext {
        FetchContext {
            widget_id: "todo".into(),
            timeout: Duration::from_secs(1),
            shape,
            options: raw.map(|s| toml::from_str(s).unwrap()),
            ..Default::default()
        }
    }

    fn fixed_now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-04-22T10:00:00+09:00").unwrap()
    }

    fn task(
        content: &str,
        priority: u8,
        due_label: Option<&str>,
        due_sort_key: Option<i64>,
        timeline_ts: Option<i64>,
        is_overdue: bool,
    ) -> TaskView {
        TaskView {
            content: content.into(),
            priority,
            url: task_link(content),
            due_label: due_label.map(String::from),
            due_sort_key,
            timeline_ts,
            is_overdue,
        }
    }

    #[test]
    fn fetchers_registry_exposes_todoist_tasks() {
        let fetchers = fetchers();
        let names: Vec<_> = fetchers.iter().map(|fetcher| fetcher.name()).collect();
        assert_eq!(names, vec!["todoist_tasks"]);
    }

    #[test]
    fn sample_body_covers_supported_shapes_and_default_shape() {
        let fetcher = TodoistTasks;
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        assert!(
            SHAPES
                .iter()
                .all(|shape| fetcher.sample_body(*shape).is_some())
        );
        assert!(fetcher.sample_body(Shape::Ratio).is_none());
    }

    #[test]
    fn cache_key_changes_with_shape_format_and_options() {
        let mut base = ctx(Some(Shape::TextBlock), None);
        let key = TodoistTasks.cache_key(&base);
        base.shape = Some(Shape::Badge);
        assert_ne!(key, TodoistTasks.cache_key(&base));

        let mut format_ctx = ctx(Some(Shape::TextBlock), None);
        let original = TodoistTasks.cache_key(&format_ctx);
        format_ctx.format = Some("compact".into());
        assert_ne!(original, TodoistTasks.cache_key(&format_ctx));

        let mut opts_ctx = ctx(Some(Shape::TextBlock), None);
        let no_opts = TodoistTasks.cache_key(&opts_ctx);
        opts_ctx.options = Some(toml::from_str("filter_due = \"today\"").unwrap());
        assert_ne!(no_opts, TodoistTasks.cache_key(&opts_ctx));
    }

    #[test]
    fn parse_options_supports_aliases_and_defaults() {
        let raw: toml::Value = toml::from_str(
            "token = \"abc\"\ndue = \"overdue\"\ninclude_overdue = false\nprojects = [\"Work\"]\nlabels = [\"ops\"]\npriorities = [4]\nmax_items = 5",
        )
        .unwrap();
        let opts = parse_options(Some(&raw)).unwrap();
        assert_eq!(opts.token.as_deref(), Some("abc"));
        assert!(matches!(opts.filter_due, Some(DueWindow::Overdue)));
        assert_eq!(opts.filter_include_overdue, Some(false));
        assert_eq!(opts.filter_projects, Some(vec!["Work".into()]));
        assert_eq!(opts.filter_labels, Some(vec!["ops".into()]));
        assert_eq!(opts.filter_priorities, Some(vec![4]));
        assert_eq!(opts.max_items, Some(5));
        assert!(parse_options(None).is_ok());
    }

    #[test]
    fn todoist_error_prefers_structured_message_then_plain_text() {
        let status = reqwest::StatusCode::UNAUTHORIZED;
        let json_error = todoist_error(status, br#"{"error":"bad token"}"#);
        let json_message = todoist_error(status, br#"{"message":"fallback"}"#);
        let plain = todoist_error(status, b" permission denied ");
        let empty = todoist_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR, b"");
        assert!(json_error.contains("bad token"));
        assert!(json_message.contains("fallback"));
        assert!(plain.contains("permission denied"));
        assert_eq!(empty, "todoist 500 Internal Server Error");
    }

    #[test]
    fn build_filter_from_structured_options() {
        let opts = options(
            "filter_due = \"today\"\nfilter_projects = [\"Work\", \"Client A\"]\nfilter_labels = [\"urgent\"]\nfilter_priorities = [1, 2]\n",
        );
        assert_eq!(
            build_filter(&opts),
            Some("(today | overdue) & (#Work | #\"Client A\") & @urgent & (p1 | p2)".into())
        );
    }

    #[test]
    fn due_all_without_other_filters_produces_no_query() {
        let opts = options("filter_due = \"all\"");
        assert_eq!(build_filter(&opts), None);
    }

    #[test]
    fn include_overdue_false_keeps_today_only() {
        let opts = options("filter_due = \"today\"\nfilter_include_overdue = false");
        assert_eq!(build_filter(&opts), Some("today".into()));
    }

    #[test]
    fn due_helpers_cover_overdue_upcoming_and_future_date_branches() {
        assert_eq!(due_clause(DueWindow::Overdue, true), Some("overdue".into()));
        assert_eq!(
            due_clause(DueWindow::Upcoming, false),
            Some("7 days & !today".into())
        );

        let today = fixed_now().date_naive();
        let future = NaiveDate::from_ymd_opt(2026, 4, 25).unwrap();
        assert_eq!(due_label_from_date(future, today), "04-25");
    }

    #[test]
    fn priorities_clause_ignores_out_of_range_values() {
        let opts = options("filter_priorities = [0, 1, 4, 9]");
        assert_eq!(build_filter(&opts), Some("(p1 | p4)".into()));
    }

    #[test]
    fn helper_clauses_trim_quote_and_join_values() {
        let projects = vec!["Work".into(), "Client A".into(), "\"Core\"".into()];
        let priorities = vec![0, 2, 4, 9];
        let clause = scoped_or_clause('#', Some(projects.as_slice()));
        assert_eq!(
            clause,
            Some("(#Work | #\"Client A\" | #\"\\\"Core\\\"\")".into())
        );
        assert_eq!(
            priorities_clause(Some(priorities.as_slice())),
            Some("(p2 | p4)".into())
        );
        assert_eq!(or_clause(&[]), None);
        assert_eq!(or_clause(&["solo".into()]), Some("solo".into()));
        assert_eq!(escape_filter_atom("plain_value-1"), "plain_value-1");
        assert_eq!(escape_filter_atom("needs space"), "\"needs space\"");
    }

    #[test]
    fn parse_due_labels_relative_days() {
        let now = t::now_in(None);
        let today_date = now.date_naive();
        let tomorrow_date = today_date.succ_opt().unwrap_or(today_date);
        let overdue_date = today_date.pred_opt().unwrap_or(today_date);
        let today = ApiDue {
            date: today_date.to_string(),
            datetime: None,
        };
        let tomorrow = ApiDue {
            date: tomorrow_date.to_string(),
            datetime: None,
        };
        let overdue = ApiDue {
            date: overdue_date.to_string(),
            datetime: None,
        };
        assert_eq!(parse_due(&today, &now).0.as_deref(), Some("today"));
        assert_eq!(parse_due(&tomorrow, &now).0.as_deref(), Some("tomorrow"));
        assert_eq!(parse_due(&overdue, &now).0.as_deref(), Some("overdue"));
    }

    #[test]
    fn parse_due_datetime_branch_uses_timestamp_and_timezone() {
        let now = fixed_now();
        let due = ApiDue {
            date: "2026-04-30".into(),
            datetime: Some("2026-04-22T00:30:00Z".into()),
        };
        let expected = parse_rfc3339_timestamp("2026-04-22T00:30:00Z").unwrap();
        assert_eq!(
            parse_due(&due, &now),
            (Some("today".into()), Some(expected), Some(expected), true)
        );
    }

    #[test]
    fn parse_due_invalid_values_return_empty_tuple() {
        let now = fixed_now();
        let due = ApiDue {
            date: "not-a-date".into(),
            datetime: Some("not-a-timestamp".into()),
        };
        assert_eq!(parse_due(&due, &now), (None, None, None, false));
    }

    #[test]
    fn to_task_view_clamps_priority_and_falls_back_to_created_timestamp() {
        let now = fixed_now();
        let task = ApiTask {
            id: "42".into(),
            content: "Fix flaky CI".into(),
            priority: 9,
            created_at: Some("2026-04-21T00:00:00Z".into()),
            due: None,
        };
        let view = to_task_view(task, &now);
        assert_eq!(view.priority, 4);
        assert_eq!(
            view.url.as_deref(),
            Some("https://app.todoist.com/app/task/42")
        );
        assert!(view.due_label.is_none());
        assert!(view.due_sort_key.is_none());
        assert_eq!(
            view.timeline_ts,
            parse_rfc3339_timestamp("2026-04-21T00:00:00Z")
        );
        assert!(!view.is_overdue);
    }

    #[test]
    fn cmp_views_orders_by_due_priority_then_content() {
        let mut tasks = vec![
            task("b-task", 2, Some("today"), Some(20), Some(20), false),
            task("a-task", 4, Some("today"), Some(20), Some(20), false),
            task("late-task", 4, Some("tomorrow"), Some(30), Some(30), false),
        ];
        tasks.sort_by(cmp_views);
        let ordered: Vec<_> = tasks.into_iter().map(|task| task.content).collect();
        assert_eq!(ordered, vec!["a-task", "b-task", "late-task"]);
    }

    #[test]
    fn badge_status_covers_ok_warn_and_error_states() {
        assert_eq!(badge_status(0, 0), Status::Ok);
        assert_eq!(badge_status(2, 0), Status::Warn);
        assert_eq!(badge_status(2, 1), Status::Error);
    }

    #[test]
    fn task_helpers_format_meta_and_links() {
        let no_due = task("Fix bug", 3, None, None, None, false);
        assert_eq!(task_meta(&no_due), "no due · P3");
        assert_eq!(task_line(&no_due), "no due · P3 Fix bug");
        assert!(task_link("").is_none());
    }

    #[test]
    fn http_and_payload_helpers_return_stable_defaults() {
        assert!(std::ptr::eq(http(), http()));

        let payload = payload(Body::Text(TextData {
            value: "todo 0 tasks (0 overdue)".into(),
        }));
        assert!(payload.icon.is_none());
        assert!(payload.status.is_none());
        assert!(payload.format.is_none());
        assert!(matches!(
            &payload.body,
            Body::Text(text) if text.value == "todo 0 tasks (0 overdue)"
        ));
    }

    #[test]
    fn render_badge_reflects_overdue_status() {
        let tasks = vec![TaskView {
            content: "Fix bug".into(),
            priority: 4,
            url: Some("https://app.todoist.com/app/task/1".into()),
            due_label: Some("overdue".into()),
            due_sort_key: Some(1),
            timeline_ts: Some(1),
            is_overdue: true,
        }];
        let body = render_body(&tasks, Shape::Badge, 10);
        let Body::Badge(b) = body else {
            panic!("expected badge");
        };
        assert_eq!(b.status, Status::Error);
        assert!(b.label.contains("overdue"));
    }

    #[test]
    fn render_text_entries_and_timeline_respect_limits_and_metadata() {
        let tasks = vec![
            task("Overdue fix", 4, Some("overdue"), Some(1), Some(1), true),
            task("Future task", 2, Some("tomorrow"), Some(2), None, false),
        ];

        let Body::Text(text) = render_body(&tasks, Shape::Text, 10) else {
            panic!("expected text");
        };
        assert_eq!(text.value, "todo 2 tasks (1 overdue)");

        let Body::Entries(entries) = render_body(&tasks, Shape::Entries, 1) else {
            panic!("expected entries");
        };
        assert_eq!(entries.items.len(), 1);
        assert_eq!(entries.items[0].key, "Overdue fix");
        assert_eq!(entries.items[0].value.as_deref(), Some("overdue · P4"));

        let Body::Timeline(timeline) = render_body(&tasks, Shape::Timeline, 10) else {
            panic!("expected timeline");
        };
        assert_eq!(timeline.events.len(), 1);
        assert_eq!(timeline.events[0].title, "Overdue fix");
        assert_eq!(timeline.events[0].status, Some(Status::Error));

        let Body::TextBlock(block) = render_body(&tasks, Shape::TextBlock, 10) else {
            panic!("expected text block");
        };
        assert_eq!(block.lines[0], "overdue · P4 Overdue fix");
    }

    #[test]
    fn linked_text_block_includes_task_url() {
        let tasks = vec![TaskView {
            content: "Review PR".into(),
            priority: 2,
            url: Some("https://app.todoist.com/app/task/42".into()),
            due_label: Some("today".into()),
            due_sort_key: Some(1),
            timeline_ts: Some(1),
            is_overdue: false,
        }];
        let body = render_body(&tasks, Shape::LinkedTextBlock, 10);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked_text_block");
        };
        assert_eq!(b.items.len(), 1);
        assert_eq!(
            b.items[0].url.as_deref(),
            Some("https://app.todoist.com/app/task/42")
        );
    }

    #[test]
    fn resolve_token_prefers_config_then_env_and_errors_when_missing() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var("TODOIST_TOKEN").ok();
        unsafe {
            std::env::set_var("TODOIST_TOKEN", "env-token");
        }
        assert_eq!(resolve_token(Some("config-token")).unwrap(), "config-token");
        assert_eq!(resolve_token(Some("")).unwrap(), "env-token");
        unsafe {
            std::env::remove_var("TODOIST_TOKEN");
        }
        assert!(matches!(
            resolve_token(None),
            Err(FetchError::Failed(msg)) if msg.contains("todoist token missing")
        ));
        unsafe {
            match previous {
                Some(value) => std::env::set_var("TODOIST_TOKEN", value),
                None => std::env::remove_var("TODOIST_TOKEN"),
            }
        }
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_options_before_network() {
        let err = TodoistTasks
            .fetch(&ctx(Some(Shape::Text), Some("bogus = 1")))
            .await;
        assert!(matches!(
            err,
            Err(FetchError::Failed(msg)) if msg.contains("invalid options")
        ));
    }

    #[test]
    fn fetch_reports_missing_token_before_network() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var("TODOIST_TOKEN").ok();
        unsafe {
            std::env::remove_var("TODOIST_TOKEN");
        }
        let err = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(TodoistTasks.fetch(&ctx(Some(Shape::Text), Some("max_items = 500"))));
        assert!(matches!(
            err,
            Err(FetchError::Failed(msg)) if msg.contains("todoist token missing")
        ));
        unsafe {
            match previous {
                Some(value) => std::env::set_var("TODOIST_TOKEN", value),
                None => std::env::remove_var("TODOIST_TOKEN"),
            }
        }
    }
}
