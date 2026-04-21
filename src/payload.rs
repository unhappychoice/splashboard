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
/// rendered — the same shape can feed multiple renderers (e.g. `Lines` feeds both the plain
/// `simple` renderer and the big-text `bignum_tui` renderer). Config's `render = "…"` picks the
/// renderer, compat-checked against the shape at dispatch time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "shape", content = "data", rename_all = "snake_case")]
pub enum Body {
    /// Zero or more lines of text. Used by anything that emits short strings (clock, greeting,
    /// branch name) or multi-line blocks (welcome notes, todo items).
    Lines(LinesData),
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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LinesData {
    pub lines: Vec<String>,
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
    fn lines_round_trips() {
        let p = Payload {
            icon: None,
            status: None,
            format: Some("{branch}".into()),
            body: Body::Lines(LinesData {
                lines: vec!["main".into()],
            }),
        };
        assert_eq!(p, round_trip(&p));
    }

    #[test]
    fn lines_parses_from_spec_example() {
        let json = r#"{"shape":"lines","data":{"lines":["main"]}}"#;
        let p: Payload = serde_json::from_str(json).unwrap();
        assert!(matches!(p.body, Body::Lines(_)));
    }

    #[test]
    fn lines_serializes_with_expected_shape_tag() {
        let p = bare(Body::Lines(LinesData {
            lines: vec!["main".into()],
        }));
        let v: serde_json::Value = serde_json::to_value(&p).unwrap();
        assert_eq!(v["shape"], "lines");
        assert_eq!(v["data"]["lines"][0], "main");
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
    fn single_line_via_lines_variant() {
        // Replaces the old Bignum round-trip: short strings are now just a Lines body with one
        // element, shared with multi-line content. Which renderer consumes it (big-text vs plain)
        // is a config decision, not a Body shape decision.
        let p = bare(Body::Lines(LinesData {
            lines: vec!["12:34".into()],
        }));
        assert_eq!(p, round_trip(&p));
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
        let p = bare(Body::Lines(LinesData { lines: vec![] }));
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("icon"));
        assert!(!json.contains("status"));
    }
}
