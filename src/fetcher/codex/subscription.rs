//! `codex_subscription` — Codex CLI plan utilisation as Codex sees it.
//!
//! Codex CLI piggybacks rate-limit state on every `token_count` event in the session JSONL:
//!
//! ```json
//! "rate_limits": {
//!   "primary":   { "used_percent": 0.0, "window_minutes": 300,   "resets_at": 1779493088 },
//!   "secondary": { "used_percent": 0.0, "window_minutes": 10080, "resets_at": 1779863530 },
//!   "plan_type": "pro",
//!   ...
//! }
//! ```
//!
//! `primary` is the 5h window, `secondary` is the 7d window. This fetcher walks the newest
//! session and reads the last such event — that's the same data Codex itself would show.
//!
//! We deliberately don't make a network call here even though OpenAI's API can return this
//! same payload: the only way to re-fetch is to make a billable inference request, which is
//! not what a splash widget should do. Freshness is bounded by "when did the user last run
//! Codex" — `format_reset` will show `now` for a window that has already rolled over.
//!
//! `Safety::Safe` — every read stays inside `~/.codex/sessions/`.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

use crate::fetcher::codex::common::{discover_session_dirs, list_session_files};
use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, RatioData,
    Status, TextBlockData, TextData, TimelineData, TimelineEvent,
};
use crate::render::Shape;

const SHAPES: &[Shape] = &[
    Shape::Ratio,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::Badge,
    Shape::Timeline,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "window",
    type_hint: "\"5h\" | \"7d\" | \"primary\" | \"secondary\"",
    required: false,
    default: Some("\"5h\""),
    description: "Which window the single-value shapes (`Ratio`, `Badge`) report. Multi-row shapes always list both windows.",
}];

pub struct CodexSubscription;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub window: Option<String>,
}

#[async_trait]
impl Fetcher for CodexSubscription {
    fn name(&self) -> &str {
        "codex_subscription"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Codex CLI subscription utilisation, parsed from the local session JSONL. Each token_count event Codex writes carries the 5h (primary) and 7d (secondary) rate-limit windows; we read the most recent one. No HTTP — re-fetching this would mean making a billable inference call."
    }
    fn refresh_interval(&self) -> u64 {
        // The newest JSONL grows as the user uses Codex; a 5-minute cache matches the
        // session-recent feel without re-reading megabytes on every cd.
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
        let target = parse_window(opts.window.as_deref());
        let shape = ctx.shape.unwrap_or(Shape::Ratio);
        let snapshot = load_snapshot()?;
        Ok(payload(render_body(&snapshot, target, shape)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowKind {
    Primary,
    Secondary,
}

fn parse_window(raw: Option<&str>) -> WindowKind {
    match raw.unwrap_or("5h").trim().to_lowercase().as_str() {
        "7d" | "secondary" | "week" | "weekly" => WindowKind::Secondary,
        // 5h is the canonical primary; any unknown value falls through to primary so a typo
        // doesn't render an empty widget.
        _ => WindowKind::Primary,
    }
}

#[derive(Debug, Clone, Default)]
struct Snapshot {
    windows: Vec<NamedWindow>,
    plan_type: Option<String>,
}

#[derive(Debug, Clone)]
struct NamedWindow {
    kind: WindowKind,
    label: String,
    utilization: f64,
    resets_at: Option<DateTime<Utc>>,
}

impl Snapshot {
    fn find(&self, kind: WindowKind) -> Option<&NamedWindow> {
        self.windows.iter().find(|w| w.kind == kind)
    }
}

fn load_snapshot() -> Result<Snapshot, FetchError> {
    let files = list_session_files(&discover_session_dirs());
    let latest = files
        .last()
        .cloned()
        .ok_or_else(|| FetchError::Failed("no Codex session JSONL found".into()))?;
    parse_latest_rate_limits(&latest)
        .ok_or_else(|| FetchError::Failed(format!("no rate_limits event in {}", latest.display())))
}

fn parse_latest_rate_limits(path: &PathBuf) -> Option<Snapshot> {
    let file = fs::File::open(path).ok()?;
    // Walk forward, keeping the most recent rate_limits seen. Reverse-scan would be faster but
    // BufReader doesn't seek backward — sessions top out at a few MB so this is fine.
    let mut latest: Option<RawRateLimits> = None;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if let Some(rl) = extract_rate_limits(&line) {
            latest = Some(rl);
        }
    }
    latest.map(snapshot_from)
}

fn extract_rate_limits(line: &str) -> Option<RawRateLimits> {
    let raw: RawLine = serde_json::from_str(line).ok()?;
    if raw.kind != "event_msg" {
        return None;
    }
    let payload = raw.payload?;
    if payload.msg_type.as_deref()? != "token_count" {
        return None;
    }
    payload.rate_limits
}

#[derive(Deserialize)]
struct RawLine {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    payload: Option<RawPayload>,
}

#[derive(Deserialize)]
struct RawPayload {
    #[serde(default, rename = "type")]
    msg_type: Option<String>,
    #[serde(default)]
    rate_limits: Option<RawRateLimits>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawRateLimits {
    #[serde(default)]
    primary: Option<RawRateWindow>,
    #[serde(default)]
    secondary: Option<RawRateWindow>,
    #[serde(default)]
    plan_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawRateWindow {
    #[serde(default)]
    used_percent: f64,
    #[serde(default)]
    window_minutes: u64,
    #[serde(default)]
    resets_at: Option<i64>,
}

fn snapshot_from(raw: RawRateLimits) -> Snapshot {
    let windows = [
        raw.primary.map(|w| named_window(WindowKind::Primary, w)),
        raw.secondary
            .map(|w| named_window(WindowKind::Secondary, w)),
    ]
    .into_iter()
    .flatten()
    .collect();
    Snapshot {
        windows,
        plan_type: raw.plan_type,
    }
}

fn named_window(kind: WindowKind, w: RawRateWindow) -> NamedWindow {
    NamedWindow {
        kind,
        label: window_label(w.window_minutes, kind),
        // `used_percent` is 0..100 (allow up to 150 to keep an over-quota window visible).
        utilization: (w.used_percent / 100.0).clamp(0.0, 1.5),
        resets_at: w.resets_at.and_then(|s| Utc.timestamp_opt(s, 0).single()),
    }
}

/// Resolve a window's display label from its `window_minutes` (300 → "5h", 10080 → "7d"),
/// falling back to a kind-relative label when the duration is unfamiliar.
fn window_label(minutes: u64, kind: WindowKind) -> String {
    // Match `0` first — every divisibility check below would otherwise succeed for it and emit
    // a misleading "0d" / "0h" label.
    if minutes == 0 {
        return match kind {
            WindowKind::Primary => "primary".into(),
            WindowKind::Secondary => "secondary".into(),
        };
    }
    match minutes {
        300 => "5h".into(),
        10_080 => "7d".into(),
        m if m % 1440 == 0 => format!("{}d", m / 1440),
        m if m % 60 == 0 => format!("{}h", m / 60),
        m => format!("{m}m"),
    }
}

fn render_body(snap: &Snapshot, target: WindowKind, shape: Shape) -> Body {
    match shape {
        Shape::Ratio => ratio_body(snap, target),
        Shape::Text => text_body(snap),
        Shape::TextBlock => text_block_body(snap),
        Shape::MarkdownTextBlock => markdown_body(snap),
        Shape::Entries => entries_body(snap),
        Shape::Bars => bars_body(snap),
        Shape::Badge => badge_body(snap, target),
        Shape::Timeline => timeline_body(snap),
        _ => text_body(snap),
    }
}

fn ratio_body(snap: &Snapshot, target: WindowKind) -> Body {
    let Some(window) = snap.find(target) else {
        return Body::Ratio(RatioData {
            value: 0.0,
            label: Some(format!("{} · n/a", default_label(target))),
            denominator: Some(100),
        });
    };
    let reset = window
        .resets_at
        .map(format_reset)
        .unwrap_or_else(|| "n/a".into());
    Body::Ratio(RatioData {
        value: window.utilization.clamp(0.0, 1.0),
        label: Some(format!("{} · resets {reset}", window.label)),
        denominator: Some(100),
    })
}

fn default_label(kind: WindowKind) -> &'static str {
    match kind {
        WindowKind::Primary => "5h",
        WindowKind::Secondary => "7d",
    }
}

fn text_body(snap: &Snapshot) -> Body {
    let parts: Vec<String> = snap
        .windows
        .iter()
        .map(|w| format!("{} {}", w.label, percent(w.utilization)))
        .collect();
    let head = match &snap.plan_type {
        Some(plan) if !plan.is_empty() => format!("[{plan}] "),
        _ => String::new(),
    };
    Body::Text(TextData {
        value: if parts.is_empty() {
            "codex subscription: n/a".into()
        } else {
            format!("{head}{}", parts.join(" · "))
        },
    })
}

fn text_block_body(snap: &Snapshot) -> Body {
    let mut lines: Vec<String> = Vec::new();
    if let Some(plan) = &snap.plan_type
        && !plan.is_empty()
    {
        lines.push(format!("plan: {plan}"));
    }
    lines.extend(snap.windows.iter().map(format_window_line));
    Body::TextBlock(TextBlockData {
        lines: if lines.is_empty() {
            vec!["codex subscription: n/a".into()]
        } else {
            lines
        },
    })
}

fn markdown_body(snap: &Snapshot) -> Body {
    let mut lines: Vec<String> = Vec::new();
    if let Some(plan) = &snap.plan_type
        && !plan.is_empty()
    {
        lines.push(format!("**plan**: `{plan}`"));
    }
    lines.extend(snap.windows.iter().map(|w| {
        let reset = w
            .resets_at
            .map(format_reset)
            .unwrap_or_else(|| "n/a".into());
        format!(
            "- **{}** — {} (resets {reset})",
            w.label,
            percent(w.utilization),
        )
    }));
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: if lines.is_empty() {
            "_codex subscription unavailable_".into()
        } else {
            lines.join("\n")
        },
    })
}

fn entries_body(snap: &Snapshot) -> Body {
    let mut items: Vec<Entry> = Vec::new();
    if let Some(plan) = &snap.plan_type
        && !plan.is_empty()
    {
        items.push(Entry {
            key: "plan".into(),
            value: Some(plan.clone()),
            status: None,
        });
    }
    items.extend(snap.windows.iter().map(|w| Entry {
        key: w.label.clone(),
        value: Some(format!(
            "{} (resets {})",
            percent(w.utilization),
            w.resets_at.map(format_reset).unwrap_or_else(|| "n/a".into()),
        )),
        status: Some(status_for(w.utilization)),
    }));
    Body::Entries(EntriesData {
        items: if items.is_empty() {
            vec![Entry {
                key: "codex".into(),
                value: Some("subscription unavailable".into()),
                status: Some(Status::Warn),
            }]
        } else {
            items
        },
    })
}

fn bars_body(snap: &Snapshot) -> Body {
    let bars: Vec<Bar> = snap
        .windows
        .iter()
        .map(|w| Bar {
            label: w.label.clone(),
            // Bars take u64 — store utilisation as basis points (0..=15000) so a fully spent
            // window outranks a quarter-spent one in chart_bar.
            value: ((w.utilization * 10_000.0).round() as i64).max(0) as u64,
            value_label: Some(percent(w.utilization)),
        })
        .collect();
    Body::Bars(BarsData { bars })
}

fn badge_body(snap: &Snapshot, target: WindowKind) -> Body {
    let Some(window) = snap.find(target) else {
        return Body::Badge(BadgeData {
            status: Status::Warn,
            label: format!("{}: n/a", default_label(target)),
        });
    };
    Body::Badge(BadgeData {
        status: status_for(window.utilization),
        label: format!("{} {}", window.label, percent(window.utilization)),
    })
}

fn timeline_body(snap: &Snapshot) -> Body {
    let mut events: Vec<TimelineEvent> = snap
        .windows
        .iter()
        .filter_map(|w| {
            w.resets_at.map(|ts| TimelineEvent {
                timestamp: ts.timestamp(),
                title: format!("{} resets", w.label),
                detail: Some(format!("at {}", percent(w.utilization))),
                status: Some(status_for(w.utilization)),
            })
        })
        .collect();
    events.sort_by_key(|e| e.timestamp);
    Body::Timeline(TimelineData { events })
}

fn format_window_line(window: &NamedWindow) -> String {
    let reset = window
        .resets_at
        .map(format_reset)
        .unwrap_or_else(|| "n/a".into());
    format!(
        "{}  {} · resets {reset}",
        window.label,
        percent(window.utilization),
    )
}

fn percent(util: f64) -> String {
    format!("{:.0}%", (util * 100.0).clamp(0.0, 150.0))
}

fn format_reset(ts: DateTime<Utc>) -> String {
    let delta = ts.signed_duration_since(Utc::now());
    if delta <= chrono::Duration::zero() {
        return "now".into();
    }
    let mins = delta.num_minutes();
    if mins < 60 {
        format!("{mins}m")
    } else if mins < 60 * 48 {
        format!("{}h", delta.num_hours())
    } else {
        format!("{}d", delta.num_days())
    }
}

fn status_for(util: f64) -> Status {
    if util >= 0.85 {
        Status::Error
    } else if util >= 0.60 {
        Status::Warn
    } else {
        Status::Ok
    }
}

fn sample_body(shape: Shape) -> Option<Body> {
    use crate::samples;
    Some(match shape {
        Shape::Ratio => samples::ratio(0.18, "5h · resets 3h"),
        Shape::Text => samples::text("[pro] 5h 18% · 7d 7%"),
        Shape::TextBlock => {
            samples::text_block(&["plan: pro", "5h  18% · resets 3h", "7d   7% · resets 5d"])
        }
        Shape::MarkdownTextBlock => samples::markdown(
            "**plan**: `pro`\n- **5h** — 18% (resets 3h)\n- **7d** — 7% (resets 5d)",
        ),
        Shape::Entries => samples::entries(&[
            ("plan", "pro"),
            ("5h", "18% (resets 3h)"),
            ("7d", "7% (resets 5d)"),
        ]),
        Shape::Bars => Body::Bars(BarsData {
            bars: vec![
                Bar {
                    label: "5h".into(),
                    value: 1800,
                    value_label: Some("18%".into()),
                },
                Bar {
                    label: "7d".into(),
                    value: 700,
                    value_label: Some("7%".into()),
                },
            ],
        }),
        Shape::Badge => samples::badge(Status::Ok, "5h 18%"),
        Shape::Timeline => samples::timeline(&[
            (
                Utc::now().timestamp() + 3 * 3_600,
                "5h resets",
                Some("at 18%"),
            ),
            (
                Utc::now().timestamp() + 5 * 86_400,
                "7d resets",
                Some("at 7%"),
            ),
        ]),
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
            widget_id: "codex-sub".into(),
            timeout: StdDuration::from_secs(1),
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            ..Default::default()
        }
    }

    fn snap_full() -> Snapshot {
        Snapshot {
            plan_type: Some("pro".into()),
            windows: vec![
                NamedWindow {
                    kind: WindowKind::Primary,
                    label: "5h".into(),
                    utilization: 0.18,
                    resets_at: Some(Utc::now() + chrono::Duration::hours(3)),
                },
                NamedWindow {
                    kind: WindowKind::Secondary,
                    label: "7d".into(),
                    utilization: 0.07,
                    resets_at: Some(Utc::now() + chrono::Duration::days(5)),
                },
            ],
        }
    }

    #[test]
    fn parse_window_accepts_friendly_aliases() {
        assert_eq!(parse_window(None), WindowKind::Primary);
        assert_eq!(parse_window(Some("5h")), WindowKind::Primary);
        assert_eq!(parse_window(Some("7d")), WindowKind::Secondary);
        assert_eq!(parse_window(Some("secondary")), WindowKind::Secondary);
        // Typo / unknown → primary fallback so the widget still draws.
        assert_eq!(parse_window(Some("five-hour")), WindowKind::Primary);
    }

    #[test]
    fn window_label_derives_from_window_minutes() {
        assert_eq!(window_label(300, WindowKind::Primary), "5h");
        assert_eq!(window_label(10_080, WindowKind::Secondary), "7d");
        assert_eq!(window_label(1440 * 14, WindowKind::Secondary), "14d");
        assert_eq!(window_label(120, WindowKind::Primary), "2h");
        assert_eq!(window_label(45, WindowKind::Primary), "45m");
        assert_eq!(window_label(0, WindowKind::Primary), "primary");
    }

    #[test]
    fn extract_rate_limits_picks_token_count_payload() {
        let line = r#"{"timestamp":"x","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"used_percent":7.0,"window_minutes":300,"resets_at":1779493088},"secondary":null,"plan_type":"pro"}}}"#;
        let rl = extract_rate_limits(line).expect("must parse rate_limits");
        assert!(rl.primary.is_some());
        assert_eq!(rl.plan_type.as_deref(), Some("pro"));
    }

    #[test]
    fn extract_rate_limits_skips_non_token_count_events() {
        // task_started carries no rate_limits — must not produce a false positive.
        let line = r#"{"timestamp":"x","type":"event_msg","payload":{"type":"task_started"}}"#;
        assert!(extract_rate_limits(line).is_none());
        // session_meta has the wrong outer kind.
        let line = r#"{"timestamp":"x","type":"session_meta","payload":{}}"#;
        assert!(extract_rate_limits(line).is_none());
    }

    #[test]
    fn snapshot_from_clamps_over_quota_at_one_and_a_half() {
        let raw = RawRateLimits {
            primary: Some(RawRateWindow {
                used_percent: 200.0,
                window_minutes: 300,
                resets_at: None,
            }),
            secondary: None,
            plan_type: None,
        };
        let snap = snapshot_from(raw);
        assert_eq!(snap.windows[0].utilization, 1.5);
    }

    #[test]
    fn parse_latest_rate_limits_keeps_the_last_seen_event() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("rollout-x.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"event_msg","payload":{{"type":"token_count","rate_limits":{{"primary":{{"used_percent":5.0,"window_minutes":300,"resets_at":1}}}}}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"event_msg","payload":{{"type":"token_count","rate_limits":{{"primary":{{"used_percent":42.0,"window_minutes":300,"resets_at":1}}}}}}}}"#
        )
        .unwrap();
        let snap = parse_latest_rate_limits(&path).expect("must parse");
        // 42% is the last event; not 5% (the first).
        assert!((snap.windows[0].utilization - 0.42).abs() < 1e-9);
    }

    #[test]
    fn ratio_body_picks_requested_window_by_kind() {
        let Body::Ratio(r) = ratio_body(&snap_full(), WindowKind::Secondary) else {
            panic!("expected ratio");
        };
        assert!((r.value - 0.07).abs() < 1e-6);
        assert!(r.label.as_deref().unwrap_or_default().starts_with("7d"));
    }

    #[test]
    fn ratio_body_falls_back_to_zero_when_window_missing() {
        let Body::Ratio(r) = ratio_body(&Snapshot::default(), WindowKind::Primary) else {
            panic!("expected ratio");
        };
        assert_eq!(r.value, 0.0);
        assert!(r.label.as_deref().unwrap_or_default().contains("n/a"));
    }

    #[test]
    fn text_body_prefixes_plan_type_when_present() {
        let Body::Text(t) = text_body(&snap_full()) else {
            panic!("expected text");
        };
        assert!(t.value.contains("[pro]"));
        assert!(t.value.contains("5h"));
        assert!(t.value.contains("7d"));
    }

    #[test]
    fn entries_body_lists_plan_then_windows() {
        let Body::Entries(e) = entries_body(&snap_full()) else {
            panic!("expected entries");
        };
        assert_eq!(e.items.len(), 3);
        assert_eq!(e.items[0].key, "plan");
        assert_eq!(e.items[1].key, "5h");
        assert_eq!(e.items[2].key, "7d");
    }

    #[test]
    fn entries_body_falls_back_when_empty() {
        let Body::Entries(e) = entries_body(&Snapshot::default()) else {
            panic!("expected entries");
        };
        assert_eq!(e.items.len(), 1);
        assert!(
            e.items[0]
                .value
                .as_deref()
                .unwrap_or_default()
                .contains("unavailable")
        );
    }

    #[test]
    fn bars_body_emits_basis_points_per_window() {
        let Body::Bars(b) = bars_body(&snap_full()) else {
            panic!("expected bars");
        };
        assert_eq!(b.bars.len(), 2);
        assert_eq!(b.bars[0].value, 1800);
        assert_eq!(b.bars[1].value, 700);
    }

    #[test]
    fn timeline_body_drops_windows_without_reset_and_sorts_by_timestamp() {
        let mut snap = snap_full();
        snap.windows[1].resets_at = None;
        let Body::Timeline(t) = timeline_body(&snap) else {
            panic!("expected timeline");
        };
        assert_eq!(t.events.len(), 1);
        assert!(t.events[0].title.starts_with("5h"));
    }

    #[test]
    fn text_block_body_includes_plan_line_and_one_per_window() {
        let Body::TextBlock(b) = text_block_body(&snap_full()) else {
            panic!("expected text_block");
        };
        assert_eq!(b.lines.len(), 3);
        assert!(b.lines[0].contains("pro"));
        assert!(b.lines[1].starts_with("5h"));
        assert!(b.lines[2].starts_with("7d"));
    }

    #[test]
    fn text_block_body_falls_back_when_no_data() {
        let Body::TextBlock(b) = text_block_body(&Snapshot::default()) else {
            panic!("expected text_block");
        };
        assert_eq!(b.lines.len(), 1);
        assert!(b.lines[0].contains("n/a"));
    }

    #[test]
    fn markdown_body_emphasises_plan_and_each_window() {
        let Body::MarkdownTextBlock(m) = markdown_body(&snap_full()) else {
            panic!("expected markdown");
        };
        assert!(m.value.contains("**plan**: `pro`"));
        assert!(m.value.contains("- **5h**"));
        assert!(m.value.contains("- **7d**"));
    }

    #[test]
    fn markdown_body_falls_back_when_empty() {
        let Body::MarkdownTextBlock(m) = markdown_body(&Snapshot::default()) else {
            panic!("expected markdown");
        };
        assert!(m.value.contains("unavailable"));
    }

    #[test]
    fn status_for_bucketed_thresholds() {
        assert_eq!(status_for(0.0), Status::Ok);
        assert_eq!(status_for(0.5), Status::Ok);
        assert_eq!(status_for(0.6), Status::Warn);
        assert_eq!(status_for(0.85), Status::Error);
    }

    #[test]
    fn percent_caps_over_quota_at_150() {
        assert_eq!(percent(0.07), "7%");
        assert_eq!(percent(1.0), "100%");
        assert_eq!(percent(1.5), "150%");
        assert_eq!(percent(2.0), "150%");
    }

    #[test]
    fn fetcher_metadata_exposes_eight_shapes_with_ratio_default() {
        let f = CodexSubscription;
        assert_eq!(f.shapes(), SHAPES);
        assert_eq!(f.default_shape(), Shape::Ratio);
        for s in SHAPES {
            assert!(f.sample_body(*s).is_some(), "sample missing for {s:?}");
        }
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("bogus = 1").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn fetch_surfaces_missing_sessions_as_failed() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempdir().unwrap();
        // CODEX_HOME points to a directory that has no `sessions/` tree at all.
        let prev = std::env::var("CODEX_HOME").ok();
        unsafe { std::env::set_var("CODEX_HOME", tmp.path()) };
        let err = CodexSubscription
            .fetch(&ctx(None, Some(Shape::Ratio)))
            .await
            .unwrap_err();
        match prev {
            Some(v) => unsafe { std::env::set_var("CODEX_HOME", v) },
            None => unsafe { std::env::remove_var("CODEX_HOME") },
        }
        assert!(format!("{err}").contains("no Codex session"));
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn fetch_reads_latest_rate_limits_from_codex_home_env() {
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
        let resets_at = (Utc::now() + chrono::Duration::hours(3)).timestamp();
        let mut f = fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"event_msg","payload":{{"type":"token_count","rate_limits":{{"primary":{{"used_percent":18.0,"window_minutes":300,"resets_at":{resets_at}}},"secondary":{{"used_percent":7.0,"window_minutes":10080,"resets_at":{resets_at}}},"plan_type":"pro"}}}}}}"#
        )
        .unwrap();

        let prev = std::env::var("CODEX_HOME").ok();
        unsafe { std::env::set_var("CODEX_HOME", tmp.path()) };
        let body = CodexSubscription
            .fetch(&ctx(None, Some(Shape::Ratio)))
            .await
            .unwrap()
            .body;
        match prev {
            Some(v) => unsafe { std::env::set_var("CODEX_HOME", v) },
            None => unsafe { std::env::remove_var("CODEX_HOME") },
        }

        let Body::Ratio(r) = body else {
            panic!("expected Ratio, got {body:?}");
        };
        assert!((r.value - 0.18).abs() < 1e-6, "ratio was {}", r.value);
        assert!(r.label.as_deref().unwrap_or("").starts_with("5h"));
    }
}
