//! `claude_subscription` — Claude (Max plan) subscription utilisation as Claude Code sees it.
//!
//! Reads the OAuth token at `~/.claude/.credentials.json` and queries the **undocumented**
//! endpoint `GET https://api.anthropic.com/api/oauth/usage` with the `oauth-2025-04-20` beta
//! header (see the catalog body and the [`ohugonnot/claude-code-statusline`] reference).
//! The response carries one entry per usage window — `five_hour`, `seven_day`,
//! `seven_day_sonnet`, `seven_day_opus`, `seven_day_omelette` (the "Claude Design" pool in the
//! claude.ai UI), and a handful of internal codenames — most of which are `null` for any one
//! account. We walk the response generically, keep every non-null window in a canonical order,
//! and label known keys with the UI string the user sees on claude.ai/account.
//!
//! Undocumented = fragile: every failure path short-circuits to [`FetchError`] so the splash
//! still renders a placeholder, never panics.
//!
//! `Safety::Safe` — host is hardcoded, the OAuth token only travels to `api.anthropic.com`.
//!
//! [`ohugonnot/claude-code-statusline`]: https://github.com/ohugonnot/claude-code-statusline/blob/main/statusline.sh

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

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
    type_hint: "string (response key — e.g. \"five_hour\" / \"seven_day\" / \"seven_day_sonnet\" / \"seven_day_omelette\")",
    required: false,
    default: Some("\"five_hour\""),
    description: "Which window the single-value shapes (`Ratio`, `Badge`) report. Multi-row shapes always list every non-null window the response carries.",
}];

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));

pub struct ClaudeSubscription;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub window: Option<String>,
}

#[async_trait]
impl Fetcher for ClaudeSubscription {
    fn name(&self) -> &str {
        "claude_subscription"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Claude (Max plan) subscription utilisation as seen by Claude Code's OAuth credentials. Reads `~/.claude/.credentials.json` and queries the undocumented `oauth/usage` endpoint, exposing every window the response carries (5-hour, 7-day, per-model and pool-specific siblings)."
    }
    fn refresh_interval(&self) -> u64 {
        15 * 60
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
        let target = normalise_window_key(opts.window.as_deref());
        let shape = ctx.shape.unwrap_or(Shape::Ratio);
        let token = load_oauth_token()?;
        let snapshot = fetch_snapshot(&token).await?;
        Ok(payload(render_body(&snapshot, &target, shape)))
    }
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

/// User-facing strings for the response's window keys, in claude.ai/account language. Unknown
/// keys fall through [`humanize_key`].
fn known_label(key: &str) -> Option<&'static str> {
    match key {
        "five_hour" => Some("5h"),
        "seven_day" => Some("7d"),
        "seven_day_opus" => Some("7d Opus"),
        "seven_day_sonnet" => Some("7d Sonnet"),
        "seven_day_oauth_apps" => Some("7d OAuth apps"),
        "seven_day_cowork" => Some("7d Cowork"),
        "seven_day_omelette" => Some("Claude Design"),
        "seven_day_omelette_promotional" => Some("Claude Design (promo)"),
        "omelette_promotional" => Some("Claude Design (promo)"),
        "tangelo" => Some("Tangelo"),
        "iguana_necktie" => Some("Iguana Necktie"),
        _ => None,
    }
}

fn label_for(key: &str) -> String {
    known_label(key)
        .map(str::to_string)
        .unwrap_or_else(|| humanize_key(key))
}

/// `seven_day_some_thing` → `7d some thing`, `tangelo` → `Tangelo`. Capitalises the first letter
/// of the first word but otherwise keeps lowercase so multi-word codenames stay scannable.
fn humanize_key(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    let normalised = key.replace("seven_day", "7d").replace('_', " ");
    let mut chars = normalised.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Canonical sort key. Lower wins. Keeps `5h` first, then `7d (all)`, then `7d Sonnet` /
/// `7d Opus`, other `seven_day_*` next, codenames last (alphabetical).
fn sort_priority(key: &str) -> (u8, &str) {
    match key {
        "five_hour" => (0, key),
        "seven_day" => (1, key),
        "seven_day_sonnet" => (2, key),
        "seven_day_opus" => (3, key),
        "seven_day_omelette" => (4, key),
        "seven_day_omelette_promotional" => (5, key),
        "omelette_promotional" => (5, key),
        _ if key.starts_with("seven_day_") => (6, key),
        _ => (7, key),
    }
}

fn normalise_window_key(raw: Option<&str>) -> String {
    let raw = raw.unwrap_or("five_hour").trim();
    // Accept a few friendly aliases for the keys users are most likely to type by hand.
    match raw {
        "" | "5h" => "five_hour".into(),
        "7d" => "seven_day".into(),
        "7d_sonnet" | "7d-sonnet" => "seven_day_sonnet".into(),
        "7d_opus" | "7d-opus" => "seven_day_opus".into(),
        "claude_design" | "design" => "seven_day_omelette".into(),
        other => other.into(),
    }
}

fn credentials_path() -> Result<PathBuf, FetchError> {
    if let Ok(p) = std::env::var("SPLASHBOARD_CLAUDE_CREDENTIALS") {
        return Ok(PathBuf::from(p));
    }
    dirs::home_dir()
        .map(|h| h.join(".claude").join(".credentials.json"))
        .ok_or_else(|| FetchError::Failed("could not resolve $HOME for Claude credentials".into()))
}

#[derive(Deserialize)]
struct CredentialsFile {
    #[serde(rename = "claudeAiOauth", default)]
    oauth: Option<OauthEntry>,
}

#[derive(Deserialize)]
struct OauthEntry {
    #[serde(rename = "accessToken", default)]
    access_token: String,
}

fn load_oauth_token() -> Result<String, FetchError> {
    let path = credentials_path()?;
    let bytes = std::fs::read(&path)
        .map_err(|e| FetchError::Failed(format!("read {}: {e}", path.display())))?;
    let creds: CredentialsFile = serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("parse {}: {e}", path.display())))?;
    let token = creds
        .oauth
        .map(|o| o.access_token)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            FetchError::Failed("no `claudeAiOauth.accessToken` in credentials".into())
        })?;
    Ok(token)
}

#[derive(Debug, Clone, Default)]
struct Snapshot {
    windows: Vec<NamedWindow>,
}

#[derive(Debug, Clone)]
struct NamedWindow {
    key: String,
    label: String,
    state: WindowState,
}

#[derive(Debug, Clone)]
struct WindowState {
    utilization: f64,
    resets_at: Option<DateTime<Utc>>,
}

impl Snapshot {
    fn find(&self, key: &str) -> Option<&NamedWindow> {
        self.windows.iter().find(|w| w.key == key)
    }
}

async fn fetch_snapshot(token: &str) -> Result<Snapshot, FetchError> {
    // The endpoint is undocumented; upstream tooling (claude-code-statusline) calls it as GET
    // with the OAuth bearer + beta header. POSTing here returns 405 Method Not Allowed.
    let res = http()
        .get(USAGE_URL)
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("content-type", "application/json")
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("claude_subscription request: {e}")))?;
    let status = res.status();
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("read body: {e}")))?;
    if !status.is_success() {
        return Err(FetchError::Failed(format!(
            "claude_subscription HTTP {status}"
        )));
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("parse oauth/usage: {e}")))?;
    Ok(parse_snapshot(&value))
}

fn parse_snapshot(value: &serde_json::Value) -> Snapshot {
    let Some(obj) = value.as_object() else {
        return Snapshot::default();
    };
    let mut windows: Vec<NamedWindow> = obj
        .iter()
        .filter_map(|(key, val)| parse_window_entry(key, val))
        .collect();
    windows.sort_by(|a, b| sort_priority(&a.key).cmp(&sort_priority(&b.key)));
    Snapshot { windows }
}

fn parse_window_entry(key: &str, val: &serde_json::Value) -> Option<NamedWindow> {
    // `extra_usage` carries a different schema (is_enabled / monthly_limit / used_credits …);
    // not part of the window family — skip it. Future enhancement can surface it as a separate
    // shape if anyone uses the paid extra-credit pool.
    if key == "extra_usage" {
        return None;
    }
    let obj = val.as_object()?;
    let util = obj.get("utilization").and_then(|v| v.as_f64())?;
    let resets_at = obj
        .get("resets_at")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));
    Some(NamedWindow {
        key: key.to_string(),
        label: label_for(key),
        state: WindowState {
            // Endpoint returns 0..100 percent — normalise to 0..1, allow 1.5 for over-quota so a
            // blown window still draws distinctly rather than clipping to "full".
            utilization: (util / 100.0).clamp(0.0, 1.5),
            resets_at,
        },
    })
}

fn render_body(snap: &Snapshot, target: &str, shape: Shape) -> Body {
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

fn ratio_body(snap: &Snapshot, target: &str) -> Body {
    let label = known_label(target).map(str::to_string).unwrap_or_else(|| {
        snap.find(target)
            .map(|w| w.label.clone())
            .unwrap_or_else(|| humanize_key(target))
    });
    let Some(window) = snap.find(target) else {
        return Body::Ratio(RatioData {
            value: 0.0,
            label: Some(format!("{label} · n/a")),
            denominator: Some(100),
        });
    };
    let reset = window
        .state
        .resets_at
        .map(format_reset)
        .unwrap_or_else(|| "n/a".into());
    Body::Ratio(RatioData {
        value: window.state.utilization.clamp(0.0, 1.0),
        label: Some(format!("{label} · resets {reset}")),
        denominator: Some(100),
    })
}

fn text_body(snap: &Snapshot) -> Body {
    let parts: Vec<String> = snap
        .windows
        .iter()
        .map(|w| format!("{} {}", w.label, percent(w.state.utilization)))
        .collect();
    Body::Text(TextData {
        value: if parts.is_empty() {
            "claude subscription: n/a".into()
        } else {
            parts.join(" · ")
        },
    })
}

fn text_block_body(snap: &Snapshot) -> Body {
    let lines: Vec<String> = snap.windows.iter().map(format_window_line).collect();
    Body::TextBlock(TextBlockData {
        lines: if lines.is_empty() {
            vec!["claude subscription: n/a".into()]
        } else {
            lines
        },
    })
}

fn markdown_body(snap: &Snapshot) -> Body {
    let lines: Vec<String> = snap
        .windows
        .iter()
        .map(|w| {
            let reset = w
                .state
                .resets_at
                .map(format_reset)
                .unwrap_or_else(|| "n/a".into());
            format!(
                "- **{}** — {} (resets {reset})",
                w.label,
                percent(w.state.utilization),
            )
        })
        .collect();
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: if lines.is_empty() {
            "_claude subscription unavailable_".into()
        } else {
            lines.join("\n")
        },
    })
}

fn entries_body(snap: &Snapshot) -> Body {
    let items: Vec<Entry> = snap
        .windows
        .iter()
        .map(|w| Entry {
            key: w.label.clone(),
            value: Some(format!(
                "{} (resets {})",
                percent(w.state.utilization),
                w.state
                    .resets_at
                    .map(format_reset)
                    .unwrap_or_else(|| "n/a".into()),
            )),
            status: Some(status_for(w.state.utilization)),
        })
        .collect();
    Body::Entries(EntriesData {
        items: if items.is_empty() {
            vec![Entry {
                key: "claude".into(),
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
            // Bars take u64 — store utilisation as basis points (0..=10000+) so a fully spent
            // window outranks a quarter-spent one in chart_bar.
            value: ((w.state.utilization * 10_000.0).round() as i64).max(0) as u64,
            value_label: Some(percent(w.state.utilization)),
        })
        .collect();
    Body::Bars(BarsData { bars })
}

fn badge_body(snap: &Snapshot, target: &str) -> Body {
    let label = known_label(target).map(str::to_string).unwrap_or_else(|| {
        snap.find(target)
            .map(|w| w.label.clone())
            .unwrap_or_else(|| humanize_key(target))
    });
    let Some(window) = snap.find(target) else {
        return Body::Badge(BadgeData {
            status: Status::Warn,
            label: format!("{label}: n/a"),
        });
    };
    Body::Badge(BadgeData {
        status: status_for(window.state.utilization),
        label: format!("{label} {}", percent(window.state.utilization)),
    })
}

fn timeline_body(snap: &Snapshot) -> Body {
    let mut events: Vec<TimelineEvent> = snap
        .windows
        .iter()
        .filter_map(|w| {
            w.state.resets_at.map(|ts| TimelineEvent {
                timestamp: ts.timestamp(),
                title: format!("{} resets", w.label),
                detail: Some(format!("at {}", percent(w.state.utilization))),
                status: Some(status_for(w.state.utilization)),
            })
        })
        .collect();
    events.sort_by_key(|e| e.timestamp);
    Body::Timeline(TimelineData { events })
}

fn format_window_line(window: &NamedWindow) -> String {
    let reset = window
        .state
        .resets_at
        .map(format_reset)
        .unwrap_or_else(|| "n/a".into());
    format!(
        "{}  {} · resets {reset}",
        window.label,
        percent(window.state.utilization),
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
        Shape::Ratio => samples::ratio(0.07, "5h · resets 2h"),
        Shape::Text => samples::text("5h 7% · 7d 2% · 7d Sonnet 0% · Claude Design 0%"),
        Shape::TextBlock => samples::text_block(&[
            "5h             7% · resets 2h",
            "7d             2% · resets 6d",
            "7d Sonnet      0% · resets 6d",
            "Claude Design  0% · resets n/a",
        ]),
        Shape::MarkdownTextBlock => samples::markdown(
            "- **5h** — 7% (resets 2h)\n- **7d** — 2% (resets 6d)\n- **7d Sonnet** — 0% (resets 6d)\n- **Claude Design** — 0% (resets n/a)",
        ),
        Shape::Entries => samples::entries(&[
            ("5h", "7% (resets 2h)"),
            ("7d", "2% (resets 6d)"),
            ("7d Sonnet", "0% (resets 6d)"),
            ("Claude Design", "0% (resets n/a)"),
        ]),
        Shape::Bars => Body::Bars(BarsData {
            bars: vec![
                Bar {
                    label: "5h".into(),
                    value: 700,
                    value_label: Some("7%".into()),
                },
                Bar {
                    label: "7d".into(),
                    value: 200,
                    value_label: Some("2%".into()),
                },
                Bar {
                    label: "7d Sonnet".into(),
                    value: 0,
                    value_label: Some("0%".into()),
                },
                Bar {
                    label: "Claude Design".into(),
                    value: 0,
                    value_label: Some("0%".into()),
                },
            ],
        }),
        Shape::Badge => samples::badge(Status::Ok, "5h 7%"),
        Shape::Timeline => samples::timeline(&[
            (Utc::now().timestamp() + 7_200, "5h resets", Some("at 7%")),
            (
                Utc::now().timestamp() + 6 * 86_400,
                "7d resets",
                Some("at 2%"),
            ),
        ]),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration as StdDuration;

    use super::*;

    fn ctx(options: Option<&str>, shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "claude-sub".into(),
            timeout: StdDuration::from_secs(1),
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            ..Default::default()
        }
    }

    fn window(key: &str, util: f64, resets_in_secs: Option<i64>) -> NamedWindow {
        NamedWindow {
            key: key.into(),
            label: label_for(key),
            state: WindowState {
                utilization: util,
                resets_at: resets_in_secs.map(|s| Utc::now() + chrono::Duration::seconds(s)),
            },
        }
    }

    fn snap_full() -> Snapshot {
        Snapshot {
            windows: vec![
                window("five_hour", 0.07, Some(2 * 3600)),
                window("seven_day", 0.02, Some(6 * 86_400)),
                window("seven_day_sonnet", 0.0, Some(6 * 86_400)),
                window("seven_day_omelette", 0.0, None),
            ],
        }
    }

    #[test]
    fn normalise_window_key_accepts_friendly_aliases() {
        assert_eq!(normalise_window_key(None), "five_hour");
        assert_eq!(normalise_window_key(Some("5h")), "five_hour");
        assert_eq!(normalise_window_key(Some("7d")), "seven_day");
        assert_eq!(normalise_window_key(Some("7d_sonnet")), "seven_day_sonnet");
        assert_eq!(
            normalise_window_key(Some("claude_design")),
            "seven_day_omelette"
        );
        // Unknown keys pass through so users on a future build with a new window can pick it.
        assert_eq!(normalise_window_key(Some("tangelo")), "tangelo");
    }

    #[test]
    fn known_label_maps_canonical_keys_to_ui_strings() {
        assert_eq!(known_label("five_hour"), Some("5h"));
        assert_eq!(known_label("seven_day_omelette"), Some("Claude Design"));
        assert_eq!(known_label("tangelo"), Some("Tangelo"));
        assert_eq!(known_label("future_window_x"), None);
    }

    #[test]
    fn humanize_key_replaces_seven_day_and_underscores() {
        assert_eq!(humanize_key("seven_day_future_thing"), "7d future thing");
        assert_eq!(humanize_key("future_thing"), "Future thing");
        assert_eq!(humanize_key(""), "");
    }

    #[test]
    fn parse_snapshot_skips_null_and_extra_usage_and_sorts_by_priority() {
        // Real response shape: many keys present, only some non-null. `extra_usage` carries a
        // different schema and must be skipped, the rest must appear in canonical order.
        let raw = serde_json::json!({
            "tangelo": null,
            "seven_day_omelette": { "utilization": 0.0, "resets_at": null },
            "seven_day_sonnet": { "utilization": 0.0, "resets_at": "2026-05-29T10:00:00Z" },
            "iguana_necktie": null,
            "five_hour": { "utilization": 7.0, "resets_at": "2026-05-23T10:40:00Z" },
            "seven_day": { "utilization": 2.0, "resets_at": "2026-05-29T10:00:00Z" },
            "seven_day_opus": null,
            "extra_usage": { "is_enabled": false, "utilization": null }
        });
        let snap = parse_snapshot(&raw);
        let keys: Vec<&str> = snap.windows.iter().map(|w| w.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "five_hour",
                "seven_day",
                "seven_day_sonnet",
                "seven_day_omelette"
            ]
        );
        assert!((snap.windows[0].state.utilization - 0.07).abs() < 1e-9);
        assert!(snap.windows[3].state.resets_at.is_none());
    }

    #[test]
    fn parse_snapshot_clamps_over_quota_at_one_and_a_half() {
        let raw = serde_json::json!({
            "five_hour": { "utilization": 200.0, "resets_at": null }
        });
        let snap = parse_snapshot(&raw);
        assert_eq!(snap.windows[0].state.utilization, 1.5);
    }

    #[test]
    fn parse_snapshot_handles_an_unexpected_top_level_shape() {
        let raw = serde_json::json!("not an object");
        let snap = parse_snapshot(&raw);
        assert!(snap.windows.is_empty());
    }

    #[test]
    fn status_for_bucketed_thresholds() {
        assert_eq!(status_for(0.0), Status::Ok);
        assert_eq!(status_for(0.5), Status::Ok);
        assert_eq!(status_for(0.6), Status::Warn);
        assert_eq!(status_for(0.85), Status::Error);
    }

    #[test]
    fn percent_formats_without_decimal_and_caps_over_quota_at_150() {
        assert_eq!(percent(0.07), "7%");
        assert_eq!(percent(1.0), "100%");
        assert_eq!(percent(1.5), "150%");
        assert_eq!(percent(2.0), "150%");
    }

    #[test]
    fn format_reset_picks_unit_by_distance() {
        let now = Utc::now();
        assert_eq!(format_reset(now - chrono::Duration::minutes(5)), "now");
        assert!(format_reset(now + chrono::Duration::minutes(30)).ends_with("m"));
        assert!(format_reset(now + chrono::Duration::hours(5)).ends_with("h"));
        assert!(format_reset(now + chrono::Duration::days(5)).ends_with("d"));
    }

    #[test]
    fn ratio_body_picks_requested_window_by_key() {
        let snap = snap_full();
        let Body::Ratio(r) = ratio_body(&snap, "seven_day") else {
            panic!("expected ratio");
        };
        assert!((r.value - 0.02).abs() < 1e-6);
        assert!(r.label.as_deref().unwrap_or_default().starts_with("7d"));
    }

    #[test]
    fn ratio_body_falls_back_to_zero_when_window_missing() {
        let Body::Ratio(r) = ratio_body(&Snapshot::default(), "five_hour") else {
            panic!("expected ratio");
        };
        assert_eq!(r.value, 0.0);
        assert!(r.label.as_deref().unwrap_or_default().contains("n/a"));
    }

    #[test]
    fn badge_body_falls_back_when_window_missing() {
        let Body::Badge(b) = badge_body(&Snapshot::default(), "five_hour") else {
            panic!("expected badge");
        };
        assert_eq!(b.status, Status::Warn);
        assert!(b.label.contains("n/a"));
    }

    #[test]
    fn entries_body_lists_every_present_window_in_canonical_order() {
        let Body::Entries(e) = entries_body(&snap_full()) else {
            panic!("expected entries");
        };
        assert_eq!(e.items.len(), 4);
        assert_eq!(e.items[0].key, "5h");
        assert_eq!(e.items[3].key, "Claude Design");
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
    fn text_block_body_lists_all_present_windows() {
        let Body::TextBlock(b) = text_block_body(&snap_full()) else {
            panic!("expected text_block");
        };
        assert_eq!(b.lines.len(), 4);
        assert!(b.lines[0].starts_with("5h"));
        assert!(b.lines[3].starts_with("Claude Design"));
    }

    #[test]
    fn bars_body_emits_basis_points_for_each_window() {
        let Body::Bars(b) = bars_body(&snap_full()) else {
            panic!("expected bars");
        };
        assert_eq!(b.bars.len(), 4);
        assert_eq!(b.bars[0].value, 700);
        assert_eq!(b.bars[1].value, 200);
    }

    #[test]
    fn timeline_body_skips_windows_without_reset_and_sorts_by_timestamp() {
        let Body::Timeline(t) = timeline_body(&snap_full()) else {
            panic!("expected timeline");
        };
        // `seven_day_omelette` (resets_at = None) is dropped; the other 3 remain.
        assert_eq!(t.events.len(), 3);
        assert!(t.events[0].timestamp <= t.events[1].timestamp);
        assert!(t.events[1].timestamp <= t.events[2].timestamp);
    }

    #[test]
    fn fetcher_metadata_exposes_eight_shapes_with_ratio_default() {
        let f = ClaudeSubscription;
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

    #[test]
    fn load_oauth_token_reads_credentials_via_env_override() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let creds = tmp.path().join("creds.json");
        std::fs::write(
            &creds,
            r#"{"claudeAiOauth":{"accessToken":"sk-claude-test"}}"#,
        )
        .unwrap();
        let prev = std::env::var("SPLASHBOARD_CLAUDE_CREDENTIALS").ok();
        unsafe { std::env::set_var("SPLASHBOARD_CLAUDE_CREDENTIALS", &creds) };
        let token = load_oauth_token().unwrap();
        match prev {
            Some(v) => unsafe { std::env::set_var("SPLASHBOARD_CLAUDE_CREDENTIALS", v) },
            None => unsafe { std::env::remove_var("SPLASHBOARD_CLAUDE_CREDENTIALS") },
        }
        assert_eq!(token, "sk-claude-test");
    }

    #[test]
    fn load_oauth_token_errors_on_empty_access_token() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let creds = tmp.path().join("creds.json");
        std::fs::write(&creds, r#"{"claudeAiOauth":{"accessToken":""}}"#).unwrap();
        let prev = std::env::var("SPLASHBOARD_CLAUDE_CREDENTIALS").ok();
        unsafe { std::env::set_var("SPLASHBOARD_CLAUDE_CREDENTIALS", &creds) };
        let err = load_oauth_token().unwrap_err();
        match prev {
            Some(v) => unsafe { std::env::set_var("SPLASHBOARD_CLAUDE_CREDENTIALS", v) },
            None => unsafe { std::env::remove_var("SPLASHBOARD_CLAUDE_CREDENTIALS") },
        }
        assert!(format!("{err}").contains("accessToken"));
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn fetch_surfaces_missing_credentials_as_failed() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("absent.json");
        let prev = std::env::var("SPLASHBOARD_CLAUDE_CREDENTIALS").ok();
        unsafe { std::env::set_var("SPLASHBOARD_CLAUDE_CREDENTIALS", &missing) };
        let err = ClaudeSubscription
            .fetch(&ctx(None, Some(Shape::Ratio)))
            .await
            .unwrap_err();
        match prev {
            Some(v) => unsafe { std::env::set_var("SPLASHBOARD_CLAUDE_CREDENTIALS", v) },
            None => unsafe { std::env::remove_var("SPLASHBOARD_CLAUDE_CREDENTIALS") },
        }
        assert!(format!("{err}").contains("read"));
    }
}
