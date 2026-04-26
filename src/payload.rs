#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Payload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(flatten)]
    pub body: Body,
}

/// Data-shape of a fetched payload. Variants describe the **shape of the data**, not how it's
/// rendered — the same shape can feed multiple renderers (e.g. `Text` feeds both the plain
/// `text_plain` renderer and the big-text `text_ascii` renderer). Config's `render = "…"` picks the
/// renderer, compat-checked against the shape at dispatch time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "shape", content = "data", rename_all = "snake_case")]
pub enum Body {
    /// A single string. Used by anything that emits one logical row: clock time, greeting,
    /// branch name, countdown label.
    Text(TextData),
    /// Zero or more lines of text. Used by anything intrinsically multi-line: recent commits,
    /// worktrees, welcome notes, todo items.
    TextBlock(TextBlockData),
    /// Zero or more lines of text where each line carries an optional URL. Renderers that
    /// understand hyperlinks (`list_links`) wrap rows whose `url` is `Some(_)` in OSC 8 escape
    /// sequences so modern terminals surface them as clickable. Renderers that don't honour
    /// links can ignore the urls and render the text only. Right for feeds — HN top, GitHub
    /// PRs/issues/releases — where each row has a canonical "open this" target.
    LinkedTextBlock(LinkedTextBlockData),
    /// Key/value rows. Used by system info, env dumps, anything label:value shaped.
    Entries(EntriesData),
    /// A single 0..=1 value with an optional display label. Gauges, progress bars, donuts.
    Ratio(RatioData),
    /// Sequence of unsigned integers. Sparklines, histograms, bar-height sequences.
    NumberSeries(NumberSeriesData),
    /// Sequence of `(x, y)` points, one or more series. Line charts, scatter.
    PointSeries(PointSeriesData),
    /// Labeled bars. BarChart, horizontal ranking.
    Bars(BarsData),
    /// Path to an image on disk.
    Image(ImageData),
    /// A month view anchored at a specific date, optionally with highlighted events. Calendar
    /// widget (and anything future that shows month-scale state) consumes this.
    Calendar(CalendarData),
    /// 2D grid of intensities. Used by the GitHub-style contribution graph, habit trackers, any
    /// daily-metric heatmap. Renderer maps each cell into a bucketed color.
    Heatmap(HeatmapData),
    /// Single traffic-light status + short label. Used by CI, deploy, SLO, oncall — one
    /// indicator per fetcher. Rows of badges are a composition concern handled by the nested
    /// layout (`combined_status_row`), not by a multi-entry payload.
    Badge(BadgeData),
    /// Time-stamped events, newest first. Used by git_recent_commits, deploy history, ci
    /// history. Timestamps are raw unix seconds so the renderer computes the `"3h ago"` label
    /// at draw time — keeping cached payloads from going stale.
    Timeline(TimelineData),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextData {
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextBlockData {
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LinkedTextBlockData {
    pub items: Vec<LinkedLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LinkedLine {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntriesData {
    pub items: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RatioData {
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Optional total the ratio is a fraction of (e.g. 365 when `value` is "day of year /
    /// 365"). Populated by fetchers that know the underlying count so renderers can express
    /// the fraction directly ("118 of 365"). Omitted when no sensible denominator exists —
    /// renderers fall back to percent-only output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denominator: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NumberSeriesData {
    pub values: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PointSeriesData {
    pub series: Vec<PointSeries>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PointSeries {
    pub name: String,
    pub points: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BarsData {
    pub bars: Vec<Bar>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bar {
    pub label: String,
    pub value: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageData {
    pub path: String,
}

/// 2D grid of intensities, row-major (`cells[row][col]`). The renderer picks a bucket per cell
/// via `thresholds` (explicit bucket boundaries) or an auto-quartile fallback when absent.
/// Labels are optional and may be rendered along the edges if space allows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HeatmapData {
    pub cells: Vec<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thresholds: Option<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_labels: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub col_labels: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BadgeData {
    pub status: Status,
    pub label: String,
}

/// Time-stamped events for the timeline renderer. Timestamps are unix seconds UTC; the renderer
/// formats a relative label (`"3h ago"`, `"yesterday"`, `"Apr 5"`) at draw time so cached
/// payloads don't freeze stale relative strings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimelineData {
    pub events: Vec<TimelineEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimelineEvent {
    /// Unix seconds UTC.
    pub timestamp: i64,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CalendarData {
    /// ISO year (e.g. 2026).
    pub year: i32,
    /// 1..=12.
    pub month: u8,
    /// Optional focus / "today" day, 1..=31. If set, the calendar renderer highlights it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day: Option<u8>,
    /// Extra days to mark (e.g. event days). Empty by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(payload: &Payload) -> Payload {
        let json = serde_json::to_string(payload).unwrap();
        serde_json::from_str(&json).unwrap()
    }

    fn bare(body: Body) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body,
        }
    }

    #[test]
    fn text_round_trips() {
        let p = Payload {
            icon: None,
            status: None,
            format: Some("{branch}".into()),
            body: Body::Text(TextData {
                value: "main".into(),
            }),
        };
        assert_eq!(p, round_trip(&p));
    }

    #[test]
    fn text_parses_from_spec_example() {
        let json = r#"{"shape":"text","data":{"value":"main"}}"#;
        let p: Payload = serde_json::from_str(json).unwrap();
        assert!(matches!(p.body, Body::Text(_)));
    }

    #[test]
    fn text_serializes_with_expected_shape_tag() {
        let p = bare(Body::Text(TextData {
            value: "main".into(),
        }));
        let v: serde_json::Value = serde_json::to_value(&p).unwrap();
        assert_eq!(v["shape"], "text");
        assert_eq!(v["data"]["value"], "main");
    }

    #[test]
    fn text_block_round_trips() {
        let p = bare(Body::TextBlock(TextBlockData {
            lines: vec!["feat: a".into(), "fix: b".into()],
        }));
        assert_eq!(p, round_trip(&p));
    }

    #[test]
    fn text_block_serializes_with_expected_shape_tag() {
        let p = bare(Body::TextBlock(TextBlockData {
            lines: vec!["a".into()],
        }));
        let v: serde_json::Value = serde_json::to_value(&p).unwrap();
        assert_eq!(v["shape"], "text_block");
        assert_eq!(v["data"]["lines"][0], "a");
    }

    #[test]
    fn entries_round_trips() {
        let p = bare(Body::Entries(EntriesData {
            items: vec![Entry {
                key: "uptime".into(),
                value: Some("3d".into()),
                status: Some(Status::Ok),
            }],
        }));
        assert_eq!(p, round_trip(&p));
    }

    #[test]
    fn ratio_round_trips() {
        let p = bare(Body::Ratio(RatioData {
            value: 0.73,
            label: Some("CPU".into()),
            denominator: Some(100),
        }));
        assert_eq!(p, round_trip(&p));
    }

    #[test]
    fn number_series_round_trips() {
        let p = bare(Body::NumberSeries(NumberSeriesData {
            values: vec![1, 2, 3],
        }));
        assert_eq!(p, round_trip(&p));
    }

    #[test]
    fn point_series_round_trips() {
        let p = bare(Body::PointSeries(PointSeriesData {
            series: vec![PointSeries {
                name: "temp".into(),
                points: vec![(0.0, 20.0), (1.0, 21.5)],
            }],
        }));
        assert_eq!(p, round_trip(&p));
    }

    #[test]
    fn bars_round_trips() {
        let p = bare(Body::Bars(BarsData {
            bars: vec![
                Bar {
                    label: "a".into(),
                    value: 3,
                },
                Bar {
                    label: "b".into(),
                    value: 5,
                },
            ],
        }));
        assert_eq!(p, round_trip(&p));
    }

    #[test]
    fn shape_tag_uses_snake_case() {
        let p = bare(Body::Bars(BarsData { bars: vec![] }));
        let v: serde_json::Value = serde_json::to_value(&p).unwrap();
        assert_eq!(v["shape"], "bars");
    }

    #[test]
    fn single_line_via_text_variant() {
        let p = bare(Body::Text(TextData {
            value: "12:34".into(),
        }));
        assert_eq!(p, round_trip(&p));
    }

    #[test]
    fn badge_round_trips() {
        let p = bare(Body::Badge(BadgeData {
            status: Status::Warn,
            label: "deploy degraded".into(),
        }));
        assert_eq!(p, round_trip(&p));
    }

    #[test]
    fn badge_serializes_with_expected_shape_tag() {
        let p = bare(Body::Badge(BadgeData {
            status: Status::Error,
            label: "oncall paging".into(),
        }));
        let v: serde_json::Value = serde_json::to_value(&p).unwrap();
        assert_eq!(v["shape"], "badge");
        assert_eq!(v["data"]["status"], "error");
        assert_eq!(v["data"]["label"], "oncall paging");
    }

    #[test]
    fn timeline_round_trips() {
        let p = bare(Body::Timeline(TimelineData {
            events: vec![
                TimelineEvent {
                    timestamp: 1_700_000_000,
                    title: "merged #42".into(),
                    detail: Some("feat(render): heatmap".into()),
                    status: Some(Status::Ok),
                },
                TimelineEvent {
                    timestamp: 1_699_990_000,
                    title: "opened #41".into(),
                    detail: None,
                    status: None,
                },
            ],
        }));
        assert_eq!(p, round_trip(&p));
    }

    #[test]
    fn timeline_serializes_with_expected_shape_tag() {
        let p = bare(Body::Timeline(TimelineData {
            events: vec![TimelineEvent {
                timestamp: 1_700_000_000,
                title: "x".into(),
                detail: None,
                status: None,
            }],
        }));
        let v: serde_json::Value = serde_json::to_value(&p).unwrap();
        assert_eq!(v["shape"], "timeline");
        assert_eq!(v["data"]["events"][0]["timestamp"], 1_700_000_000_i64);
        assert_eq!(v["data"]["events"][0]["title"], "x");
        assert!(v["data"]["events"][0].get("detail").is_none());
        assert!(v["data"]["events"][0].get("status").is_none());
    }

    #[test]
    fn image_round_trips() {
        let p = bare(Body::Image(ImageData {
            path: "/tmp/a.png".into(),
        }));
        assert_eq!(p, round_trip(&p));
    }

    #[test]
    fn optional_fields_absent_in_serialization() {
        let p = bare(Body::TextBlock(TextBlockData { lines: vec![] }));
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("icon"));
        assert!(!json.contains("status"));
    }
}
