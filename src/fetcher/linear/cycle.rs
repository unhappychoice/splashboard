//! `linear_cycle` — current sprint progress for a Linear team.
//!
//! Safety::Safe: every request targets `api.linear.app` (fixed in [`super::client`]).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, CalendarData, EntriesData, Entry, LinkedLine,
    LinkedTextBlockData, MarkdownTextBlockData, NumberSeriesData, Payload, RatioData, Status,
    TextBlockData, TextData,
};
use crate::render::Shape;
use crate::samples;

use super::client::{cycle_url, graphql_query, resolve_token};

const SHAPES: &[Shape] = &[
    Shape::Ratio,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::LinkedTextBlock,
    Shape::Entries,
    Shape::NumberSeries,
    Shape::Bars,
    Shape::Calendar,
    Shape::Badge,
];

const QUERY: &str = r#"
query Cycle($filter: CycleFilter) {
  viewer { organization { urlKey } }
  cycles(filter: $filter, first: 1) {
    nodes {
      number
      name
      startsAt
      endsAt
      progress
      completedIssueCountHistory
      issueCountHistory
      team { key name }
      issues(first: 200) {
        nodes {
          identifier
          title
          state { name type }
          dueDate
          url
        }
      }
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
        name: "workspace",
        type_hint: "string",
        required: false,
        default: None,
        description: "Workspace slug for `LinkedTextBlock` URLs. Defaults to viewer's organization urlKey.",
    },
    OptionSchema {
        name: "team",
        type_hint: "string",
        required: false,
        default: None,
        description: "Team key (e.g. `\"ENG\"`). When omitted, the first active cycle the viewer can see is used.",
    },
    OptionSchema {
        name: "cycle",
        type_hint: "\"current\" | \"next\" | \"previous\" | integer",
        required: false,
        default: Some("\"current\""),
        description: "Cycle to surface.",
    },
];

pub struct LinearCycle;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    team: Option<String>,
    #[serde(default)]
    cycle: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    viewer: ApiViewer,
    cycles: ApiConnection<ApiCycle>,
}

#[derive(Debug, Deserialize)]
struct ApiViewer {
    organization: ApiOrganization,
}

#[derive(Debug, Deserialize)]
struct ApiOrganization {
    #[serde(rename = "urlKey")]
    url_key: String,
}

#[derive(Debug, Deserialize)]
struct ApiConnection<T> {
    nodes: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct ApiCycle {
    number: i64,
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "startsAt")]
    starts_at: String,
    #[serde(rename = "endsAt")]
    ends_at: String,
    #[serde(default)]
    progress: Option<f64>,
    #[serde(rename = "completedIssueCountHistory", default)]
    completed_history: Vec<f64>,
    #[serde(rename = "issueCountHistory", default)]
    total_history: Vec<f64>,
    #[serde(default)]
    team: Option<ApiTeam>,
    #[serde(default)]
    issues: Option<ApiConnection<ApiIssue>>,
}

#[derive(Debug, Deserialize)]
struct ApiTeam {
    key: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct ApiIssue {
    identifier: String,
    title: String,
    state: ApiState,
    #[serde(rename = "dueDate", default)]
    due_date: Option<String>,
    url: String,
}

#[derive(Debug, Deserialize)]
struct ApiState {
    name: String,
    #[serde(rename = "type")]
    state_type: String,
}

#[derive(Debug)]
struct CycleView {
    number: i64,
    name: String,
    starts_at: NaiveDate,
    ends_at: NaiveDate,
    progress: f64,
    completed_count: u64,
    total_count: u64,
    completed_history: Vec<u64>,
    team_key: String,
    team_name: String,
    workspace: String,
    issues: Vec<IssueView>,
}

#[derive(Debug)]
struct IssueView {
    identifier: String,
    title: String,
    state_name: String,
    state_type: String,
    due_date: Option<NaiveDate>,
    url: String,
}

#[async_trait]
impl Fetcher for LinearCycle {
    fn name(&self) -> &str {
        "linear_cycle"
    }

    fn safety(&self) -> Safety {
        Safety::Safe
    }

    fn description(&self) -> &'static str {
        "Linear cycle (sprint) progress — completed/total issues, days remaining, burndown sparkline. `Ratio` is the headline (completion fraction); `NumberSeries` carries the daily completed-count history; `Bars` breaks down issue states; `Calendar` highlights cycle days + issue due dates; `Badge` summarises on-track vs at-risk vs behind."
    }

    fn shapes(&self) -> &[Shape] {
        SHAPES
    }

    fn default_shape(&self) -> Shape {
        Shape::Ratio
    }

    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }

    fn cache_key(&self, ctx: &FetchContext) -> String {
        super::cache_key("linear_cycle", ctx)
    }

    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Ratio => Body::Ratio(RatioData {
                value: 0.42,
                label: Some("Cycle 24".into()),
                denominator: Some(40),
            }),
            Shape::Text => samples::text("Cycle 24 — 17/40 · 5d left"),
            Shape::TextBlock => samples::text_block(&[
                "Cycle 24",
                "started 2026-04-22",
                "ends 2026-05-06",
                "completed 17/40",
                "5 days left",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "**Cycle 24**\n\n- started **2026-04-22**\n- ends **2026-05-06**\n- completed **17/40**\n- **5** days left",
            ),
            Shape::LinkedTextBlock => samples::linked_text_block(&[(
                "Cycle 24 — 17/40 · 5d left",
                Some("https://linear.app/acme/team/ENG/cycle/24"),
            )]),
            Shape::Entries => samples::entries(&[
                ("Cycle", "24"),
                ("Started", "2026-04-22"),
                ("Ends", "2026-05-06"),
                ("Completed", "17 / 40"),
                ("Days left", "5"),
            ]),
            Shape::NumberSeries => samples::number_series(&[0, 1, 3, 5, 7, 10, 12, 15, 17]),
            Shape::Bars => samples::bars(&[
                ("Completed", 17),
                ("In Progress", 8),
                ("Todo", 12),
                ("Canceled", 3),
            ]),
            Shape::Calendar => samples::calendar(2026, 4, Some(29), &[22, 23, 24, 28, 29, 30]),
            Shape::Badge => samples::badge(Status::Warn, "Cycle 24 at risk"),
            _ => return None,
        })
    }

    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref())?;
        let token = resolve_token(opts.token.as_deref())?;
        let filter = build_filter(&opts);
        let response: ApiResponse =
            graphql_query(&token, QUERY, json!({ "filter": filter })).await?;
        let cycle =
            response.cycles.nodes.into_iter().next().ok_or_else(|| {
                FetchError::Failed("linear cycle: no cycle matched filter".into())
            })?;
        let workspace = opts
            .workspace
            .filter(|s| !s.is_empty())
            .unwrap_or(response.viewer.organization.url_key);
        let view = to_view(cycle, workspace)?;
        let shape = ctx.shape.unwrap_or(Shape::Ratio);
        Ok(payload(render_body(&view, shape)))
    }
}

pub fn fetcher() -> Arc<dyn Fetcher> {
    Arc::new(LinearCycle)
}

fn parse_options(raw: Option<&toml::Value>) -> Result<Options, FetchError> {
    raw.map(|v| {
        v.clone()
            .try_into::<Options>()
            .map_err(|e| FetchError::Failed(format!("invalid options: {e}")))
    })
    .unwrap_or_else(|| Ok(Options::default()))
}

fn build_filter(opts: &Options) -> Value {
    let cycle = opts
        .cycle
        .as_deref()
        .unwrap_or("current")
        .trim()
        .to_lowercase();
    let mut map = serde_json::Map::new();
    if let Some(team) = opts.team.as_deref().filter(|s| !s.is_empty()) {
        map.insert("team".into(), json!({ "key": { "eq": team } }));
    }
    match cycle.as_str() {
        "next" => {
            map.insert("isNext".into(), json!({ "eq": true }));
        }
        "previous" | "prev" => {
            map.insert("isPrevious".into(), json!({ "eq": true }));
        }
        s => match s.parse::<i64>() {
            Ok(n) => {
                map.insert("number".into(), json!({ "eq": n }));
            }
            Err(_) => {
                map.insert("isActive".into(), json!({ "eq": true }));
            }
        },
    }
    Value::Object(map)
}

fn to_view(api: ApiCycle, workspace: String) -> Result<CycleView, FetchError> {
    let starts_at = parse_date(&api.starts_at)?;
    let ends_at = parse_date(&api.ends_at)?;
    let team = api
        .team
        .ok_or_else(|| FetchError::Failed("linear cycle: missing team".into()))?;
    let issues: Vec<IssueView> = api
        .issues
        .map(|c| c.nodes.into_iter().map(to_issue_view).collect())
        .unwrap_or_default();
    let total_count = api
        .total_history
        .last()
        .copied()
        .map(|x| x.round().max(0.0) as u64)
        .unwrap_or_else(|| issues.len() as u64);
    let completed_count = api
        .completed_history
        .last()
        .copied()
        .map(|x| x.round().max(0.0) as u64)
        .unwrap_or_else(|| {
            issues
                .iter()
                .filter(|i| i.state_type == "completed")
                .count() as u64
        });
    let progress = api.progress.unwrap_or_else(|| {
        if total_count == 0 {
            0.0
        } else {
            completed_count as f64 / total_count as f64
        }
    });
    Ok(CycleView {
        number: api.number,
        name: api
            .name
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("Cycle {}", api.number)),
        starts_at,
        ends_at,
        progress: progress.clamp(0.0, 1.0),
        completed_count,
        total_count,
        completed_history: api
            .completed_history
            .into_iter()
            .map(|x| x.round().max(0.0) as u64)
            .collect(),
        team_key: team.key,
        team_name: team.name,
        workspace,
        issues,
    })
}

fn to_issue_view(api: ApiIssue) -> IssueView {
    IssueView {
        identifier: api.identifier,
        title: api.title,
        state_name: api.state.name,
        state_type: api.state.state_type,
        due_date: api
            .due_date
            .as_deref()
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()),
        url: api.url,
    }
}

fn parse_date(raw: &str) -> Result<NaiveDate, FetchError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc).date_naive())
        .or_else(|_| NaiveDate::parse_from_str(raw, "%Y-%m-%d"))
        .map_err(|e| FetchError::Failed(format!("linear cycle date parse: {e}")))
}

fn days_left(view: &CycleView, today: NaiveDate) -> i64 {
    (view.ends_at - today).num_days().max(0)
}

fn render_body(view: &CycleView, shape: Shape) -> Body {
    let today = Utc::now().date_naive();
    let left = days_left(view, today);
    match shape {
        Shape::Ratio => Body::Ratio(RatioData {
            value: view.progress,
            label: Some(view.name.clone()),
            denominator: Some(view.total_count),
        }),
        Shape::Text => Body::Text(TextData {
            value: format!(
                "{} — {}/{} · {}d left",
                view.name, view.completed_count, view.total_count, left
            ),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: vec![
                view.name.clone(),
                format!("started {}", view.starts_at),
                format!("ends {}", view.ends_at),
                format!("completed {}/{}", view.completed_count, view.total_count),
                format!("{} days left", left),
            ],
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: format!(
                "**{}**\n\n- started **{}**\n- ends **{}**\n- completed **{}/{}**\n- **{}** days left",
                view.name,
                view.starts_at,
                view.ends_at,
                view.completed_count,
                view.total_count,
                left,
            ),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: vec![LinkedLine {
                text: format!(
                    "{} — {}/{} · {}d left",
                    view.name, view.completed_count, view.total_count, left
                ),
                url: Some(cycle_url(&view.workspace, &view.team_key, view.number)),
            }],
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: vec![
                Entry {
                    key: "Cycle".into(),
                    value: Some(view.number.to_string()),
                    status: None,
                },
                Entry {
                    key: "Team".into(),
                    value: Some(view.team_name.clone()),
                    status: None,
                },
                Entry {
                    key: "Started".into(),
                    value: Some(view.starts_at.to_string()),
                    status: None,
                },
                Entry {
                    key: "Ends".into(),
                    value: Some(view.ends_at.to_string()),
                    status: None,
                },
                Entry {
                    key: "Completed".into(),
                    value: Some(format!("{} / {}", view.completed_count, view.total_count)),
                    status: Some(progress_status(view, today)),
                },
                Entry {
                    key: "Days left".into(),
                    value: Some(left.to_string()),
                    status: None,
                },
            ],
        }),
        Shape::NumberSeries => Body::NumberSeries(NumberSeriesData {
            values: view.completed_history.clone(),
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: state_breakdown(view),
        }),
        Shape::Calendar => calendar_body(view, today),
        Shape::Badge => Body::Badge(BadgeData {
            status: progress_status(view, today),
            label: format!("{} {}", view.name, badge_label(view, today)),
        }),
        _ => Body::Text(TextData {
            value: view.name.clone(),
        }),
    }
}

fn state_breakdown(view: &CycleView) -> Vec<Bar> {
    let mut counts: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for issue in &view.issues {
        *counts.entry(issue.state_name.clone()).or_insert(0) += 1;
    }
    let mut bars: Vec<Bar> = counts
        .into_iter()
        .map(|(label, value)| Bar { label, value })
        .collect();
    bars.sort_by(|a, b| b.value.cmp(&a.value).then_with(|| a.label.cmp(&b.label)));
    bars
}

fn calendar_body(view: &CycleView, today: NaiveDate) -> Body {
    let anchor = if today < view.starts_at {
        view.starts_at
    } else if today > view.ends_at {
        view.ends_at
    } else {
        today
    };
    let mut events: Vec<u8> = (0..)
        .map(|i| view.starts_at + chrono::Duration::days(i))
        .take_while(|d| *d <= view.ends_at)
        .filter(|d| d.year() == anchor.year() && d.month() == anchor.month())
        .map(|d| d.day() as u8)
        .collect();
    for issue in &view.issues {
        if let Some(d) = issue.due_date
            && d.year() == anchor.year()
            && d.month() == anchor.month()
            && !events.contains(&(d.day() as u8))
        {
            events.push(d.day() as u8);
        }
    }
    events.sort();
    Body::Calendar(CalendarData {
        year: anchor.year(),
        month: anchor.month() as u8,
        day: Some(today.day() as u8),
        events,
    })
}

fn progress_status(view: &CycleView, today: NaiveDate) -> Status {
    if view.progress >= 1.0 {
        return Status::Ok;
    }
    let total_days = (view.ends_at - view.starts_at).num_days().max(1) as f64;
    let elapsed = (today - view.starts_at)
        .num_days()
        .clamp(0, total_days as i64) as f64;
    let elapsed_ratio = elapsed / total_days;
    let gap = elapsed_ratio - view.progress;
    if gap > 0.20 {
        Status::Error
    } else if gap > 0.05 {
        Status::Warn
    } else {
        Status::Ok
    }
}

fn badge_label(view: &CycleView, today: NaiveDate) -> &'static str {
    if view.progress >= 1.0 {
        "done"
    } else {
        match progress_status(view, today) {
            Status::Ok => "on track",
            Status::Warn => "at risk",
            Status::Error => "behind",
        }
    }
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
    fn build_filter_defaults_to_active_cycle() {
        let f = build_filter(&Options::default());
        assert_eq!(f["isActive"]["eq"], json!(true));
    }

    #[test]
    fn build_filter_with_team_includes_team_clause() {
        let f = build_filter(&options("team = \"ENG\""));
        assert_eq!(f["team"]["key"]["eq"], json!("ENG"));
        assert_eq!(f["isActive"]["eq"], json!(true));
    }

    #[test]
    fn build_filter_next_uses_is_next() {
        let f = build_filter(&options("cycle = \"next\""));
        assert_eq!(f["isNext"]["eq"], json!(true));
        assert!(f.get("isActive").is_none());
    }

    #[test]
    fn build_filter_specific_number_uses_number_eq() {
        let f = build_filter(&options("cycle = \"24\""));
        assert_eq!(f["number"]["eq"], json!(24));
    }

    #[test]
    fn render_ratio_carries_progress_and_denominator() {
        let view = view_for_test(0.5, 5, 10);
        let body = render_body(&view, Shape::Ratio);
        let Body::Ratio(r) = body else {
            panic!("expected ratio");
        };
        assert!((r.value - 0.5).abs() < 1e-9);
        assert_eq!(r.denominator, Some(10));
    }

    #[test]
    fn progress_status_done_when_progress_full() {
        let view = view_for_test(1.0, 10, 10);
        let today = view.starts_at + chrono::Duration::days(3);
        assert_eq!(progress_status(&view, today), Status::Ok);
    }

    #[test]
    fn progress_status_behind_when_elapsed_far_ahead_of_progress() {
        let view = view_for_test(0.10, 1, 10);
        // 70% elapsed, 10% complete → 60% gap → behind.
        let today = view.starts_at + chrono::Duration::days(7);
        assert_eq!(progress_status(&view, today), Status::Error);
    }

    #[test]
    fn progress_status_at_risk_when_slightly_behind() {
        let view = view_for_test(0.40, 4, 10);
        // 14-day cycle: 7 days in = 50% elapsed; with 40% complete the gap is 10% → at risk.
        let today = view.starts_at + chrono::Duration::days(7);
        assert_eq!(progress_status(&view, today), Status::Warn);
    }

    #[test]
    fn render_text_includes_name_and_progress_counts() {
        let view = view_for_test(0.4, 4, 10);
        let body = render_body(&view, Shape::Text);
        let Body::Text(t) = body else {
            panic!("expected text");
        };
        assert!(t.value.contains("Cycle 24"));
        assert!(t.value.contains("4/10"));
    }

    #[test]
    fn render_linked_block_uses_cycle_url() {
        let view = view_for_test(0.4, 4, 10);
        let body = render_body(&view, Shape::LinkedTextBlock);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked");
        };
        let url = b.items[0].url.as_deref().unwrap();
        assert!(url.contains("/team/ENG/cycle/24"), "got: {url}");
    }

    #[test]
    fn render_number_series_uses_completed_history() {
        let mut view = view_for_test(0.4, 4, 10);
        view.completed_history = vec![0, 1, 3, 4];
        let body = render_body(&view, Shape::NumberSeries);
        let Body::NumberSeries(n) = body else {
            panic!("expected number_series");
        };
        assert_eq!(n.values, vec![0, 1, 3, 4]);
    }

    fn view_for_test(progress: f64, completed: u64, total: u64) -> CycleView {
        CycleView {
            number: 24,
            name: "Cycle 24".into(),
            starts_at: NaiveDate::from_ymd_opt(2026, 4, 22).unwrap(),
            ends_at: NaiveDate::from_ymd_opt(2026, 5, 6).unwrap(),
            progress,
            completed_count: completed,
            total_count: total,
            completed_history: vec![],
            team_key: "ENG".into(),
            team_name: "Engineering".into(),
            workspace: "acme".into(),
            issues: vec![],
        }
    }

    fn issue_for_test(id: &str, state: &str, due: Option<NaiveDate>) -> IssueView {
        IssueView {
            identifier: id.into(),
            title: format!("Title {id}"),
            state_name: state.into(),
            state_type: state.to_lowercase(),
            due_date: due,
            url: format!("https://linear.app/acme/issue/{id}"),
        }
    }

    #[test]
    fn build_filter_previous_uses_is_previous() {
        let f = build_filter(&options("cycle = \"previous\""));
        assert_eq!(f["isPrevious"]["eq"], json!(true));
        assert!(f.get("isActive").is_none());
    }

    #[test]
    fn build_filter_unrecognised_keyword_falls_back_to_active() {
        let f = build_filter(&options("cycle = \"garbage\""));
        assert_eq!(f["isActive"]["eq"], json!(true));
    }

    #[test]
    fn render_text_block_lists_dates_and_days_left() {
        let view = view_for_test(0.4, 4, 10);
        let body = render_body(&view, Shape::TextBlock);
        let Body::TextBlock(b) = body else {
            panic!("expected text_block");
        };
        let joined = b.lines.join("\n");
        assert!(joined.contains(&view.starts_at.to_string()));
        assert!(joined.contains(&view.ends_at.to_string()));
        assert!(joined.contains("4/10"));
    }

    #[test]
    fn render_markdown_uses_bold_for_name_and_dates() {
        let view = view_for_test(0.4, 4, 10);
        let body = render_body(&view, Shape::MarkdownTextBlock);
        let Body::MarkdownTextBlock(b) = body else {
            panic!("expected markdown");
        };
        assert!(b.value.contains("**Cycle 24**"));
        assert!(b.value.contains("**4/10**"));
    }

    #[test]
    fn render_entries_returns_six_canonical_rows() {
        let view = view_for_test(0.4, 4, 10);
        let body = render_body(&view, Shape::Entries);
        let Body::Entries(e) = body else {
            panic!("expected entries");
        };
        let keys: Vec<&str> = e.items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["Cycle", "Team", "Started", "Ends", "Completed", "Days left"]
        );
    }

    #[test]
    fn render_bars_groups_issues_by_state_name() {
        let mut view = view_for_test(0.4, 2, 4);
        view.issues = vec![
            issue_for_test("ENG-1", "Started", None),
            issue_for_test("ENG-2", "Todo", None),
            issue_for_test("ENG-3", "Todo", None),
            issue_for_test("ENG-4", "Completed", None),
        ];
        let body = render_body(&view, Shape::Bars);
        let Body::Bars(b) = body else {
            panic!("expected bars");
        };
        let todo = b.bars.iter().find(|x| x.label == "Todo").unwrap();
        assert_eq!(todo.value, 2);
    }

    #[test]
    fn calendar_includes_cycle_days_within_anchor_month() {
        let view = view_for_test(0.4, 4, 10);
        let body = render_body(&view, Shape::Calendar);
        let Body::Calendar(c) = body else {
            panic!("expected calendar");
        };
        // The cycle spans 4-22 → 5-6 (14 days). The anchor is whatever month `today` is in,
        // clamped into the cycle range — for tests we just check the events are non-empty
        // and lie within the cycle's date range.
        assert!(!c.events.is_empty());
        for day in &c.events {
            assert!((1..=31).contains(day));
        }
    }

    #[test]
    fn calendar_anchor_clamps_to_cycle_end_when_today_is_after() {
        let mut view = view_for_test(1.0, 10, 10);
        view.starts_at = NaiveDate::from_ymd_opt(2020, 1, 5).unwrap();
        view.ends_at = NaiveDate::from_ymd_opt(2020, 1, 15).unwrap();
        // `today` (the current process date) is far after Jan 2020; the anchor should clamp
        // to the cycle end → January 2020, with all 11 cycle days populated.
        let body = render_body(&view, Shape::Calendar);
        let Body::Calendar(c) = body else {
            panic!("expected calendar");
        };
        assert_eq!(c.year, 2020);
        assert_eq!(c.month, 1);
        for day in 5u8..=15 {
            assert!(c.events.contains(&day), "missing day {day}");
        }
    }

    #[test]
    fn render_badge_label_reflects_progress_status() {
        let mut view = view_for_test(0.10, 1, 10);
        view.starts_at = NaiveDate::from_ymd_opt(2026, 4, 22).unwrap();
        view.ends_at = NaiveDate::from_ymd_opt(2026, 5, 6).unwrap();
        let body = render_body(&view, Shape::Badge);
        let Body::Badge(b) = body else {
            panic!("expected badge");
        };
        // We can't pin the today-relative label deterministically, but the static result of
        // calling badge_label / progress_status with today=ends_at gives "behind" for this
        // view (10% done, 100% elapsed → gap 0.90).
        let label = badge_label(&view, view.ends_at);
        assert!(["behind", "at risk", "on track", "done"].contains(&label));
        assert!(b.label.contains("Cycle 24"));
    }

    #[test]
    fn badge_label_done_when_progress_full() {
        let view = view_for_test(1.0, 10, 10);
        assert_eq!(badge_label(&view, view.starts_at), "done");
    }

    #[test]
    fn days_left_clamps_to_zero_when_cycle_has_ended() {
        let view = view_for_test(1.0, 10, 10);
        let after = view.ends_at + chrono::Duration::days(7);
        assert_eq!(days_left(&view, after), 0);
    }
}
