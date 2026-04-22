//! `github_notifications` — the authenticated user's current notification inbox. Uses the
//! `/notifications` endpoint (unread by default). `Safe` — fixed-path, authenticated.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    Body, EntriesData, Entry, LinesData, Payload, TimelineData, TimelineEvent,
};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{cache_key, parse_options, parse_timestamp, payload, placeholder};

const SHAPES: &[Shape] = &[Shape::Lines, Shape::Entries, Shape::Timeline];
const DEFAULT_LIMIT: u32 = 10;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=50)",
        required: false,
        default: Some("10"),
        description: "Maximum number of notifications to show.",
    },
    OptionSchema {
        name: "all",
        type_hint: "boolean",
        required: false,
        default: Some("false"),
        description: "Include already-read notifications. Defaults to unread-only.",
    },
];

pub struct GithubNotifications;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub all: Option<bool>,
}

#[async_trait]
impl Fetcher for GithubNotifications {
    fn name(&self) -> &str {
        "github_notifications"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        cache_key(self.name(), ctx, "")
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Lines => samples::lines(&[
                "splashboard review_requested: feat: heatmap",
                "ratatui mention: rfc: themes",
            ]),
            Shape::Entries => samples::entries(&[
                ("splashboard", "review_requested: feat: heatmap"),
                ("ratatui", "mention: rfc: themes"),
            ]),
            Shape::Timeline => samples::timeline(&[
                (1_774_000_000, "splashboard", Some("review_requested: feat: heatmap")),
                (1_773_800_000, "ratatui", Some("mention: rfc: themes")),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return Ok(placeholder(&msg)),
        };
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 50);
        let all = opts.all.unwrap_or(false);
        let path = format!("/notifications?per_page={limit}&all={all}");
        let items: Vec<Notification> = rest_get(&path).await?;
        Ok(payload(render_body(
            &items,
            ctx.shape.unwrap_or(Shape::Lines),
        )))
    }
}

#[derive(Debug, Deserialize)]
struct Notification {
    reason: String,
    updated_at: String,
    subject: Subject,
    repository: Repo,
}

#[derive(Debug, Deserialize)]
struct Subject {
    title: String,
}

#[derive(Debug, Deserialize)]
struct Repo {
    full_name: String,
}

fn render_body(items: &[Notification], shape: Shape) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: items
                .iter()
                .map(|n| Entry {
                    key: short_repo(&n.repository.full_name),
                    value: Some(format!("{}: {}", n.reason, n.subject.title)),
                    status: None,
                })
                .collect(),
        }),
        Shape::Timeline => Body::Timeline(TimelineData {
            events: items
                .iter()
                .map(|n| TimelineEvent {
                    timestamp: parse_timestamp(&n.updated_at),
                    title: short_repo(&n.repository.full_name),
                    detail: Some(format!("{}: {}", n.reason, n.subject.title)),
                    status: None,
                })
                .collect(),
        }),
        _ => Body::Lines(LinesData {
            lines: items
                .iter()
                .map(|n| format!("{} {}: {}", n.repository.full_name, n.reason, n.subject.title))
                .collect(),
        }),
    }
}

fn short_repo(full: &str) -> String {
    full.split_once('/').map(|(_, r)| r.to_string()).unwrap_or_else(|| full.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Notification {
        Notification {
            reason: "review_requested".into(),
            updated_at: "2026-04-22T10:15:30Z".into(),
            subject: Subject {
                title: "feat: heatmap".into(),
            },
            repository: Repo {
                full_name: "unhappychoice/splashboard".into(),
            },
        }
    }

    #[test]
    fn short_repo_drops_owner() {
        assert_eq!(short_repo("unhappychoice/splashboard"), "splashboard");
        assert_eq!(short_repo("singleton"), "singleton");
    }

    #[test]
    fn lines_body_composes_repo_reason_title() {
        let body = render_body(&[sample()], Shape::Lines);
        let Body::Lines(l) = body else {
            panic!("expected lines");
        };
        assert!(l.lines[0].contains("unhappychoice/splashboard"));
        assert!(l.lines[0].contains("review_requested"));
        assert!(l.lines[0].contains("feat: heatmap"));
    }
}
