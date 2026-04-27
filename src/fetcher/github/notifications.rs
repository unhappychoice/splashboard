//! `github_notifications` — the authenticated user's current notification inbox. Uses the
//! `/notifications` endpoint (unread by default). `Safe` — fixed-path, authenticated.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData, Payload, Status,
    TextBlockData, TimelineData, TimelineEvent,
};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{cache_key, parse_options, parse_timestamp, payload};

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::TextBlock,
    Shape::Entries,
    Shape::Timeline,
    Shape::Badge,
];
const DEFAULT_LIMIT: u32 = 10;
/// `/notifications` `per_page` ceiling — also the page size used for the `Badge` shape so the
/// reported count is exact up to 50 and explicitly capped (`"50+"`) past it.
const BADGE_PAGE: u32 = 50;

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
    fn description(&self) -> &'static str {
        "The authenticated user's GitHub notification inbox, unread by default, with each row tagged by reason (mention, review_requested, etc.). Set `all = true` to include already-read notifications."
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
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "splashboard review_requested: feat: heatmap",
                    Some("https://github.com/unhappychoice/splashboard/pull/12"),
                ),
                (
                    "ratatui mention: rfc: themes",
                    Some("https://github.com/ratatui/ratatui/issues/345"),
                ),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "splashboard review_requested: feat: heatmap",
                "ratatui mention: rfc: themes",
            ]),
            Shape::Entries => samples::entries(&[
                ("splashboard", "review_requested: feat: heatmap"),
                ("ratatui", "mention: rfc: themes"),
            ]),
            Shape::Timeline => samples::timeline(&[
                (
                    1_774_000_000,
                    "splashboard",
                    Some("review_requested: feat: heatmap"),
                ),
                (1_773_800_000, "ratatui", Some("mention: rfc: themes")),
            ]),
            Shape::Badge => samples::badge(Status::Warn, "2 notifications"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        // Badge needs the inbox total, not a list slice — bypass `limit` and request the API
        // max so 0..=50 is exact. List shapes still honour the user's `limit` for display.
        let per_page = if shape == Shape::Badge {
            BADGE_PAGE
        } else {
            opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, BADGE_PAGE)
        };
        let all = opts.all.unwrap_or(false);
        let path = format!("/notifications?per_page={per_page}&all={all}");
        let items: Vec<Notification> = rest_get(&path).await?;
        let capped = items.len() as u32 >= per_page;
        Ok(payload(render_body(&items, shape, capped)))
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
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Repo {
    full_name: String,
}

fn render_body(items: &[Notification], shape: Shape, capped: bool) -> Body {
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
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: items
                .iter()
                .map(|n| LinkedLine {
                    text: format!(
                        "{} {}: {}",
                        n.repository.full_name, n.reason, n.subject.title
                    ),
                    url: subject_html_url(n),
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
        Shape::Badge => Body::Badge(notifications_badge(items.len(), capped)),
        _ => Body::TextBlock(TextBlockData {
            lines: items
                .iter()
                .map(|n| {
                    format!(
                        "{} {}: {}",
                        n.repository.full_name, n.reason, n.subject.title
                    )
                })
                .collect(),
        }),
    }
}

fn notifications_badge(count: usize, capped: bool) -> BadgeData {
    BadgeData {
        status: if count == 0 { Status::Ok } else { Status::Warn },
        label: match (count, capped) {
            (0, _) => "inbox zero".into(),
            (1, false) => "1 notification".into(),
            (n, true) => format!("{n}+ notifications"),
            (n, false) => format!("{n} notifications"),
        },
    }
}

fn short_repo(full: &str) -> String {
    full.split_once('/')
        .map(|(_, r)| r.to_string())
        .unwrap_or_else(|| full.to_string())
}

/// Converts a notification's subject API URL (`api.github.com/repos/.../pulls/N`) to the
/// browser-friendly HTML URL (`github.com/.../pulls/N`). Returns `None` when the URL is missing
/// or doesn't match the expected prefix; callers fall back to leaving the row unlinked.
fn subject_html_url(n: &Notification) -> Option<String> {
    let url = n.subject.url.as_deref()?;
    let rest = url.strip_prefix("https://api.github.com/repos/")?;
    Some(format!("https://github.com/{rest}"))
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
                url: Some("https://api.github.com/repos/unhappychoice/splashboard/pulls/1".into()),
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
    fn badge_body_collapses_to_count_pill() {
        let body = render_body(&[sample(), sample()], Shape::Badge, false);
        let Body::Badge(b) = body else {
            panic!("expected badge")
        };
        assert_eq!(b.status, Status::Warn);
        assert_eq!(b.label, "2 notifications");
        let body = render_body(&[], Shape::Badge, false);
        let Body::Badge(b) = body else {
            panic!("expected badge")
        };
        assert_eq!(b.status, Status::Ok);
        assert_eq!(b.label, "inbox zero");
    }

    #[test]
    fn badge_label_marks_capped_count_with_plus() {
        // Simulate a full BADGE_PAGE response — the page is exhausted, so the true total may be
        // higher and the label has to admit that with `"50+"`.
        let items: Vec<Notification> = (0..BADGE_PAGE).map(|_| sample()).collect();
        let body = render_body(&items, Shape::Badge, true);
        let Body::Badge(b) = body else {
            panic!("expected badge")
        };
        assert_eq!(b.label, format!("{BADGE_PAGE}+ notifications"));
    }

    #[test]
    fn text_block_body_composes_repo_reason_title() {
        let body = render_body(&[sample()], Shape::TextBlock, false);
        let Body::TextBlock(l) = body else {
            panic!("expected text_block");
        };
        assert!(l.lines[0].contains("unhappychoice/splashboard"));
        assert!(l.lines[0].contains("review_requested"));
        assert!(l.lines[0].contains("feat: heatmap"));
    }
}
