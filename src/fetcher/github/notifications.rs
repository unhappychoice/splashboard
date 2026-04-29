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
    use std::future::Future;
    use std::time::Duration;

    use super::*;

    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        restore: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn set(pairs: &[(&'static str, Option<&str>)]) -> Self {
            let lock = crate::paths::TEST_ENV_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let restore = pairs
                .iter()
                .map(|(key, value)| {
                    let previous = std::env::var(key).ok();
                    match value {
                        Some(value) => unsafe { std::env::set_var(key, value) },
                        None => unsafe { std::env::remove_var(key) },
                    }
                    (*key, previous)
                })
                .collect();
            Self {
                _lock: lock,
                restore,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            self.restore.iter().for_each(|(key, value)| match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            });
        }
    }

    fn ctx(options: Option<&str>, shape: Option<Shape>, format: Option<&str>) -> FetchContext {
        FetchContext {
            widget_id: "notifications".into(),
            format: format.map(str::to_string),
            timeout: Duration::from_secs(1),
            file_format: None,
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            timezone: None,
            locale: None,
        }
    }

    fn run_async<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

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
    fn options_default_to_none() {
        assert!(Options::default().limit.is_none());
        assert!(Options::default().all.is_none());
    }

    #[test]
    fn options_deserialize_limit_and_all() {
        let raw: toml::Value = toml::from_str("limit = 7\nall = true").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.limit, Some(7));
        assert_eq!(opts.all, Some(true));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("bogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn fetcher_metadata_and_samples_cover_supported_shapes() {
        let fetcher = GithubNotifications;
        assert_eq!(fetcher.name(), "github_notifications");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("notification inbox"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(fetcher.option_schemas().len(), 2);
        assert_eq!(fetcher.option_schemas()[0].name, "limit");
        assert_eq!(fetcher.option_schemas()[1].name, "all");

        let Some(Body::LinkedTextBlock(linked)) = fetcher.sample_body(Shape::LinkedTextBlock)
        else {
            panic!("expected linked text block sample");
        };
        assert_eq!(
            linked.items[0].url.as_deref(),
            Some("https://github.com/unhappychoice/splashboard/pull/12")
        );

        let Some(Body::TextBlock(text)) = fetcher.sample_body(Shape::TextBlock) else {
            panic!("expected text block sample");
        };
        assert_eq!(text.lines[1], "ratatui mention: rfc: themes");

        let Some(Body::Entries(entries)) = fetcher.sample_body(Shape::Entries) else {
            panic!("expected entries sample");
        };
        assert_eq!(entries.items[0].key, "splashboard");
        assert_eq!(
            entries.items[1].value.as_deref(),
            Some("mention: rfc: themes")
        );

        let Some(Body::Timeline(timeline)) = fetcher.sample_body(Shape::Timeline) else {
            panic!("expected timeline sample");
        };
        assert_eq!(timeline.events[0].title, "splashboard");
        assert_eq!(
            timeline.events[1].detail.as_deref(),
            Some("mention: rfc: themes")
        );

        let Some(Body::Badge(badge)) = fetcher.sample_body(Shape::Badge) else {
            panic!("expected badge sample");
        };
        assert_eq!(badge.status, Status::Warn);
        assert_eq!(badge.label, "2 notifications");
        assert!(fetcher.sample_body(Shape::Text).is_none());
    }

    #[test]
    fn cache_key_changes_with_shape_and_format() {
        let fetcher = GithubNotifications;
        let linked = fetcher.cache_key(&ctx(None, Some(Shape::LinkedTextBlock), None));
        let badge = fetcher.cache_key(&ctx(None, Some(Shape::Badge), None));
        let markdown = fetcher.cache_key(&ctx(None, Some(Shape::LinkedTextBlock), Some("md")));
        assert_ne!(linked, badge);
        assert_ne!(linked, markdown);
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
        let items: Vec<Notification> = (0..BADGE_PAGE).map(|_| sample()).collect();
        let body = render_body(&items, Shape::Badge, true);
        let Body::Badge(b) = body else {
            panic!("expected badge")
        };
        assert_eq!(b.label, format!("{BADGE_PAGE}+ notifications"));
    }

    #[test]
    fn badge_label_keeps_singular_form() {
        let badge = notifications_badge(1, false);
        assert_eq!(badge.status, Status::Warn);
        assert_eq!(badge.label, "1 notification");
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

    #[test]
    fn linked_body_converts_subject_api_url_to_html_url() {
        let body = render_body(&[sample()], Shape::LinkedTextBlock, false);
        let Body::LinkedTextBlock(linked) = body else {
            panic!("expected linked text block");
        };
        assert_eq!(
            linked.items[0].url.as_deref(),
            Some("https://github.com/unhappychoice/splashboard/pulls/1")
        );
        assert!(linked.items[0].text.contains("review_requested"));
    }

    #[test]
    fn linked_body_leaves_rows_unlinked_when_subject_url_cannot_be_mapped() {
        let mut missing = sample();
        missing.subject.url = None;
        assert!(subject_html_url(&missing).is_none());

        let mut wrong_prefix = sample();
        wrong_prefix.subject.url = Some("https://api.github.com/user".into());
        assert!(subject_html_url(&wrong_prefix).is_none());
    }

    #[test]
    fn entries_body_shortens_repo_name_and_keeps_reason() {
        let body = render_body(&[sample()], Shape::Entries, false);
        let Body::Entries(entries) = body else {
            panic!("expected entries");
        };
        assert_eq!(entries.items[0].key, "splashboard");
        assert_eq!(
            entries.items[0].value.as_deref(),
            Some("review_requested: feat: heatmap")
        );
    }

    #[test]
    fn timeline_body_uses_short_repo_and_parsed_timestamp() {
        let body = render_body(&[sample()], Shape::Timeline, false);
        let Body::Timeline(timeline) = body else {
            panic!("expected timeline");
        };
        assert_eq!(timeline.events[0].title, "splashboard");
        assert_eq!(
            timeline.events[0].detail.as_deref(),
            Some("review_requested: feat: heatmap")
        );
        assert!(timeline.events[0].timestamp > 0);
    }

    #[test]
    fn fetch_rejects_unknown_options() {
        let err = run_async(GithubNotifications.fetch(&ctx(Some("bogus = true"), None, None)))
            .unwrap_err();
        assert!(matches!(err, FetchError::Failed(message) if message.contains("unknown field")));
    }

    #[test]
    fn fetch_without_token_fails_after_building_default_list_path() {
        let _guard = EnvGuard::set(&[("GH_TOKEN", None), ("GITHUB_TOKEN", None)]);
        let err = run_async(GithubNotifications.fetch(&ctx(None, None, None))).unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(message) if message == "GH_TOKEN / GITHUB_TOKEN not set"
        ));
    }

    #[test]
    fn fetch_without_token_covers_badge_shape_and_clamped_limit_path() {
        let _guard = EnvGuard::set(&[("GH_TOKEN", None), ("GITHUB_TOKEN", None)]);
        let err = run_async(GithubNotifications.fetch(&ctx(
            Some("limit = 99\nall = true"),
            Some(Shape::Badge),
            None,
        )))
        .unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(message) if message == "GH_TOKEN / GITHUB_TOKEN not set"
        ));
    }
}
