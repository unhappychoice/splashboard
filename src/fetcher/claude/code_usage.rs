//! `claude_code_usage` — token + cost rollup from local Claude Code session JSONL files.
//!
//! Walks `<root>/projects/<encoded-cwd>/<session-uuid>.jsonl` under every root in
//! [`crate::fetcher::claude::common::discover_jsonl_dirs`], filters lines to assistant events
//! since the configured window, deduplicates by `(message.id, request_id)` keeping the row with
//! the higher token total, then groups by project / model / day depending on `group_by`.
//!
//! `Safety::Safe` — every read is rooted at a `$HOME`-relative directory the user owns. No
//! network, no token, no exec. Pricing is hardcoded in [`super::common`] so a stale price feed
//! can't disrupt the fetch.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Deserialize;

use crate::fetcher::claude::common::{
    cost_usd, discover_jsonl_dirs, format_cost, format_tokens, project_name_from_cwd,
};
use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, NumberSeriesData,
    Payload, Status, TextBlockData, TextData,
};
use crate::render::Shape;

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::NumberSeries,
    Shape::Badge,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "since",
        type_hint: "\"today\" | \"7d\" | \"30d\" | \"all\"",
        required: false,
        default: Some("\"today\""),
        description: "Window the rollup covers, anchored on the current UTC day.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=50)",
        required: false,
        default: Some("10"),
        description: "Maximum rows surfaced by multi-row shapes (`Bars`, `Entries`, …).",
    },
    OptionSchema {
        name: "group_by",
        type_hint: "\"project\" | \"model\" | \"day\"",
        required: false,
        default: Some("\"project\""),
        description: "Axis used by `Bars` / `Entries` / `TextBlock` row shapes.",
    },
];

const DEFAULT_LIMIT: u32 = 10;
const MAX_LIMIT: u32 = 50;

pub struct ClaudeCodeUsage;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub since: Option<String>,
    pub limit: Option<u32>,
    pub group_by: Option<String>,
}

#[async_trait]
impl Fetcher for ClaudeCodeUsage {
    fn name(&self) -> &str {
        "claude_code_usage"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Token and cost rollup from local Claude Code session JSONL files. Aggregates assistant turns under the chosen `since` window and groups them by project, model, or day. Pricing is bundled, so the splash works offline."
    }
    fn refresh_interval(&self) -> u64 {
        // The JSONL files only grow when the user is actively running Claude Code; a 5-minute
        // refresh is fast enough to mirror "what I just spent" without re-walking MBs every cd.
        5 * 60
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let extra = ctx
            .options
            .as_ref()
            .and_then(|v| toml::to_string(v).ok())
            .unwrap_or_default();
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        sample_body(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let since = parse_since(opts.since.as_deref())?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as usize;
        let group_by = parse_group_by(opts.group_by.as_deref())?;
        let shape = ctx.shape.unwrap_or(Shape::Text);

        let snapshot = build_snapshot(&since, group_by, limit);
        Ok(payload(render_body(&snapshot, shape)))
    }
}

#[derive(Debug, Clone, Copy)]
enum Since {
    Today,
    Days(i64),
    All,
}

impl Since {
    fn label(&self) -> &'static str {
        match self {
            Since::Today => "Today",
            Since::Days(7) => "7d",
            Since::Days(30) => "30d",
            Since::Days(_) => "window",
            Since::All => "All-time",
        }
    }
    fn includes(&self, ts: DateTime<Utc>) -> bool {
        match self {
            Since::All => true,
            Since::Today => ts.date_naive() == Utc::now().date_naive(),
            Since::Days(n) => Utc::now() - Duration::days(*n) <= ts,
        }
    }
}

fn parse_since(raw: Option<&str>) -> Result<Since, FetchError> {
    match raw.unwrap_or("today").trim().to_lowercase().as_str() {
        "today" => Ok(Since::Today),
        "7d" => Ok(Since::Days(7)),
        "30d" => Ok(Since::Days(30)),
        "all" => Ok(Since::All),
        other => Err(FetchError::Failed(format!(
            "invalid since: {other:?} (expected today | 7d | 30d | all)"
        ))),
    }
}

#[derive(Debug, Clone, Copy)]
enum GroupBy {
    Project,
    Model,
    Day,
}

fn parse_group_by(raw: Option<&str>) -> Result<GroupBy, FetchError> {
    match raw.unwrap_or("project").trim().to_lowercase().as_str() {
        "project" => Ok(GroupBy::Project),
        "model" => Ok(GroupBy::Model),
        "day" => Ok(GroupBy::Day),
        other => Err(FetchError::Failed(format!(
            "invalid group_by: {other:?} (expected project | model | day)"
        ))),
    }
}

#[derive(Debug, Clone, Default)]
struct Snapshot {
    rows: Vec<UsageRow>,
    total_cost: f64,
    total_tokens: u64,
    daily_tokens: Vec<u64>,
    since_label: &'static str,
}

#[derive(Debug, Clone)]
struct UsageRow {
    label: String,
    tokens: u64,
    cost: f64,
}

#[derive(Debug, Clone)]
struct UsageEvent {
    timestamp: DateTime<Utc>,
    model: String,
    project: String,
    message_id: String,
    request_id: String,
    input_tokens: u64,
    output_tokens: u64,
    cache_write_5m: u64,
    cache_write_1h: u64,
    cache_read: u64,
}

impl UsageEvent {
    fn token_total(&self) -> u64 {
        self.input_tokens
            + self.output_tokens
            + self.cache_write_5m
            + self.cache_write_1h
            + self.cache_read
    }
    fn cost(&self) -> f64 {
        cost_usd(
            &self.model,
            self.input_tokens,
            self.output_tokens,
            self.cache_write_5m,
            self.cache_write_1h,
            self.cache_read,
        )
    }
}

fn build_snapshot(since: &Since, group_by: GroupBy, limit: usize) -> Snapshot {
    let events = dedup(collect_events(&discover_jsonl_dirs(), since));
    let total_tokens = events.iter().map(UsageEvent::token_total).sum();
    let total_cost = events.iter().map(UsageEvent::cost).sum();
    let rows = group_rows(&events, group_by, limit);
    let daily_tokens = daily_series(&events, since);
    Snapshot {
        rows,
        total_cost,
        total_tokens,
        daily_tokens,
        since_label: since.label(),
    }
}

fn collect_events(roots: &[PathBuf], since: &Since) -> Vec<UsageEvent> {
    roots.iter().flat_map(|r| walk_jsonl(r, since)).collect()
}

fn walk_jsonl(root: &PathBuf, since: &Since) -> Vec<UsageEvent> {
    let Ok(projects) = fs::read_dir(root) else {
        return Vec::new();
    };
    projects
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .flat_map(|dir| read_sessions(&dir.path(), since))
        .collect()
}

fn read_sessions(dir: &PathBuf, since: &Since) -> Vec<UsageEvent> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .flat_map(|e| read_session_file(&e.path(), since))
        .collect()
}

fn read_session_file(path: &PathBuf, since: &Since) -> Vec<UsageEvent> {
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| line.contains("\"type\":\"assistant\""))
        .filter_map(|line| parse_event(&line))
        .filter(|e| since.includes(e.timestamp))
        .collect()
}

#[derive(Deserialize)]
struct RawEvent {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    timestamp: String,
    #[serde(default)]
    cwd: String,
    #[serde(rename = "requestId", default)]
    request_id: String,
    #[serde(default)]
    message: Option<RawMessage>,
}

#[derive(Deserialize)]
struct RawMessage {
    #[serde(default)]
    id: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    usage: Option<RawUsage>,
}

#[derive(Deserialize)]
struct RawUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation: Option<RawCacheCreation>,
}

#[derive(Deserialize)]
struct RawCacheCreation {
    #[serde(default)]
    ephemeral_5m_input_tokens: u64,
    #[serde(default)]
    ephemeral_1h_input_tokens: u64,
}

fn parse_event(line: &str) -> Option<UsageEvent> {
    let raw: RawEvent = serde_json::from_str(line).ok()?;
    if raw.kind != "assistant" {
        return None;
    }
    let message = raw.message?;
    let usage = message.usage?;
    let timestamp = DateTime::parse_from_rfc3339(&raw.timestamp)
        .ok()?
        .with_timezone(&Utc);
    let (cache_write_5m, cache_write_1h) = match usage.cache_creation {
        Some(c) => (c.ephemeral_5m_input_tokens, c.ephemeral_1h_input_tokens),
        // Older JSONL sessions only carry the rollup count; treat it all as 5m write tier.
        None => (usage.cache_creation_input_tokens, 0),
    };
    Some(UsageEvent {
        timestamp,
        model: message.model,
        project: project_name_from_cwd(&raw.cwd),
        message_id: message.id,
        request_id: raw.request_id,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_write_5m,
        cache_write_1h,
        cache_read: usage.cache_read_input_tokens,
    })
}

fn dedup(events: Vec<UsageEvent>) -> Vec<UsageEvent> {
    let mut by_key: HashMap<(String, String), UsageEvent> = HashMap::new();
    for e in events {
        let key = (e.message_id.clone(), e.request_id.clone());
        // ccusage gotcha: a `(message.id, requestId)` can repeat across files when a session
        // resumes. Keep the row with the higher total — that's the canonical final usage.
        by_key
            .entry(key)
            .and_modify(|prev| {
                if e.token_total() > prev.token_total() {
                    *prev = e.clone();
                }
            })
            .or_insert(e);
    }
    by_key.into_values().collect()
}

fn group_rows(events: &[UsageEvent], group_by: GroupBy, limit: usize) -> Vec<UsageRow> {
    let mut by_key: HashMap<String, (u64, f64)> = HashMap::new();
    for e in events {
        let key = match group_by {
            GroupBy::Project => e.project.clone(),
            GroupBy::Model => short_model_name(&e.model),
            GroupBy::Day => e.timestamp.date_naive().to_string(),
        };
        let entry = by_key.entry(key).or_insert((0, 0.0));
        entry.0 += e.token_total();
        entry.1 += e.cost();
    }
    let mut rows: Vec<UsageRow> = by_key
        .into_iter()
        .map(|(label, (tokens, cost))| UsageRow {
            label,
            tokens,
            cost,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.cost
            .partial_cmp(&a.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.tokens.cmp(&a.tokens))
    });
    rows.truncate(limit);
    rows
}

/// `claude-opus-4-7-20250929` → `opus-4-7`. Drops the family prefix and any trailing date so the
/// `model` group key reads as a model family rather than a release id.
fn short_model_name(model: &str) -> String {
    let lower = model.to_lowercase();
    let stripped = lower.strip_prefix("claude-").unwrap_or(&lower);
    stripped
        .rsplit_once('-')
        .map(|(head, tail)| {
            if tail.chars().all(|c| c.is_ascii_digit()) && tail.len() >= 6 {
                head.to_string()
            } else {
                stripped.to_string()
            }
        })
        .unwrap_or_else(|| stripped.to_string())
}

fn daily_series(events: &[UsageEvent], since: &Since) -> Vec<u64> {
    let span_days = match since {
        Since::Today => 1,
        Since::Days(n) => *n as usize,
        Since::All => 30,
    };
    let mut buckets: HashMap<NaiveDate, u64> = HashMap::new();
    for e in events {
        *buckets.entry(e.timestamp.date_naive()).or_default() += e.token_total();
    }
    let today = Utc::now().date_naive();
    (0..span_days)
        .rev()
        .map(|i| {
            let day = today - Duration::days(i as i64);
            buckets.get(&day).copied().unwrap_or(0)
        })
        .collect()
}

fn render_body(snap: &Snapshot, shape: Shape) -> Body {
    match shape {
        Shape::Text => text_body(snap),
        Shape::TextBlock => text_block_body(snap),
        Shape::MarkdownTextBlock => markdown_body(snap),
        Shape::Entries => entries_body(snap),
        Shape::Bars => bars_body(snap),
        Shape::NumberSeries => number_series_body(snap),
        Shape::Badge => badge_body(snap),
        _ => text_body(snap),
    }
}

fn text_body(snap: &Snapshot) -> Body {
    Body::Text(TextData {
        value: headline(snap),
    })
}

fn headline(snap: &Snapshot) -> String {
    if snap.total_tokens == 0 {
        format!("{}: no sessions", snap.since_label)
    } else {
        format!(
            "{}: {} tokens / {}",
            snap.since_label,
            format_tokens(snap.total_tokens),
            format_cost(snap.total_cost),
        )
    }
}

fn text_block_body(snap: &Snapshot) -> Body {
    let mut lines = vec![headline(snap)];
    lines.extend(snap.rows.iter().map(format_row));
    if snap.rows.is_empty() && snap.total_tokens > 0 {
        // Window covers activity but the grouping yielded no rows — likely an obscure axis on a
        // tiny session. Keep the headline; don't add a misleading "no data" line.
    } else if snap.rows.is_empty() {
        lines.push("no recent Claude Code activity".into());
    }
    Body::TextBlock(TextBlockData { lines })
}

fn format_row(row: &UsageRow) -> String {
    format!(
        "{}  {} / {}",
        row.label,
        format_tokens(row.tokens),
        format_cost(row.cost),
    )
}

fn markdown_body(snap: &Snapshot) -> Body {
    let mut lines = vec![format!("**{}**", headline(snap))];
    lines.extend(snap.rows.iter().map(|r| {
        format!(
            "- **{}** — {} / {}",
            r.label,
            format_tokens(r.tokens),
            format_cost(r.cost),
        )
    }));
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: lines.join("\n"),
    })
}

fn entries_body(snap: &Snapshot) -> Body {
    let items = if snap.rows.is_empty() {
        vec![Entry {
            key: snap.since_label.into(),
            value: Some(headline(snap)),
            status: None,
        }]
    } else {
        snap.rows
            .iter()
            .map(|r| Entry {
                key: r.label.clone(),
                value: Some(format!(
                    "{} / {}",
                    format_tokens(r.tokens),
                    format_cost(r.cost)
                )),
                status: None,
            })
            .collect()
    };
    Body::Entries(EntriesData { items })
}

fn bars_body(snap: &Snapshot) -> Body {
    // Bars use raw token counts so chart_bar still ranks correctly; the value_label carries the
    // formatted cost so list_ranking presents money instead of seven-digit token counts.
    let bars = snap
        .rows
        .iter()
        .map(|r| Bar {
            label: r.label.clone(),
            value: r.tokens,
            value_label: Some(format_cost(r.cost)),
        })
        .collect();
    Body::Bars(BarsData { bars })
}

fn number_series_body(snap: &Snapshot) -> Body {
    Body::NumberSeries(NumberSeriesData {
        values: snap.daily_tokens.clone(),
    })
}

fn badge_body(snap: &Snapshot) -> Body {
    let (status, label) = if snap.total_tokens == 0 {
        (Status::Warn, format!("{}: quiet", snap.since_label))
    } else {
        let tone = if snap.total_cost >= 50.0 {
            Status::Error
        } else if snap.total_cost >= 10.0 {
            Status::Warn
        } else {
            Status::Ok
        };
        (
            tone,
            format!("{} {}", snap.since_label, format_cost(snap.total_cost)),
        )
    };
    Body::Badge(BadgeData { status, label })
}

fn sample_body(shape: Shape) -> Option<Body> {
    use crate::samples;
    Some(match shape {
        Shape::Text => samples::text("Today: 3.4M tokens / $12.34"),
        Shape::TextBlock => samples::text_block(&[
            "Today: 3.4M tokens / $12.34",
            "splashboard  2.1M / $7.20",
            "playground   1.3M / $5.14",
        ]),
        Shape::MarkdownTextBlock => samples::markdown(
            "**Today: 3.4M tokens / $12.34**\n- **splashboard** — 2.1M / $7.20\n- **playground** — 1.3M / $5.14",
        ),
        Shape::Entries => samples::entries(&[
            ("splashboard", "2.1M / $7.20"),
            ("playground", "1.3M / $5.14"),
        ]),
        Shape::Bars => Body::Bars(BarsData {
            bars: vec![
                Bar {
                    label: "splashboard".into(),
                    value: 2_100_000,
                    value_label: Some("$7.20".into()),
                },
                Bar {
                    label: "playground".into(),
                    value: 1_300_000,
                    value_label: Some("$5.14".into()),
                },
            ],
        }),
        Shape::NumberSeries => {
            samples::number_series(&[0, 100_000, 250_000, 800_000, 600_000, 2_400_000, 3_400_000])
        }
        Shape::Badge => samples::badge(Status::Warn, "Today $12.34"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::time::Duration as StdDuration;

    use tempfile::tempdir;

    use super::*;

    fn ctx(options: Option<&str>, shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "claude-code".into(),
            timeout: StdDuration::from_secs(1),
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            ..Default::default()
        }
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("bogus = 1").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn parse_since_accepts_known_windows() {
        assert!(matches!(parse_since(None).unwrap(), Since::Today));
        assert!(matches!(parse_since(Some("7d")).unwrap(), Since::Days(7)));
        assert!(matches!(parse_since(Some("30d")).unwrap(), Since::Days(30)));
        assert!(matches!(parse_since(Some("all")).unwrap(), Since::All));
    }

    #[test]
    fn parse_since_rejects_unknown_window() {
        let err = parse_since(Some("yesterday")).unwrap_err();
        assert!(format!("{err}").contains("yesterday"));
    }

    #[test]
    fn parse_group_by_defaults_to_project() {
        assert!(matches!(parse_group_by(None).unwrap(), GroupBy::Project));
    }

    #[test]
    fn parse_event_extracts_usage_and_cache_split() {
        let line = r#"{"type":"assistant","timestamp":"2026-05-05T16:15:07.713Z","cwd":"/home/owner/.ghq/github.com/x/y","requestId":"req_1","message":{"id":"msg_1","model":"claude-opus-4-7","usage":{"input_tokens":6,"output_tokens":254,"cache_creation_input_tokens":25094,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":25094}}}}"#;
        let event = parse_event(line).expect("usage row must parse");
        assert_eq!(event.project, "y");
        assert_eq!(event.message_id, "msg_1");
        assert_eq!(event.input_tokens, 6);
        assert_eq!(event.output_tokens, 254);
        assert_eq!(event.cache_write_1h, 25094);
        assert_eq!(event.cache_write_5m, 0);
    }

    #[test]
    fn parse_event_skips_non_assistant_lines() {
        let line = r#"{"type":"user","message":{"content":"hi"}}"#;
        assert!(parse_event(line).is_none());
    }

    #[test]
    fn parse_event_falls_back_to_rollup_count_when_split_missing() {
        // Older sessions don't carry the `cache_creation` split — fall back to attributing the
        // legacy rollup to the 5m tier so the cost math still runs.
        let line = r#"{"type":"assistant","timestamp":"2026-05-05T00:00:00Z","cwd":"/x","requestId":"r","message":{"id":"m","model":"claude-sonnet-4-6","usage":{"input_tokens":0,"output_tokens":0,"cache_creation_input_tokens":1000,"cache_read_input_tokens":0}}}"#;
        let event = parse_event(line).unwrap();
        assert_eq!(event.cache_write_5m, 1000);
        assert_eq!(event.cache_write_1h, 0);
    }

    #[test]
    fn dedup_keeps_higher_token_total_on_collision() {
        let base = UsageEvent {
            timestamp: Utc::now(),
            model: "claude-sonnet-4-6".into(),
            project: "x".into(),
            message_id: "m".into(),
            request_id: "r".into(),
            input_tokens: 0,
            output_tokens: 0,
            cache_write_5m: 0,
            cache_write_1h: 0,
            cache_read: 0,
        };
        let small = UsageEvent {
            input_tokens: 10,
            ..base.clone()
        };
        let large = UsageEvent {
            input_tokens: 100,
            ..base.clone()
        };
        let kept = dedup(vec![small, large.clone()]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].input_tokens, 100);
    }

    #[test]
    fn group_rows_sorts_by_cost_desc_and_truncates_to_limit() {
        let make = |project: &str, output: u64| UsageEvent {
            timestamp: Utc::now(),
            model: "claude-opus-4-7".into(),
            project: project.into(),
            message_id: project.into(),
            request_id: project.into(),
            input_tokens: 0,
            output_tokens: output,
            cache_write_5m: 0,
            cache_write_1h: 0,
            cache_read: 0,
        };
        let events = vec![make("a", 1_000), make("b", 5_000), make("c", 100)];
        let rows = group_rows(&events, GroupBy::Project, 2);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "b");
        assert_eq!(rows[1].label, "a");
    }

    #[test]
    fn short_model_name_strips_family_prefix_and_release_date() {
        assert_eq!(short_model_name("claude-opus-4-7-20250929"), "opus-4-7");
        assert_eq!(short_model_name("claude-sonnet-4-6"), "sonnet-4-6");
        assert_eq!(short_model_name("claude-3-5-sonnet"), "3-5-sonnet");
    }

    #[test]
    fn since_today_excludes_yesterday() {
        let yesterday = Utc::now() - Duration::days(1);
        assert!(!Since::Today.includes(yesterday));
        assert!(Since::Today.includes(Utc::now()));
    }

    #[test]
    fn since_days_window_includes_recent_and_excludes_old() {
        let recent = Utc::now() - Duration::days(3);
        let old = Utc::now() - Duration::days(40);
        assert!(Since::Days(7).includes(recent));
        assert!(!Since::Days(7).includes(old));
        assert!(Since::All.includes(old));
    }

    #[test]
    fn daily_series_length_matches_window() {
        let series = daily_series(&[], &Since::Days(7));
        assert_eq!(series.len(), 7);
        assert!(series.iter().all(|&n| n == 0));
    }

    #[test]
    fn headline_reports_quiet_window_when_empty() {
        let snap = Snapshot {
            since_label: "Today",
            ..Default::default()
        };
        assert_eq!(headline(&snap), "Today: no sessions");
    }

    #[test]
    fn badge_warns_on_quiet_window() {
        let snap = Snapshot {
            since_label: "Today",
            ..Default::default()
        };
        let Body::Badge(b) = badge_body(&snap) else {
            panic!("expected badge");
        };
        assert_eq!(b.status, Status::Warn);
        assert!(b.label.ends_with("quiet"));
    }

    #[test]
    fn badge_promotes_to_error_above_50_usd() {
        let snap = Snapshot {
            total_tokens: 1,
            total_cost: 75.0,
            since_label: "Today",
            ..Default::default()
        };
        let Body::Badge(b) = badge_body(&snap) else {
            panic!("expected badge");
        };
        assert_eq!(b.status, Status::Error);
    }

    #[test]
    fn bars_body_carries_cost_value_label() {
        let snap = Snapshot {
            rows: vec![UsageRow {
                label: "x".into(),
                tokens: 1_500_000,
                cost: 5.0,
            }],
            ..Default::default()
        };
        let Body::Bars(b) = bars_body(&snap) else {
            panic!("expected bars");
        };
        assert_eq!(b.bars[0].value, 1_500_000);
        assert_eq!(b.bars[0].value_label.as_deref(), Some("$5.000"));
    }

    #[test]
    fn entries_body_falls_back_to_headline_when_no_rows() {
        let snap = Snapshot {
            since_label: "7d",
            ..Default::default()
        };
        let Body::Entries(e) = entries_body(&snap) else {
            panic!("expected entries");
        };
        assert_eq!(e.items.len(), 1);
        assert_eq!(e.items[0].key, "7d");
    }

    #[test]
    fn fetcher_metadata_lists_seven_shapes_with_text_default() {
        let f = ClaudeCodeUsage;
        assert_eq!(f.shapes(), SHAPES);
        assert_eq!(f.default_shape(), Shape::Text);
        for s in SHAPES {
            assert!(f.sample_body(*s).is_some(), "sample missing for {s:?}");
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn fetch_reads_jsonl_from_env_override_root() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempdir().unwrap();
        let projects = tmp.path().join("projects").join("-home-x-y");
        fs::create_dir_all(&projects).unwrap();
        let mut file = fs::File::create(projects.join("session.jsonl")).unwrap();
        let now = Utc::now().to_rfc3339();
        writeln!(
            file,
            r#"{{"type":"assistant","timestamp":"{now}","cwd":"/home/x/y","requestId":"r","message":{{"id":"m","model":"claude-opus-4-7","usage":{{"input_tokens":1000,"output_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}}}}"#
        )
        .unwrap();
        let prev = std::env::var("CLAUDE_CONFIG_DIR").ok();
        unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", tmp.path()) };

        let body = ClaudeCodeUsage
            .fetch(&ctx(Some("since = \"today\""), Some(Shape::Text)))
            .await
            .unwrap()
            .body;

        match prev {
            Some(v) => unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("CLAUDE_CONFIG_DIR") },
        }

        let Body::Text(t) = body else {
            panic!("expected Text, got {body:?}");
        };
        assert!(t.value.contains("Today"));
        assert!(t.value.contains("3k"), "value was {:?}", t.value);
    }

    #[test]
    fn day_grouping_buckets_by_calendar_date() {
        let day1 = Utc.with_ymd_and_hms(2026, 5, 5, 10, 0, 0).unwrap();
        let day2 = Utc.with_ymd_and_hms(2026, 5, 6, 10, 0, 0).unwrap();
        let mk = |ts: DateTime<Utc>, id: &str, out: u64| UsageEvent {
            timestamp: ts,
            model: "claude-opus-4-7".into(),
            project: "x".into(),
            message_id: id.into(),
            request_id: id.into(),
            input_tokens: 0,
            output_tokens: out,
            cache_write_5m: 0,
            cache_write_1h: 0,
            cache_read: 0,
        };
        let events = vec![
            mk(day1, "a", 1_000),
            mk(day1, "b", 2_000),
            mk(day2, "c", 500),
        ];
        let rows = group_rows(&events, GroupBy::Day, 10);
        assert_eq!(rows.len(), 2);
        // Day 1 contributed 3_000 tokens vs day 2's 500, so it ranks first.
        assert!(rows[0].label.ends_with("05-05"));
        assert_eq!(rows[0].tokens, 3_000);
    }

    use chrono::TimeZone;
}
