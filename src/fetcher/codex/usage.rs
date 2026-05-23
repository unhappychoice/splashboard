//! `codex_usage` — token + cost rollup from local Codex CLI session JSONL files.
//!
//! Walks `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` (or `$CODEX_HOME/sessions/...`),
//! filters lines to `event_msg` of type `token_count` carrying per-turn `last_token_usage`,
//! then groups by project / model / day depending on `group_by`. The `turn_context` event
//! that precedes each turn carries the model id and `cwd`; we thread that state forward as
//! we parse so token_count events get attributed to the right turn.
//!
//! `Safety::Safe` — every session read is rooted at a `$HOME`-relative directory the user
//! owns. The fetcher *does* make a single HTTP GET for the LLM pricing snapshot (see
//! [`crate::fetcher::llm_pricing`]) — the URL is hardcoded to splashboard's own GitHub repo,
//! which keeps it under the host-fixed-URL Safe rule. On HTTP failure we fall back to the
//! embedded floor so cost columns stay populated for the headline models.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Deserialize;

use crate::fetcher::codex::common::{
    discover_session_dirs, format_cost, format_tokens, list_session_files, project_name_from_cwd,
    short_model_name,
};
use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::llm_pricing::{self, PriceMap};
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

pub struct CodexUsage;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub since: Option<String>,
    pub limit: Option<u32>,
    pub group_by: Option<String>,
}

#[async_trait]
impl Fetcher for CodexUsage {
    fn name(&self) -> &str {
        "codex_usage"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Token and cost rollup from local Codex CLI session JSONL files. Aggregates per-turn usage under the chosen `since` window and groups it by project, model, or day. Pricing is bundled, so the splash works offline."
    }
    fn refresh_interval(&self) -> u64 {
        // Mirrors claude_code_usage: JSONL only grows when Codex is actively running, so a
        // 5-minute refresh tracks "what I just spent" without re-walking MBs every cd.
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

        let prices = llm_pricing::fetch_pricing(http()).await;
        let snapshot = build_snapshot(&since, group_by, limit, &prices);
        Ok(payload(render_body(&snapshot, shape)))
    }
}

fn http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("splashboard/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client should build with default config")
    })
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
            Since::Days(n) => {
                // Calendar-day comparison so this matches `daily_series`'s bucket window exactly
                // (today .. today-(n-1)). A rolling `now - n days` cutoff would leak partial-day
                // events into the totals that never show up in the per-day series.
                let today = Utc::now().date_naive();
                let start = today - Duration::days((*n - 1).max(0));
                let day = ts.date_naive();
                day >= start && day <= today
            }
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
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
}

impl UsageEvent {
    fn token_total(&self) -> u64 {
        // `cached_input_tokens` is a subset of `input_tokens` in OpenAI's schema; don't double-
        // count it. Reasoning tokens already roll into `output_tokens`.
        self.input_tokens + self.output_tokens
    }
    fn cost(&self, prices: &PriceMap) -> f64 {
        // OpenAI reports `cached_input_tokens` as a subset of `input_tokens`; pass the
        // non-cached chunk at the full input rate and the cached chunk at `cache_read`.
        let billable_input = self.input_tokens.saturating_sub(self.cached_input_tokens);
        llm_pricing::cost_usd(
            prices,
            &self.model,
            billable_input,
            self.output_tokens,
            self.cached_input_tokens,
            0,
            0,
        )
    }
}

fn build_snapshot(since: &Since, group_by: GroupBy, limit: usize, prices: &PriceMap) -> Snapshot {
    let events = collect_events(&list_session_files(&discover_session_dirs()), since);
    let total_tokens = events.iter().map(UsageEvent::token_total).sum();
    let total_cost: f64 = events.iter().map(|e| e.cost(prices)).sum();
    let rows = group_rows(&events, group_by, limit, prices);
    let daily_tokens = daily_series(&events, since);
    Snapshot {
        rows,
        total_cost,
        total_tokens,
        daily_tokens,
        since_label: since.label(),
    }
}

fn collect_events(files: &[PathBuf], since: &Since) -> Vec<UsageEvent> {
    files
        .iter()
        .flat_map(|f| read_session_file(f, since))
        .collect()
}

fn read_session_file(path: &PathBuf, since: &Since) -> Vec<UsageEvent> {
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    let mut model = String::new();
    let mut cwd = String::new();
    let mut out: Vec<UsageEvent> = Vec::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        // `session_meta` and `turn_context` set the current model/cwd; `token_count` events
        // consume that context. Order is sequential, so a running mutable state is enough.
        if let Some(raw) = parse_line(&line) {
            apply_event(raw, &mut model, &mut cwd, since, &mut out);
        }
    }
    out
}

fn apply_event(
    raw: RawLine,
    model: &mut String,
    cwd: &mut String,
    since: &Since,
    out: &mut Vec<UsageEvent>,
) {
    let RawLine {
        timestamp,
        kind,
        payload,
    } = raw;
    match kind.as_str() {
        "session_meta" => {
            if let Some(c) = payload.cwd {
                *cwd = c;
            }
        }
        "turn_context" => {
            if let Some(m) = payload.model {
                *model = m;
            }
            if let Some(c) = payload.cwd {
                *cwd = c;
            }
        }
        "event_msg" => {
            if let Some(usage) = token_usage_from_event(&payload)
                && let Some(ts) = parse_ts(&timestamp)
                && since.includes(ts)
            {
                out.push(UsageEvent {
                    timestamp: ts,
                    model: model.clone(),
                    project: project_name_from_cwd(cwd),
                    input_tokens: usage.input_tokens,
                    cached_input_tokens: usage.cached_input_tokens,
                    output_tokens: usage.output_tokens,
                });
            }
        }
        _ => {}
    }
}

fn parse_ts(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[derive(Deserialize)]
struct RawLine {
    #[serde(default)]
    timestamp: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    payload: RawPayload,
}

#[derive(Default, Deserialize)]
struct RawPayload {
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    model: Option<String>,
    // `event_msg` payloads carry their own `type` discriminator (`token_count`, `task_started`,
    // …); we flatten just the fields we read so unknown subtypes don't trip the deserializer.
    #[serde(default, rename = "type")]
    msg_type: Option<String>,
    #[serde(default)]
    info: Option<RawTokenInfo>,
}

#[derive(Deserialize)]
struct RawTokenInfo {
    #[serde(default)]
    last_token_usage: Option<RawTokenUsage>,
}

#[derive(Deserialize)]
struct RawTokenUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cached_input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

fn parse_line(line: &str) -> Option<RawLine> {
    serde_json::from_str(line).ok()
}

fn token_usage_from_event(payload: &RawPayload) -> Option<RawTokenUsage> {
    if payload.msg_type.as_deref()? != "token_count" {
        return None;
    }
    let info = payload.info.as_ref()?;
    info.last_token_usage.as_ref().map(|u| RawTokenUsage {
        input_tokens: u.input_tokens,
        cached_input_tokens: u.cached_input_tokens,
        output_tokens: u.output_tokens,
    })
}

fn group_rows(
    events: &[UsageEvent],
    group_by: GroupBy,
    limit: usize,
    prices: &PriceMap,
) -> Vec<UsageRow> {
    let mut by_key: HashMap<String, (u64, f64)> = HashMap::new();
    for e in events {
        let key = match group_by {
            GroupBy::Project => e.project.clone(),
            GroupBy::Model => short_model_name(&e.model),
            GroupBy::Day => e.timestamp.date_naive().to_string(),
        };
        let entry = by_key.entry(key).or_insert((0, 0.0));
        entry.0 += e.token_total();
        entry.1 += e.cost(prices);
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
    if snap.rows.is_empty() && snap.total_tokens == 0 {
        lines.push("no recent Codex CLI activity".into());
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
        Shape::Text => samples::text("Today: 1.8M tokens / $7.42"),
        Shape::TextBlock => samples::text_block(&[
            "Today: 1.8M tokens / $7.42",
            "splashboard  1.2M / $4.80",
            "image_gen    600k / $2.62",
        ]),
        Shape::MarkdownTextBlock => samples::markdown(
            "**Today: 1.8M tokens / $7.42**\n- **splashboard** — 1.2M / $4.80\n- **image_gen** — 600k / $2.62",
        ),
        Shape::Entries => samples::entries(&[
            ("splashboard", "1.2M / $4.80"),
            ("image_gen", "600k / $2.62"),
        ]),
        Shape::Bars => Body::Bars(BarsData {
            bars: vec![
                Bar {
                    label: "splashboard".into(),
                    value: 1_200_000,
                    value_label: Some("$4.80".into()),
                },
                Bar {
                    label: "image_gen".into(),
                    value: 600_000,
                    value_label: Some("$2.62".into()),
                },
            ],
        }),
        Shape::NumberSeries => {
            samples::number_series(&[0, 80_000, 200_000, 500_000, 700_000, 1_400_000, 1_800_000])
        }
        Shape::Badge => samples::badge(Status::Warn, "Today $7.42"),
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
            widget_id: "codex".into(),
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
    fn token_usage_from_event_skips_non_token_count_messages() {
        let raw = serde_json::from_str::<RawLine>(
            r#"{"timestamp":"2026-05-22T18:38:08.073Z","type":"event_msg","payload":{"type":"task_started"}}"#,
        )
        .unwrap();
        assert!(token_usage_from_event(&raw.payload).is_none());
    }

    #[test]
    fn token_usage_from_event_extracts_last_turn_usage() {
        let raw = serde_json::from_str::<RawLine>(
            r#"{"timestamp":"2026-05-22T18:38:13.916Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":14878,"cached_input_tokens":6528,"output_tokens":194}}}}"#,
        )
        .unwrap();
        let usage = token_usage_from_event(&raw.payload).unwrap();
        assert_eq!(usage.input_tokens, 14878);
        assert_eq!(usage.cached_input_tokens, 6528);
        assert_eq!(usage.output_tokens, 194);
    }

    #[test]
    fn token_usage_from_event_skips_startup_event_with_null_info() {
        // Codex emits an initial `token_count` with `"info":null` before any turn runs; that
        // must not produce a UsageEvent or every session would gain a phantom zero-cost turn.
        let raw = serde_json::from_str::<RawLine>(
            r#"{"timestamp":"2026-05-22T18:38:08.455Z","type":"event_msg","payload":{"type":"token_count","info":null}}"#,
        )
        .unwrap();
        assert!(token_usage_from_event(&raw.payload).is_none());
    }

    #[test]
    fn read_session_file_threads_turn_context_model_and_cwd_into_each_event() {
        // turn_context sets the model + cwd; the following token_count must inherit them.
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("rollout-2026-05-23T00-00-00-x.jsonl");
        let now = Utc::now().to_rfc3339();
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"timestamp":"{now}","type":"session_meta","payload":{{"cwd":"/h/proj-a"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"timestamp":"{now}","type":"turn_context","payload":{{"model":"gpt-5","cwd":"/h/proj-a"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"timestamp":"{now}","type":"event_msg","payload":{{"type":"token_count","info":{{"last_token_usage":{{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":300}}}}}}}}"#
        )
        .unwrap();
        let events = read_session_file(&path, &Since::All);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].model, "gpt-5");
        assert_eq!(events[0].project, "proj-a");
        assert_eq!(events[0].input_tokens, 1000);
    }

    #[test]
    fn group_rows_sorts_by_cost_desc_and_truncates_to_limit() {
        let make = |project: &str, output: u64| UsageEvent {
            timestamp: Utc::now(),
            model: "gpt-5".into(),
            project: project.into(),
            input_tokens: 0,
            cached_input_tokens: 0,
            output_tokens: output,
        };
        let events = vec![make("a", 1_000), make("b", 5_000), make("c", 100)];
        let rows = group_rows(&events, GroupBy::Project, 2, &llm_pricing::embedded_floor());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "b");
        assert_eq!(rows[1].label, "a");
    }

    #[test]
    fn since_today_excludes_yesterday() {
        let yesterday = Utc::now() - Duration::days(1);
        assert!(!Since::Today.includes(yesterday));
        assert!(Since::Today.includes(Utc::now()));
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
    fn fetcher_metadata_lists_seven_shapes_with_text_default() {
        let f = CodexUsage;
        assert_eq!(f.shapes(), SHAPES);
        assert_eq!(f.default_shape(), Shape::Text);
        for s in SHAPES {
            assert!(f.sample_body(*s).is_some(), "sample missing for {s:?}");
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn fetch_reads_jsonl_from_codex_home_env_override() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempdir().unwrap();
        let day = tmp
            .path()
            .join("sessions")
            .join("2026")
            .join("05")
            .join("23");
        fs::create_dir_all(&day).unwrap();
        let path = day.join("rollout-2026-05-23T00-00-00-x.jsonl");
        let now = Utc::now().to_rfc3339();
        let mut file = fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"{now}","type":"session_meta","payload":{{"cwd":"/h/y"}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"{now}","type":"turn_context","payload":{{"model":"gpt-5","cwd":"/h/y"}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"{now}","type":"event_msg","payload":{{"type":"token_count","info":{{"last_token_usage":{{"input_tokens":1000,"cached_input_tokens":0,"output_tokens":2000}}}}}}}}"#
        )
        .unwrap();

        let prev = std::env::var("CODEX_HOME").ok();
        unsafe { std::env::set_var("CODEX_HOME", tmp.path()) };
        let body = CodexUsage
            .fetch(&ctx(Some("since = \"today\""), Some(Shape::Text)))
            .await
            .unwrap()
            .body;
        match prev {
            Some(v) => unsafe { std::env::set_var("CODEX_HOME", v) },
            None => unsafe { std::env::remove_var("CODEX_HOME") },
        }

        let Body::Text(t) = body else {
            panic!("expected Text, got {body:?}");
        };
        assert!(t.value.contains("Today"));
        assert!(t.value.contains("3k"), "value was {:?}", t.value);
    }

    fn snap_with_rows() -> Snapshot {
        Snapshot {
            rows: vec![
                UsageRow {
                    label: "splashboard".into(),
                    tokens: 2_100_000,
                    cost: 7.2,
                },
                UsageRow {
                    label: "playground".into(),
                    tokens: 1_300_000,
                    cost: 5.14,
                },
            ],
            total_cost: 12.34,
            total_tokens: 3_400_000,
            daily_tokens: vec![0, 100_000, 250_000, 3_400_000],
            since_label: "Today",
        }
    }

    #[test]
    fn text_body_renders_headline_with_total_tokens_and_cost() {
        let Body::Text(t) = text_body(&snap_with_rows()) else {
            panic!("expected text");
        };
        assert!(t.value.starts_with("Today"));
        assert!(t.value.contains("3.4M"));
        assert!(t.value.contains("$12.34"));
    }

    #[test]
    fn text_block_body_lists_headline_then_one_line_per_row() {
        let Body::TextBlock(b) = text_block_body(&snap_with_rows()) else {
            panic!("expected text_block");
        };
        assert_eq!(b.lines.len(), 3);
        assert!(b.lines[0].contains("$12.34"));
        assert!(b.lines[1].starts_with("splashboard"));
        assert!(b.lines[2].starts_with("playground"));
    }

    #[test]
    fn text_block_body_falls_back_to_quiet_line_when_no_activity() {
        // No rows AND no tokens at all → the fetcher must surface a placeholder line, otherwise
        // a quiet user just sees a bare "Today: no sessions" headline with nothing under it.
        let snap = Snapshot {
            since_label: "Today",
            ..Default::default()
        };
        let Body::TextBlock(b) = text_block_body(&snap) else {
            panic!("expected text_block");
        };
        assert!(b.lines.iter().any(|l| l.contains("no recent")));
    }

    #[test]
    fn markdown_body_emphasises_headline_and_each_row() {
        let Body::MarkdownTextBlock(m) = markdown_body(&snap_with_rows()) else {
            panic!("expected markdown");
        };
        assert!(m.value.starts_with("**Today"));
        assert!(m.value.contains("- **splashboard**"));
    }

    #[test]
    fn entries_body_falls_back_to_headline_when_no_rows() {
        // Mirrors claude_code_usage: entries shape must always emit at least one row so
        // grid_table doesn't render a blank box.
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
    fn entries_body_carries_one_item_per_row_with_token_and_cost_value() {
        let Body::Entries(e) = entries_body(&snap_with_rows()) else {
            panic!("expected entries");
        };
        assert_eq!(e.items.len(), 2);
        assert_eq!(e.items[0].key, "splashboard");
        let value = e.items[0].value.as_deref().unwrap_or_default();
        assert!(value.contains("2.1M"));
        assert!(value.contains("$"));
    }

    #[test]
    fn bars_body_uses_token_count_as_value_and_cost_string_as_label() {
        // chart_bar ranks by the integer value field; list_ranking surfaces the value_label
        // verbatim. We want bars to rank by token count and *display* cost.
        let Body::Bars(b) = bars_body(&snap_with_rows()) else {
            panic!("expected bars");
        };
        assert_eq!(b.bars.len(), 2);
        assert_eq!(b.bars[0].value, 2_100_000);
        assert!(b.bars[0].value_label.as_deref().unwrap_or("").contains("$"));
    }

    #[test]
    fn number_series_body_carries_daily_tokens_in_order() {
        let Body::NumberSeries(n) = number_series_body(&snap_with_rows()) else {
            panic!("expected number_series");
        };
        assert_eq!(n.values, vec![0, 100_000, 250_000, 3_400_000]);
    }
}
