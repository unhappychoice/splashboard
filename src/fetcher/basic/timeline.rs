//! `basic_timeline` — chronological events authored inline. Right for fixed milestones
//! (release history, project anniversaries) where the date is decided once and the splash just
//! displays it forever.

use chrono::DateTime;
use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload, Status, TimelineData, TimelineEvent};
use crate::render::Shape;

use super::common;

const SHAPES: &[Shape] = &[Shape::Timeline];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "events",
    type_hint: "list of {at: RFC3339 string, title, detail?, status?}",
    required: false,
    default: Some("[]"),
    description: "Timeline events. `at` accepts any RFC 3339 / ISO 8601 timestamp (e.g. `\"2024-06-01T00:00:00Z\"` or `\"2024-06-01T09:00:00+09:00\"`); the renderer formats the relative `\"3h ago\"` / `\"Jun 1\"` label at draw time.",
}];

pub struct BasicTimeline;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub events: Vec<EventConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventConfig {
    pub at: String,
    pub title: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub status: Option<Status>,
}

impl RealtimeFetcher for BasicTimeline {
    fn name(&self) -> &str {
        "basic_timeline"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Renders a `Timeline` from inline `[widget.options].events`. Each event takes an RFC 3339 `at` timestamp plus a `title`; optional `detail` and `status` decorate the row. Right for fixed milestones (releases, anniversaries, planned freezes) where the dates don't change."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        (shape == Shape::Timeline).then(|| {
            Body::Timeline(TimelineData {
                events: vec![
                    TimelineEvent {
                        timestamp: 1_704_067_200,
                        title: "v1.0 released".into(),
                        detail: Some("first major release".into()),
                        status: Some(Status::Ok),
                    },
                    TimelineEvent {
                        timestamp: 1_706_745_600,
                        title: "Feature freeze".into(),
                        detail: None,
                        status: None,
                    },
                ],
            })
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        let events: Result<Vec<TimelineEvent>, String> =
            opts.events.into_iter().map(parse_event).collect();
        match events {
            Ok(events) => common::bare(Body::Timeline(TimelineData { events })),
            Err(msg) => common::placeholder(&msg),
        }
    }
}

fn parse_event(raw: EventConfig) -> Result<TimelineEvent, String> {
    let timestamp = DateTime::parse_from_rfc3339(&raw.at)
        .map(|dt| dt.timestamp())
        .map_err(|e| format!("basic_timeline: invalid `at` value `{}`: {e}", raw.at))?;
    Ok(TimelineEvent {
        timestamp,
        title: raw.title,
        detail: raw.detail,
        status: raw.status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compute(toml_src: &str) -> Payload {
        BasicTimeline.compute(&FetchContext {
            options: Some(toml::from_str(toml_src).unwrap()),
            ..Default::default()
        })
    }

    #[test]
    fn contract() {
        assert_eq!(BasicTimeline.name(), "basic_timeline");
        assert_eq!(BasicTimeline.shapes(), &[Shape::Timeline]);
    }

    #[test]
    fn parses_rfc3339_timestamps_to_unix_seconds() {
        let p = compute(
            r#"
            [[events]]
            at = "2024-01-01T00:00:00Z"
            title = "v1.0"
            [[events]]
            at = "2024-02-01T09:00:00+09:00"
            title = "Feature freeze"
            detail = "no new features after this"
            status = "warn"
            "#,
        );
        let Body::Timeline(d) = p.body else {
            panic!("expected Timeline");
        };
        assert_eq!(d.events.len(), 2);
        assert_eq!(d.events[0].timestamp, 1_704_067_200);
        assert_eq!(d.events[0].title, "v1.0");
        assert_eq!(d.events[1].detail.as_deref(), Some("no new features after this"));
        assert_eq!(d.events[1].status, Some(Status::Warn));
    }

    #[test]
    fn invalid_timestamp_renders_placeholder() {
        let p = compute(
            r#"
            [[events]]
            at = "not-a-date"
            title = "x"
            "#,
        );
        let Body::TextBlock(d) = p.body else {
            panic!("expected placeholder");
        };
        assert!(d.lines[0].contains("invalid `at` value"));
    }

    #[test]
    fn no_events_yields_empty_timeline() {
        let p = BasicTimeline.compute(&FetchContext::default());
        let Body::Timeline(d) = p.body else {
            panic!("expected Timeline");
        };
        assert!(d.events.is_empty());
    }
}
