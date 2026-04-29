//! `linear_notifications` — Linear inbox snapshot with structured filters.
//!
//! Safety::Safe: every request targets `api.linear.app` (fixed in [`super::client`]).

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData,
    MarkdownTextBlockData, Payload, Status, TextBlockData, TextData, TimelineData, TimelineEvent,
};
use crate::render::Shape;
use crate::samples;

use super::client::{graphql_query, resolve_token};

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::Badge,
    Shape::Timeline,
];

const DEFAULT_LIMIT: usize = 10;
const MAX_LIMIT: usize = 100;
const FETCH_LIMIT: i64 = 50;

// Linear's `NotificationFilter` does not expose `readAt`, so the read-state filter is applied
// client-side (see [`matches_read`]). We pull a wider page (`FETCH_LIMIT = 50`) and slice
// after filtering; that gives `filter_read = "unread"` (the default daily-driver lens) enough
// headroom to surface the top N unread items even when the inbox is mostly read traffic.
//
// `actor` lives on each concrete subtype rather than the `Notification` interface, so we pull
// it under each fragment we render. Subtypes we don't fragment on (`OauthClientApproval...`)
// surface as type-only rows without an actor — better than crashing the query. `comment.body`
// is the fallback content for PR / review notifications where Linear's GitHub integration
// emits an `IssueNotification` whose `actor` is the GitHub user (no Linear `User` row, so
// `displayName` is null) — at least the comment body still gives the row some context.
const QUERY: &str = r#"
query Notifications($first: Int) {
  notifications(first: $first) {
    nodes {
      id
      type
      readAt
      createdAt
      ... on IssueNotification {
        actor { displayName email }
        issue { identifier title url team { key } }
        comment { body }
      }
      ... on ProjectNotification {
        actor { displayName email }
        project { name url }
      }
      ... on PullRequestNotification {
        actor { displayName email }
        pullRequest { number title url }
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
        name: "filter_read",
        type_hint: "\"unread\" | \"read\" | \"all\"",
        required: false,
        default: Some("\"unread\""),
        description: "Read-state filter.",
    },
    OptionSchema {
        name: "filter_type",
        type_hint: "\"mention\" | \"assigned\" | \"comment\" | \"status_changed\" | \"project_update\" | \"any\"",
        required: false,
        default: Some("\"any\""),
        description: "Notification type filter (matched on the API `type` string).",
    },
    OptionSchema {
        name: "filter_team",
        type_hint: "string",
        required: false,
        default: None,
        description: "Team key (e.g. `\"ENG\"`) — keeps only issue notifications from the team.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=100)",
        required: false,
        default: Some("10"),
        description: "Max rows for list-shaped renderers.",
    },
];

pub struct LinearNotifications;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    filter_read: Option<String>,
    #[serde(default)]
    filter_type: Option<String>,
    #[serde(default)]
    filter_team: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    notifications: ApiConnection<ApiNotification>,
}

#[derive(Debug, Deserialize)]
struct ApiConnection<T> {
    nodes: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct ApiNotification {
    #[serde(rename = "type")]
    notif_type: String,
    #[serde(rename = "readAt", default)]
    read_at: Option<String>,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(default)]
    actor: Option<ApiUser>,
    #[serde(default)]
    issue: Option<ApiIssue>,
    #[serde(default)]
    project: Option<ApiProject>,
    #[serde(default)]
    comment: Option<ApiComment>,
    #[serde(rename = "pullRequest", default)]
    pull_request: Option<ApiPullRequest>,
}

#[derive(Debug, Deserialize)]
struct ApiPullRequest {
    number: i64,
    title: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct ApiProject {
    name: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct ApiComment {
    #[serde(default)]
    body: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiUser {
    #[serde(rename = "displayName", default)]
    display_name: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiIssue {
    identifier: String,
    title: String,
    url: String,
    #[serde(default)]
    team: Option<ApiTeam>,
}

#[derive(Debug, Deserialize)]
struct ApiTeam {
    key: String,
}

#[derive(Debug)]
struct NotifView {
    notif_type: String,
    type_label: String,
    actor: Option<String>,
    issue_identifier: Option<String>,
    issue_title: Option<String>,
    issue_url: Option<String>,
    team_key: Option<String>,
    /// Comment body when the notification carries one (notifications about comments,
    /// PR activity, etc.). Stripped to the first non-empty line so multi-paragraph
    /// review comments don't blow up the row height.
    snippet: Option<String>,
    created_ts: i64,
    is_unread: bool,
}

#[async_trait]
impl Fetcher for LinearNotifications {
    fn name(&self) -> &str {
        "linear_notifications"
    }

    fn safety(&self) -> Safety {
        Safety::Safe
    }

    fn description(&self) -> &'static str {
        "Linear inbox snapshot — mentions, assignments, comments, status changes — with structured filters for read state, type, and team. List shapes link each row to the originating issue."
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
        super::cache_key("linear_notifications", ctx)
    }

    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "ENG-123 @sarah mentioned you · 2h ago",
                    Some("https://linear.app/acme/issue/ENG-123"),
                ),
                (
                    "ENG-118 assigned by @max · yesterday",
                    Some("https://linear.app/acme/issue/ENG-118"),
                ),
            ]),
            Shape::Text => samples::text("3 unread"),
            Shape::TextBlock => samples::text_block(&[
                "ENG-123 @sarah mentioned you · 2h ago",
                "ENG-118 assigned by @max · yesterday",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "- **ENG-123** *mention* by @sarah · 2h ago\n- **ENG-118** *assigned* by @max · yesterday",
            ),
            Shape::Entries => samples::entries(&[
                ("ENG-123", "@sarah mentioned you"),
                ("ENG-118", "assigned by @max"),
            ]),
            Shape::Bars => samples::bars(&[("Mention", 2), ("Assigned", 1)]),
            Shape::Badge => samples::badge(Status::Warn, "linear inbox 3"),
            Shape::Timeline => samples::timeline(&[
                (
                    1_745_896_800,
                    "ENG-123 mention by @sarah",
                    Some("Fix login flow"),
                ),
                (
                    1_745_810_400,
                    "ENG-118 assigned by @max",
                    Some("Polish dashboard"),
                ),
            ]),
            _ => return None,
        })
    }

    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref())?;
        let token = resolve_token(opts.token.as_deref())?;
        let response: ApiResponse =
            graphql_query(&token, QUERY, json!({ "first": FETCH_LIMIT })).await?;
        let mut views: Vec<NotifView> = response
            .notifications
            .nodes
            .into_iter()
            .map(to_view)
            .filter(|v| matches_read(v, opts.filter_read.as_deref()))
            .filter(|v| matches_type(v, opts.filter_type.as_deref()))
            .filter(|v| matches_team(v, opts.filter_team.as_deref()))
            .collect();
        views.sort_by_key(|v| std::cmp::Reverse(v.created_ts));
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        Ok(payload(render_body(&views, shape, limit)))
    }
}

pub fn fetcher() -> Arc<dyn Fetcher> {
    Arc::new(LinearNotifications)
}

fn parse_options(raw: Option<&toml::Value>) -> Result<Options, FetchError> {
    raw.map(|v| {
        v.clone()
            .try_into::<Options>()
            .map_err(|e| FetchError::Failed(format!("invalid options: {e}")))
    })
    .unwrap_or_else(|| Ok(Options::default()))
}

fn matches_read(v: &NotifView, raw: Option<&str>) -> bool {
    match raw.unwrap_or("unread").trim().to_lowercase().as_str() {
        "read" => !v.is_unread,
        "all" => true,
        _ => v.is_unread,
    }
}

fn matches_type(v: &NotifView, raw: Option<&str>) -> bool {
    let kind = raw.unwrap_or("any").trim().to_lowercase();
    if kind == "any" {
        return true;
    }
    let t = v.notif_type.to_lowercase();
    match kind.as_str() {
        "mention" => t.contains("mention"),
        "assigned" => t.contains("assigned"),
        "comment" => t.contains("comment"),
        "status_changed" => t.contains("status"),
        "project_update" => t.starts_with("projectupdate"),
        _ => true,
    }
}

fn matches_team(v: &NotifView, raw: Option<&str>) -> bool {
    let Some(team) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    v.team_key.as_deref().is_some_and(|k| k == team)
}

fn to_view(api: ApiNotification) -> NotifView {
    let type_label = pretty_type(&api.notif_type);
    let actor = api.actor.and_then(|a| a.display_name.or(a.email));
    let (subject_id, subject_title, subject_url, team_key) = if let Some(i) = api.issue {
        (
            Some(i.identifier),
            Some(i.title),
            Some(i.url),
            i.team.map(|t| t.key),
        )
    } else if let Some(pr) = api.pull_request {
        let repo = repo_from_pr_url(&pr.url);
        let id = match repo {
            Some(r) => format!("{r}#{}", pr.number),
            None => format!("PR #{}", pr.number),
        };
        (Some(id), Some(pr.title), Some(pr.url), None)
    } else if let Some(p) = api.project {
        (None, Some(p.name), Some(p.url), None)
    } else {
        (None, None, None, None)
    };
    let snippet = api.comment.and_then(|c| c.body).and_then(short_snippet);
    NotifView {
        notif_type: api.notif_type,
        type_label,
        actor,
        issue_identifier: subject_id,
        issue_title: subject_title,
        issue_url: subject_url,
        team_key,
        snippet,
        created_ts: parse_ts(&api.created_at),
        is_unread: api.read_at.is_none(),
    }
}

/// Parse the `<owner>/<repo>` slug from a GitHub PR URL like
/// `https://github.com/acme/widgets/pull/123`. Linear's `PullRequest` type doesn't expose
/// the repo name as a field, so the URL is the only place that information lives.
fn repo_from_pr_url(url: &str) -> Option<String> {
    let path = url.split("github.com/").nth(1)?;
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if owner.is_empty() || repo.is_empty() {
        None
    } else {
        Some(format!("{owner}/{repo}"))
    }
}

/// Reduce a comment body to a single-line snippet ≤ 80 chars. Returns `None` for empty input
/// so downstream rendering can drop the part rather than emit a `· · ·` separator chain.
fn short_snippet(body: String) -> Option<String> {
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())?
        .to_string();
    if line.chars().count() > 80 {
        Some(format!("{}…", line.chars().take(79).collect::<String>()))
    } else {
        Some(line)
    }
}

fn pretty_type(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("pullrequestreview") || lower.contains("reviewrequested") {
        "PR review".into()
    } else if lower.contains("pullrequest") {
        "PR".into()
    } else if lower.contains("mention") {
        "mention".into()
    } else if lower.contains("assigned") {
        "assigned".into()
    } else if lower.contains("comment") {
        "comment".into()
    } else if lower.contains("status") {
        "status".into()
    } else if lower.starts_with("projectupdate") {
        "project update".into()
    } else if lower.contains("subscribed") {
        "subscribed".into()
    } else if lower.contains("created") {
        "created".into()
    } else if lower.contains("due") || lower.contains("reminder") {
        "reminder".into()
    } else {
        // Camel-case → spaced lowercase so an unknown `issueDoneSomething` reads as
        // `"issue done something"` instead of leaking the raw API token.
        humanize_camel(raw)
    }
}

fn humanize_camel(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 4);
    for (i, ch) in raw.chars().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            out.push(' ');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

fn parse_ts(raw: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.timestamp())
        .unwrap_or(0)
}

fn render_body(views: &[NotifView], shape: Shape, limit: usize) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: format!("{} unread", views.iter().filter(|v| v.is_unread).count()),
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
                    url: v.issue_url.clone(),
                })
                .collect(),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: views
                .iter()
                .take(limit)
                .map(|v| Entry {
                    key: v
                        .issue_identifier
                        .clone()
                        .unwrap_or_else(|| v.type_label.clone()),
                    value: Some(entry_value(v)),
                    status: v.is_unread.then_some(Status::Warn),
                })
                .collect(),
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: tally_by_type(views),
        }),
        Shape::Badge => Body::Badge(BadgeData {
            status: badge_status(views),
            label: format!(
                "linear inbox {}",
                views.iter().filter(|v| v.is_unread).count()
            ),
        }),
        Shape::Timeline => Body::Timeline(TimelineData {
            events: views
                .iter()
                .take(limit)
                .map(|v| TimelineEvent {
                    timestamp: v.created_ts,
                    title: timeline_title(v),
                    detail: v.issue_title.clone(),
                    status: v.is_unread.then_some(Status::Warn),
                })
                .collect(),
        }),
        _ => Body::TextBlock(TextBlockData {
            lines: views.iter().take(limit).map(line_for).collect(),
        }),
    }
}

fn line_for(v: &NotifView) -> String {
    join_parts(&[
        v.issue_identifier.clone(),
        Some(v.type_label.clone()),
        v.actor.as_deref().map(|a| format!("by @{a}")),
        v.issue_title.clone().or_else(|| v.snippet.clone()),
    ])
}

fn markdown_line_for(v: &NotifView) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(id) = &v.issue_identifier {
        parts.push(format!("**{id}**"));
    }
    parts.push(format!("*{}*", v.type_label));
    if let Some(a) = &v.actor {
        parts.push(format!("by @{a}"));
    }
    if let Some(title) = v.issue_title.as_ref().or(v.snippet.as_ref()) {
        parts.push(title.clone());
    }
    format!("- {}", parts.join(" · "))
}

fn entry_value(v: &NotifView) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(a) = &v.actor {
        parts.push(format!("@{a}"));
    }
    parts.push(v.type_label.clone());
    if let Some(title) = v.issue_title.as_ref().or(v.snippet.as_ref()) {
        parts.push(title.clone());
    }
    parts.join(" · ")
}

fn timeline_title(v: &NotifView) -> String {
    line_for(v)
}

fn join_parts(parts: &[Option<String>]) -> String {
    parts
        .iter()
        .filter_map(|p| p.as_deref().filter(|s| !s.is_empty()))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn tally_by_type(views: &[NotifView]) -> Vec<Bar> {
    let mut counts: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for v in views {
        let label = capitalize(&v.type_label);
        *counts.entry(label).or_insert(0) += 1;
    }
    let mut bars: Vec<Bar> = counts
        .into_iter()
        .map(|(label, value)| Bar { label, value })
        .collect();
    bars.sort_by(|a, b| b.value.cmp(&a.value).then_with(|| a.label.cmp(&b.label)));
    bars
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect::<String>())
        .unwrap_or_default()
}

fn badge_status(views: &[NotifView]) -> Status {
    let unread = views.iter().filter(|v| v.is_unread).count();
    match unread {
        0 => Status::Ok,
        1..=3 => Status::Ok,
        4..=9 => Status::Warn,
        _ => Status::Error,
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

    #[test]
    fn matches_read_unread_default_keeps_only_unread_rows() {
        let unread = view_for_test("issueMention", true, Some("E-1"));
        let read = view_for_test("issueMention", false, Some("E-2"));
        assert!(matches_read(&unread, None));
        assert!(!matches_read(&read, None));
    }

    #[test]
    fn matches_read_read_inverts_the_filter() {
        let unread = view_for_test("issueMention", true, Some("E-1"));
        let read = view_for_test("issueMention", false, Some("E-2"));
        assert!(!matches_read(&unread, Some("read")));
        assert!(matches_read(&read, Some("read")));
    }

    #[test]
    fn matches_read_all_keeps_everything() {
        let unread = view_for_test("issueMention", true, Some("E-1"));
        let read = view_for_test("issueMention", false, Some("E-2"));
        assert!(matches_read(&unread, Some("all")));
        assert!(matches_read(&read, Some("all")));
    }

    #[test]
    fn pretty_type_collapses_known_synonyms() {
        assert_eq!(pretty_type("issueMention"), "mention");
        assert_eq!(pretty_type("issueAssignedToYou"), "assigned");
        assert_eq!(pretty_type("issueCommentMention"), "mention");
        assert_eq!(pretty_type("issueStatusChanged"), "status");
        assert_eq!(pretty_type("projectUpdateCreated"), "project update");
    }

    #[test]
    fn matches_type_filters_by_substring() {
        let v = view_for_test("issueMention", true, Some("ENG-1"));
        assert!(matches_type(&v, Some("mention")));
        assert!(!matches_type(&v, Some("assigned")));
        assert!(matches_type(&v, Some("any")));
    }

    #[test]
    fn matches_team_keeps_only_specified_team() {
        let v = view_for_test("issueMention", true, Some("ENG-1"));
        assert!(matches_team(&v, None));
        assert!(matches_team(&v, Some("ENG")));
        assert!(!matches_team(&v, Some("OPS")));
    }

    #[test]
    fn render_text_emits_unread_count() {
        let views = vec![
            view_for_test("issueMention", true, Some("E-1")),
            view_for_test("issueMention", false, Some("E-2")),
        ];
        let body = render_body(&views, Shape::Text, 10);
        let Body::Text(t) = body else {
            panic!("expected text");
        };
        assert!(t.value.contains("1"));
    }

    #[test]
    fn render_linked_text_block_carries_issue_url() {
        let views = vec![view_for_test("issueMention", true, Some("ENG-7"))];
        let body = render_body(&views, Shape::LinkedTextBlock, 10);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked");
        };
        assert!(b.items[0].url.as_deref().unwrap().contains("ENG-7"));
    }

    #[test]
    fn render_bars_groups_by_pretty_type() {
        let views = vec![
            view_for_test("issueMention", true, Some("E-1")),
            view_for_test("issueMention", true, Some("E-2")),
            view_for_test("issueAssignedToYou", true, Some("E-3")),
        ];
        let body = render_body(&views, Shape::Bars, 10);
        let Body::Bars(b) = body else {
            panic!("expected bars");
        };
        assert_eq!(b.bars[0].label, "Mention");
        assert_eq!(b.bars[0].value, 2);
    }

    #[test]
    fn badge_critical_when_many_unread() {
        let views: Vec<NotifView> = (0..15)
            .map(|i| view_for_test("issueMention", true, Some(&format!("E-{i}"))))
            .collect();
        let body = render_body(&views, Shape::Badge, 10);
        let Body::Badge(b) = body else {
            panic!("expected badge");
        };
        assert_eq!(b.status, Status::Error);
    }

    fn view_for_test(notif_type: &str, unread: bool, identifier: Option<&str>) -> NotifView {
        NotifView {
            notif_type: notif_type.into(),
            type_label: pretty_type(notif_type),
            actor: Some("sarah".into()),
            issue_identifier: identifier.map(String::from),
            issue_title: identifier.map(|s| format!("Issue {s}")),
            issue_url: identifier.map(|s| format!("https://linear.app/acme/issue/{s}")),
            team_key: identifier.map(|_| "ENG".into()),
            snippet: None,
            created_ts: 1_745_896_800,
            is_unread: unread,
        }
    }

    #[test]
    fn short_snippet_takes_first_non_empty_line_and_caps_length() {
        let s = short_snippet("\n\n  Hello world\nsecond line".to_string()).unwrap();
        assert_eq!(s, "Hello world");
        let long = "x".repeat(120);
        let trimmed = short_snippet(long).unwrap();
        assert!(trimmed.chars().count() <= 80);
        assert!(trimmed.ends_with('…'));
    }

    #[test]
    fn repo_from_pr_url_extracts_owner_slash_repo() {
        assert_eq!(
            repo_from_pr_url("https://github.com/acme/widgets/pull/123"),
            Some("acme/widgets".into())
        );
    }

    #[test]
    fn repo_from_pr_url_returns_none_for_non_github_host() {
        assert_eq!(
            repo_from_pr_url("https://gitlab.com/acme/widgets/-/merge_requests/9"),
            None
        );
    }

    #[test]
    fn line_for_falls_back_to_snippet_when_no_issue_title() {
        let mut v = view_for_test("pullRequestReviewRequested", true, None);
        v.snippet = Some("Please review the auth refactor".into());
        let line = line_for(&v);
        assert!(line.contains("Please review"), "got: {line}");
    }
}
