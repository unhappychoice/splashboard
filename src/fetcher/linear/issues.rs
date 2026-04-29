//! `linear_issues` — Linear issues snapshot with structured filters.
//!
//! Safety::Safe: every request targets `api.linear.app` (fixed in [`super::client`]).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, CalendarData, EntriesData, Entry, LinkedLine,
    LinkedTextBlockData, MarkdownTextBlockData, Payload, Status, TextBlockData, TextData,
    TimelineData, TimelineEvent,
};
use crate::render::Shape;
use crate::samples;
use crate::time as t;

use super::client::{graphql_query, resolve_token};

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::Calendar,
    Shape::Badge,
    Shape::Timeline,
];

const DEFAULT_LIMIT: usize = 10;
const MAX_LIMIT: usize = 100;

const QUERY: &str = r#"
query Issues($filter: IssueFilter, $first: Int) {
  issues(filter: $filter, first: $first, orderBy: updatedAt) {
    nodes {
      identifier
      title
      priority
      priorityLabel
      state { name type }
      assignee { displayName email }
      team { key name }
      project { name }
      labels { nodes { name } }
      dueDate
      url
      updatedAt
    }
  }
}
"#;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "token",
        type_hint: "string",
        required: false,
        default: None,
        description: "Linear personal API key (`lin_api_*`). Falls back to `LINEAR_TOKEN` env.",
    },
    OptionSchema {
        name: "filter_status",
        type_hint: "\"open\" | \"unstarted\" | \"started\" | \"completed\" | \"canceled\" | \"triage\" | \"backlog\"",
        required: false,
        default: Some("\"open\""),
        description: "Workflow state filter. `open` excludes completed and canceled.",
    },
    OptionSchema {
        name: "filter_assignee",
        type_hint: "\"me\" | \"unassigned\" | email | \"any\"",
        required: false,
        default: Some("\"me\""),
        description: "Assignee scope.",
    },
    OptionSchema {
        name: "filter_team",
        type_hint: "string",
        required: false,
        default: None,
        description: "Team key (e.g. `\"ENG\"`).",
    },
    OptionSchema {
        name: "filter_project",
        type_hint: "string",
        required: false,
        default: None,
        description: "Project name match.",
    },
    OptionSchema {
        name: "filter_priority",
        type_hint: "\"urgent\" | \"high\" | \"medium\" | \"low\" | \"none\"",
        required: false,
        default: None,
        description: "Priority filter.",
    },
    OptionSchema {
        name: "filter_due",
        type_hint: "\"overdue\" | \"today\" | \"this_week\" | \"no_due\" | \"any\"",
        required: false,
        default: Some("\"any\""),
        description: "Due-date window.",
    },
    OptionSchema {
        name: "filter_label",
        type_hint: "string",
        required: false,
        default: None,
        description: "Label name match.",
    },
    OptionSchema {
        name: "group_by",
        type_hint: "\"priority\" | \"status\" | \"team\" | \"assignee\" | \"project\"",
        required: false,
        default: Some("\"priority\""),
        description: "Bars grouping. Ignored by other shapes.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=100)",
        required: false,
        default: Some("10"),
        description: "Max rows for list-shaped renderers.",
    },
];

pub struct LinearIssues;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    filter_status: Option<String>,
    #[serde(default)]
    filter_assignee: Option<String>,
    #[serde(default)]
    filter_team: Option<String>,
    #[serde(default)]
    filter_project: Option<String>,
    #[serde(default)]
    filter_priority: Option<String>,
    #[serde(default)]
    filter_due: Option<String>,
    #[serde(default)]
    filter_label: Option<String>,
    #[serde(default)]
    group_by: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    issues: ApiConnection<ApiIssue>,
}

#[derive(Debug, Deserialize)]
struct ApiConnection<T> {
    nodes: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct ApiIssue {
    identifier: String,
    title: String,
    priority: f32,
    #[serde(rename = "priorityLabel")]
    priority_label: Option<String>,
    state: ApiState,
    #[serde(default)]
    assignee: Option<ApiUser>,
    #[serde(default)]
    team: Option<ApiTeam>,
    #[serde(default)]
    project: Option<ApiProject>,
    #[serde(default)]
    labels: Option<ApiConnection<ApiLabel>>,
    #[serde(rename = "dueDate", default)]
    due_date: Option<String>,
    url: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct ApiState {
    name: String,
    #[serde(rename = "type")]
    state_type: String,
}

#[derive(Debug, Deserialize)]
struct ApiUser {
    #[serde(rename = "displayName", default)]
    display_name: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiTeam {
    key: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct ApiProject {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ApiLabel {
    name: String,
}

#[derive(Debug)]
struct IssueView {
    identifier: String,
    title: String,
    priority: u8,
    priority_label: String,
    state_name: String,
    state_type: String,
    assignee: Option<String>,
    team_name: Option<String>,
    project: Option<String>,
    labels: Vec<String>,
    due_date: Option<NaiveDate>,
    url: String,
    updated_at: i64,
}

#[async_trait]
impl Fetcher for LinearIssues {
    fn name(&self) -> &str {
        "linear_issues"
    }

    fn safety(&self) -> Safety {
        Safety::Safe
    }

    fn description(&self) -> &'static str {
        "Linear issues snapshot with filter-first options (status, assignee, team, project, priority, due window, label). `Bars` groups by `group_by` (default priority). `Calendar` highlights due dates; list shapes link each row to the Linear issue."
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
        super::cache_key("linear_issues", ctx)
    }

    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "ENG-123 Fix login flow [In Progress] · today · P1",
                    Some("https://linear.app/acme/issue/ENG-123"),
                ),
                (
                    "ENG-118 Polish dashboard layout [Todo] · P3",
                    Some("https://linear.app/acme/issue/ENG-118"),
                ),
            ]),
            Shape::Text => samples::text("7 issues"),
            Shape::TextBlock => samples::text_block(&[
                "ENG-123 Fix login flow [In Progress] · today · P1",
                "ENG-118 Polish dashboard layout [Todo] · P3",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "- **ENG-123** Fix login flow *[In Progress]* · today · P1\n- **ENG-118** Polish dashboard layout *[Todo]* · P3",
            ),
            Shape::Entries => samples::entries(&[
                ("ENG-123", "Fix login flow"),
                ("ENG-118", "Polish dashboard layout"),
            ]),
            Shape::Bars => samples::bars(&[("Urgent", 1), ("High", 3), ("Medium", 2), ("Low", 1)]),
            Shape::Calendar => samples::calendar(2026, 4, Some(29), &[2, 14, 22]),
            Shape::Badge => samples::badge(Status::Warn, "linear 7"),
            Shape::Timeline => samples::timeline(&[
                (
                    1_745_896_800,
                    "ENG-123 Fix login flow",
                    Some("In Progress · P1"),
                ),
                (
                    1_745_810_400,
                    "ENG-118 Polish dashboard layout",
                    Some("Todo · P3"),
                ),
            ]),
            _ => return None,
        })
    }

    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref())?;
        let token = resolve_token(opts.token.as_deref())?;
        let now = t::now_in(ctx.timezone.as_deref());
        let today = now.date_naive();
        let filter = build_filter(&opts, today);
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let response: ApiResponse = graphql_query(
            &token,
            QUERY,
            json!({ "filter": filter, "first": limit as i64 }),
        )
        .await?;
        let mut views: Vec<IssueView> = response.issues.nodes.into_iter().map(to_view).collect();
        views.sort_by(cmp_views);
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        Ok(payload(render_body(&views, shape, &opts, limit)))
    }
}

pub fn fetcher() -> Arc<dyn Fetcher> {
    Arc::new(LinearIssues)
}

fn parse_options(raw: Option<&toml::Value>) -> Result<Options, FetchError> {
    raw.map(|v| {
        v.clone()
            .try_into::<Options>()
            .map_err(|e| FetchError::Failed(format!("invalid options: {e}")))
    })
    .unwrap_or_else(|| Ok(Options::default()))
}

fn build_filter(opts: &Options, today: NaiveDate) -> Value {
    let mut clauses: Vec<(&str, Value)> = Vec::new();
    if let Some(v) = state_filter(opts.filter_status.as_deref()) {
        clauses.push(("state", v));
    }
    if let Some(v) = assignee_filter(opts.filter_assignee.as_deref()) {
        clauses.push(("assignee", v));
    }
    if let Some(team) = opts.filter_team.as_deref().filter(|s| !s.is_empty()) {
        clauses.push(("team", json!({ "key": { "eq": team } })));
    }
    if let Some(project) = opts.filter_project.as_deref().filter(|s| !s.is_empty()) {
        clauses.push(("project", json!({ "name": { "eq": project } })));
    }
    if let Some(p) = priority_filter(opts.filter_priority.as_deref()) {
        clauses.push(("priority", p));
    }
    if let Some(v) = due_filter(opts.filter_due.as_deref(), today) {
        clauses.push(("dueDate", v));
    }
    if let Some(label) = opts.filter_label.as_deref().filter(|s| !s.is_empty()) {
        clauses.push(("labels", json!({ "name": { "eq": label } })));
    }
    let mut map = serde_json::Map::with_capacity(clauses.len());
    for (k, v) in clauses {
        map.insert(k.into(), v);
    }
    Value::Object(map)
}

fn state_filter(raw: Option<&str>) -> Option<Value> {
    let kind = raw.unwrap_or("open").trim().to_lowercase();
    match kind.as_str() {
        "any" => None,
        "open" => Some(json!({ "type": { "nin": ["completed", "canceled"] } })),
        "unstarted" | "started" | "completed" | "canceled" | "triage" | "backlog" => {
            Some(json!({ "type": { "eq": kind } }))
        }
        _ => Some(json!({ "name": { "eq": raw.unwrap_or_default() } })),
    }
}

fn assignee_filter(raw: Option<&str>) -> Option<Value> {
    match raw.unwrap_or("me").trim().to_lowercase().as_str() {
        "me" => Some(json!({ "isMe": { "eq": true } })),
        "unassigned" => Some(json!({ "null": true })),
        "any" => None,
        other => Some(json!({ "email": { "eq": other } })),
    }
}

fn priority_filter(raw: Option<&str>) -> Option<Value> {
    let v = raw?.trim().to_lowercase();
    match v.as_str() {
        "none" => Some(json!({ "eq": 0 })),
        "urgent" => Some(json!({ "eq": 1 })),
        "high" => Some(json!({ "eq": 2 })),
        "medium" => Some(json!({ "eq": 3 })),
        "low" => Some(json!({ "eq": 4 })),
        _ => v.parse::<u8>().ok().map(|n| json!({ "eq": n })),
    }
}

fn due_filter(raw: Option<&str>, today: NaiveDate) -> Option<Value> {
    let kind = raw.unwrap_or("any").trim().to_lowercase();
    let iso = today.format("%Y-%m-%d").to_string();
    let week = (today + chrono::Duration::days(7))
        .format("%Y-%m-%d")
        .to_string();
    match kind.as_str() {
        "any" => None,
        "no_due" => Some(json!({ "null": true })),
        "overdue" => Some(json!({ "lt": iso })),
        "today" => Some(json!({ "eq": iso })),
        "this_week" => Some(json!({ "gte": iso, "lte": week })),
        _ => None,
    }
}

fn to_view(api: ApiIssue) -> IssueView {
    IssueView {
        identifier: api.identifier,
        title: api.title,
        priority: priority_to_u8(api.priority),
        priority_label: api.priority_label.unwrap_or_else(|| "—".into()),
        state_name: api.state.name,
        state_type: api.state.state_type,
        assignee: api.assignee.and_then(|a| a.display_name.or(a.email)),
        team_name: api.team.map(|t| t.name),
        project: api.project.map(|p| p.name),
        labels: api
            .labels
            .map(|c| c.nodes.into_iter().map(|l| l.name).collect())
            .unwrap_or_default(),
        due_date: api
            .due_date
            .as_deref()
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()),
        url: api.url,
        updated_at: parse_timestamp(&api.updated_at),
    }
}

fn priority_to_u8(p: f32) -> u8 {
    let n = p.round() as i32;
    n.clamp(0, 4) as u8
}

fn parse_timestamp(raw: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.timestamp())
        .unwrap_or(0)
}

fn cmp_views(a: &IssueView, b: &IssueView) -> std::cmp::Ordering {
    priority_rank(a.priority)
        .cmp(&priority_rank(b.priority))
        .then_with(|| due_rank(a.due_date).cmp(&due_rank(b.due_date)))
        .then_with(|| b.updated_at.cmp(&a.updated_at))
}

/// 0 (Urgent) ranks above 1..4; "no priority" (0 from API) ranks last.
fn priority_rank(p: u8) -> u8 {
    match p {
        0 => 5,
        n => n,
    }
}

fn due_rank(d: Option<NaiveDate>) -> i64 {
    d.map(|x| x.num_days_from_ce() as i64).unwrap_or(i64::MAX)
}

fn render_body(views: &[IssueView], shape: Shape, opts: &Options, limit: usize) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: format!("{} issues", views.len()),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: views.iter().take(limit).map(line_for).collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: views
                .iter()
                .take(limit)
                .map(markdown_line_for)
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: views
                .iter()
                .take(limit)
                .map(|v| LinkedLine {
                    text: line_for(v),
                    url: Some(v.url.clone()),
                })
                .collect(),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: views
                .iter()
                .take(limit)
                .map(|v| Entry {
                    key: v.identifier.clone(),
                    value: Some(v.title.clone()),
                    status: state_to_status(&v.state_type),
                })
                .collect(),
        }),
        Shape::Bars => bars_body(views, opts.group_by.as_deref()),
        Shape::Calendar => calendar_body(views),
        Shape::Badge => Body::Badge(BadgeData {
            status: badge_status(views.len()),
            label: format!("linear {}", views.len()),
        }),
        Shape::Timeline => Body::Timeline(TimelineData {
            events: views
                .iter()
                .take(limit)
                .map(|v| TimelineEvent {
                    timestamp: v.updated_at,
                    title: format!("{} {}", v.identifier, v.title),
                    detail: Some(format!("{} · P{}", v.state_name, v.priority)),
                    status: state_to_status(&v.state_type),
                })
                .collect(),
        }),
        _ => Body::TextBlock(TextBlockData {
            lines: views.iter().take(limit).map(line_for).collect(),
        }),
    }
}

fn line_for(v: &IssueView) -> String {
    let due = v
        .due_date
        .map(|d| format!(" · due {:02}-{:02}", d.month(), d.day()))
        .unwrap_or_default();
    format!(
        "{} {} [{}]{} · P{}",
        v.identifier, v.title, v.state_name, due, v.priority
    )
}

fn markdown_line_for(v: &IssueView) -> String {
    let due = v
        .due_date
        .map(|d| format!(" · due {:02}-{:02}", d.month(), d.day()))
        .unwrap_or_default();
    format!(
        "- **{}** {} *[{}]*{} · P{}",
        v.identifier, v.title, v.state_name, due, v.priority
    )
}

fn state_to_status(state_type: &str) -> Option<Status> {
    match state_type {
        "completed" => Some(Status::Ok),
        "started" => Some(Status::Warn),
        "canceled" => Some(Status::Error),
        _ => None,
    }
}

fn badge_status(total: usize) -> Status {
    match total {
        0 => Status::Ok,
        1..=3 => Status::Ok,
        4..=9 => Status::Warn,
        _ => Status::Error,
    }
}

fn bars_body(views: &[IssueView], group_by: Option<&str>) -> Body {
    let key = group_by.unwrap_or("priority").trim().to_lowercase();
    let extract: Box<dyn Fn(&IssueView) -> String> = match key.as_str() {
        "status" => Box::new(|v| v.state_name.clone()),
        "team" => Box::new(|v| v.team_name.clone().unwrap_or_else(|| "—".into())),
        "assignee" => Box::new(|v| v.assignee.clone().unwrap_or_else(|| "Unassigned".into())),
        "project" => Box::new(|v| v.project.clone().unwrap_or_else(|| "—".into())),
        _ => Box::new(|v| v.priority_label.clone()),
    };
    Body::Bars(BarsData {
        bars: tally(views, extract.as_ref()),
    })
}

fn tally(views: &[IssueView], extract: &dyn Fn(&IssueView) -> String) -> Vec<Bar> {
    let mut counts: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for v in views {
        *counts.entry(extract(v)).or_insert(0) += 1;
    }
    let mut bars: Vec<Bar> = counts
        .into_iter()
        .map(|(label, value)| Bar { label, value })
        .collect();
    bars.sort_by(|a, b| b.value.cmp(&a.value).then_with(|| a.label.cmp(&b.label)));
    bars
}

fn calendar_body(views: &[IssueView]) -> Body {
    let now = t::now_in(None);
    let today = now.date_naive();
    let events: Vec<u8> = views
        .iter()
        .filter_map(|v| v.due_date)
        .filter(|d| d.year() == today.year() && d.month() == today.month())
        .map(|d| d.day() as u8)
        .collect();
    Body::Calendar(CalendarData {
        year: today.year(),
        month: today.month() as u8,
        day: Some(today.day() as u8),
        events,
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

    #[test]
    fn state_filter_open_excludes_completed_and_canceled() {
        let v = state_filter(Some("open")).unwrap();
        assert_eq!(
            v["type"]["nin"],
            json!(["completed", "canceled"]),
            "got: {v}"
        );
    }

    #[test]
    fn state_filter_named_state_uses_eq() {
        let v = state_filter(Some("started")).unwrap();
        assert_eq!(v["type"]["eq"], json!("started"));
    }

    #[test]
    fn state_filter_any_means_no_filter() {
        assert!(state_filter(Some("any")).is_none());
    }

    #[test]
    fn assignee_filter_me_uses_isme() {
        let v = assignee_filter(Some("me")).unwrap();
        assert_eq!(v["isMe"]["eq"], json!(true));
    }

    #[test]
    fn assignee_filter_unassigned_uses_null() {
        let v = assignee_filter(Some("unassigned")).unwrap();
        assert_eq!(v["null"], json!(true));
    }

    #[test]
    fn assignee_filter_email_uses_eq() {
        let v = assignee_filter(Some("a@b.com")).unwrap();
        assert_eq!(v["email"]["eq"], json!("a@b.com"));
    }

    #[test]
    fn priority_filter_named_to_numeric() {
        assert_eq!(priority_filter(Some("urgent")).unwrap()["eq"], json!(1));
        assert_eq!(priority_filter(Some("low")).unwrap()["eq"], json!(4));
        assert_eq!(priority_filter(Some("none")).unwrap()["eq"], json!(0));
    }

    #[test]
    fn due_filter_overdue_uses_lt_today() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 29).unwrap();
        let v = due_filter(Some("overdue"), today).unwrap();
        assert_eq!(v["lt"], json!("2026-04-29"));
    }

    #[test]
    fn due_filter_this_week_spans_seven_days() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 29).unwrap();
        let v = due_filter(Some("this_week"), today).unwrap();
        assert_eq!(v["gte"], json!("2026-04-29"));
        assert_eq!(v["lte"], json!("2026-05-06"));
    }

    #[test]
    fn build_filter_combines_clauses() {
        let opts = options(
            "filter_status = \"started\"\nfilter_team = \"ENG\"\nfilter_priority = \"high\"\n",
        );
        let f = build_filter(&opts, NaiveDate::from_ymd_opt(2026, 4, 29).unwrap());
        assert_eq!(f["state"]["type"]["eq"], json!("started"));
        assert_eq!(f["team"]["key"]["eq"], json!("ENG"));
        assert_eq!(f["priority"]["eq"], json!(2));
        // Default `filter_assignee` of "me" still applied.
        assert_eq!(f["assignee"]["isMe"]["eq"], json!(true));
    }

    #[test]
    fn cmp_views_promotes_urgent_above_high() {
        let urgent = view_for_test("ENG-1", 1, None, 0);
        let high = view_for_test("ENG-2", 2, None, 0);
        let mut list = [high, urgent];
        list.sort_by(cmp_views);
        assert_eq!(list[0].identifier, "ENG-1");
    }

    #[test]
    fn cmp_views_demotes_no_priority_below_low() {
        let none = view_for_test("ENG-1", 0, None, 0);
        let low = view_for_test("ENG-2", 4, None, 0);
        let mut list = [none, low];
        list.sort_by(cmp_views);
        assert_eq!(list[0].identifier, "ENG-2");
    }

    #[test]
    fn render_text_emits_count() {
        let views = vec![view_for_test("ENG-1", 1, None, 0)];
        let body = render_body(&views, Shape::Text, &Options::default(), 10);
        let Body::Text(t) = body else {
            panic!("expected text");
        };
        assert!(t.value.contains("1"));
    }

    #[test]
    fn render_linked_text_block_includes_url() {
        let views = vec![view_for_test("ENG-1", 1, None, 0)];
        let body = render_body(&views, Shape::LinkedTextBlock, &Options::default(), 10);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked");
        };
        assert!(b.items[0].url.as_deref().unwrap().contains("ENG-1"));
    }

    #[test]
    fn render_bars_groups_by_priority_label_by_default() {
        let views = vec![
            view_for_test("ENG-1", 1, None, 0),
            view_for_test("ENG-2", 1, None, 0),
            view_for_test("ENG-3", 2, None, 0),
        ];
        let body = render_body(&views, Shape::Bars, &Options::default(), 10);
        let Body::Bars(b) = body else {
            panic!("expected bars");
        };
        let urgent = b.bars.iter().find(|x| x.label == "Urgent").unwrap();
        assert_eq!(urgent.value, 2);
    }

    #[test]
    fn render_badge_status_scales_with_count() {
        let one = view_for_test("E-1", 1, None, 0);
        let body = render_body(&[one], Shape::Badge, &Options::default(), 10);
        let Body::Badge(b) = body else {
            panic!("expected badge");
        };
        assert_eq!(b.status, Status::Ok);
        let many: Vec<IssueView> = (0..15).map(|i| view_for_test("E", 1, None, i)).collect();
        let body = render_body(&many, Shape::Badge, &Options::default(), 10);
        let Body::Badge(b) = body else {
            panic!("expected badge");
        };
        assert_eq!(b.status, Status::Error);
    }

    fn view_for_test(id: &str, priority: u8, due: Option<NaiveDate>, ts: i64) -> IssueView {
        IssueView {
            identifier: id.into(),
            title: "title".into(),
            priority,
            priority_label: match priority {
                1 => "Urgent".into(),
                2 => "High".into(),
                3 => "Medium".into(),
                4 => "Low".into(),
                _ => "No priority".into(),
            },
            state_name: "Todo".into(),
            state_type: "unstarted".into(),
            assignee: None,
            team_name: None,
            project: None,
            labels: vec![],
            due_date: due,
            url: format!("https://linear.app/acme/issue/{id}"),
            updated_at: ts,
        }
    }

    #[test]
    fn render_text_block_caps_at_limit() {
        let views: Vec<IssueView> = (0..15)
            .map(|i| view_for_test(&format!("ENG-{i}"), 1, None, 0))
            .collect();
        let body = render_body(&views, Shape::TextBlock, &Options::default(), 5);
        let Body::TextBlock(b) = body else {
            panic!("expected text_block");
        };
        assert_eq!(b.lines.len(), 5);
    }

    #[test]
    fn render_markdown_emboldens_identifier() {
        let views = vec![view_for_test("ENG-7", 2, None, 0)];
        let body = render_body(&views, Shape::MarkdownTextBlock, &Options::default(), 10);
        let Body::MarkdownTextBlock(b) = body else {
            panic!("expected markdown");
        };
        assert!(b.value.contains("**ENG-7**"), "got: {}", b.value);
        assert!(b.value.contains("*[Todo]*"), "got: {}", b.value);
    }

    #[test]
    fn render_entries_maps_state_type_to_status() {
        let mut started = view_for_test("ENG-1", 2, None, 0);
        started.state_type = "started".into();
        let mut completed = view_for_test("ENG-2", 2, None, 0);
        completed.state_type = "completed".into();
        let mut canceled = view_for_test("ENG-3", 2, None, 0);
        canceled.state_type = "canceled".into();
        let body = render_body(
            &[started, completed, canceled],
            Shape::Entries,
            &Options::default(),
            10,
        );
        let Body::Entries(e) = body else {
            panic!("expected entries");
        };
        assert_eq!(e.items[0].status, Some(Status::Warn));
        assert_eq!(e.items[1].status, Some(Status::Ok));
        assert_eq!(e.items[2].status, Some(Status::Error));
    }

    #[test]
    fn render_calendar_filters_due_dates_to_current_month() {
        let now = t::now_in(None).date_naive();
        let in_month = NaiveDate::from_ymd_opt(now.year(), now.month(), 5).unwrap();
        let last_year = NaiveDate::from_ymd_opt(now.year() - 1, now.month(), 9).unwrap();
        let views = vec![
            view_for_test("ENG-1", 1, Some(in_month), 0),
            view_for_test("ENG-2", 2, Some(last_year), 0),
            view_for_test("ENG-3", 3, None, 0),
        ];
        let body = render_body(&views, Shape::Calendar, &Options::default(), 10);
        let Body::Calendar(c) = body else {
            panic!("expected calendar");
        };
        assert_eq!(c.year, now.year());
        assert!(c.events.contains(&5));
        assert!(!c.events.contains(&9));
    }

    #[test]
    fn render_timeline_carries_state_status_and_priority_detail() {
        let mut started = view_for_test("ENG-1", 1, None, 1_700_000_000);
        started.state_type = "started".into();
        started.state_name = "In Progress".into();
        let body = render_body(&[started], Shape::Timeline, &Options::default(), 10);
        let Body::Timeline(t) = body else {
            panic!("expected timeline");
        };
        assert_eq!(t.events[0].status, Some(Status::Warn));
        assert_eq!(t.events[0].timestamp, 1_700_000_000);
        let detail = t.events[0].detail.as_deref().unwrap();
        assert!(detail.contains("In Progress"), "got: {detail}");
        assert!(detail.contains("P1"), "got: {detail}");
    }

    #[test]
    fn bars_group_by_status_uses_state_name() {
        let mut a = view_for_test("ENG-1", 1, None, 0);
        a.state_name = "In Progress".into();
        let mut b = view_for_test("ENG-2", 1, None, 0);
        b.state_name = "Todo".into();
        let mut c = view_for_test("ENG-3", 1, None, 0);
        c.state_name = "Todo".into();
        let opts = Options {
            group_by: Some("status".into()),
            ..Default::default()
        };
        let body = render_body(&[a, b, c], Shape::Bars, &opts, 10);
        let Body::Bars(bars) = body else {
            panic!("expected bars");
        };
        let todo = bars.bars.iter().find(|x| x.label == "Todo").unwrap();
        assert_eq!(todo.value, 2);
    }

    #[test]
    fn cmp_views_breaks_ties_with_due_date_then_recency() {
        let dec1 = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
        let dec5 = NaiveDate::from_ymd_opt(2026, 12, 5).unwrap();
        let earlier = view_for_test("ENG-A", 2, Some(dec5), 100);
        let later = view_for_test("ENG-B", 2, Some(dec1), 50);
        let mut list = [earlier, later];
        list.sort_by(cmp_views);
        // Same priority → earlier due date wins.
        assert_eq!(list[0].identifier, "ENG-B");
    }
}
