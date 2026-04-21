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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "render", content = "data", rename_all = "snake_case")]
pub enum Body {
    Text(TextData),
    List(ListData),
    Gauge(GaugeData),
    Sparkline(SparklineData),
    LineChart(LineChartData),
    BarChart(BarChartData),
    Bignum(BignumData),
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
pub struct TextData {
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ListData {
    pub items: Vec<ListItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ListItem {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GaugeData {
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SparklineData {
    pub values: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LineChartData {
    pub series: Vec<LineSeries>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LineSeries {
    pub name: String,
    pub points: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BarChartData {
    pub bars: Vec<Bar>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bar {
    pub label: String,
    pub value: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BignumData {
    pub text: String,
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
    fn text_round_trips() {
        let p = Payload {
            icon: None,
            status: None,
            format: Some("{branch}".into()),
            body: Body::Text(TextData {
                lines: vec!["main".into()],
            }),
        };
        assert_eq!(p, round_trip(&p));
    }

    #[test]
    fn text_parses_from_spec_example() {
        let json = r#"{"render":"text","data":{"lines":["main"]}}"#;
        let p: Payload = serde_json::from_str(json).unwrap();
        assert!(matches!(p.body, Body::Text(_)));
    }

    #[test]
    fn text_serializes_with_expected_shape() {
        let p = bare(Body::Text(TextData {
            lines: vec!["main".into()],
        }));
        let v: serde_json::Value = serde_json::to_value(&p).unwrap();
        assert_eq!(v["render"], "text");
        assert_eq!(v["data"]["lines"][0], "main");
    }

    #[test]
    fn list_round_trips() {
        let p = bare(Body::List(ListData {
            items: vec![ListItem {
                key: "uptime".into(),
                value: Some("3d".into()),
                status: Some(Status::Ok),
            }],
        }));
        assert_eq!(p, round_trip(&p));
    }

    #[test]
    fn gauge_round_trips() {
        let p = bare(Body::Gauge(GaugeData {
            value: 0.73,
            label: Some("CPU".into()),
        }));
        assert_eq!(p, round_trip(&p));
    }

    #[test]
    fn sparkline_round_trips() {
        let p = bare(Body::Sparkline(SparklineData {
            values: vec![1, 2, 3],
        }));
        assert_eq!(p, round_trip(&p));
    }

    #[test]
    fn line_chart_round_trips() {
        let p = bare(Body::LineChart(LineChartData {
            series: vec![LineSeries {
                name: "temp".into(),
                points: vec![(0.0, 20.0), (1.0, 21.5)],
            }],
        }));
        assert_eq!(p, round_trip(&p));
    }

    #[test]
    fn bar_chart_round_trips() {
        let p = bare(Body::BarChart(BarChartData {
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
    fn bar_chart_tag_uses_snake_case() {
        let p = bare(Body::BarChart(BarChartData { bars: vec![] }));
        let v: serde_json::Value = serde_json::to_value(&p).unwrap();
        assert_eq!(v["render"], "bar_chart");
    }

    #[test]
    fn bignum_round_trips() {
        let p = bare(Body::Bignum(BignumData {
            text: "12:34".into(),
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
        let p = bare(Body::Text(TextData { lines: vec![] }));
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("icon"));
        assert!(!json.contains("status"));
    }
}
