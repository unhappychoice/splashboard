//! `github_assigned_issues` — open issues assigned to the authenticated user.
//! Uses `/search/issues?q=is:issue+is:open+assignee:@me`. `Safe` (fixed query).

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{cache_key, parse_options, payload};
use super::items::{SearchResult, render_items};

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::TextBlock,
    Shape::Entries,
    Shape::Timeline,
];
const DEFAULT_LIMIT: u32 = 10;

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "limit",
    type_hint: "integer (1..=30)",
    required: false,
    default: Some("10"),
    description: "Maximum number of issues to show.",
}];

pub struct GithubAssignedIssues;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for GithubAssignedIssues {
    fn name(&self) -> &str {
        "github_assigned_issues"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Open issues assigned to the authenticated user across every repo. Pairs with `github_my_prs`, `github_review_requests`, and `github_notifications` to cover the personal queue."
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
                    "unhappychoice/splashboard#41 meta: widget catalog & roadmap",
                    Some("https://github.com/unhappychoice/splashboard/issues/41"),
                ),
                (
                    "unhappychoice/splashboard#17 theme system",
                    Some("https://github.com/unhappychoice/splashboard/issues/17"),
                ),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "unhappychoice/splashboard#41 meta: widget catalog & roadmap",
                "unhappychoice/splashboard#17 theme system",
            ]),
            Shape::Entries => samples::entries(&[
                ("splashboard #41", "meta: widget catalog"),
                ("splashboard #17", "theme system"),
            ]),
            Shape::Timeline => samples::timeline(&[
                (
                    1_774_000_000,
                    "splashboard#41",
                    Some("meta: widget catalog"),
                ),
                (1_773_500_000, "splashboard#17", Some("theme system")),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 30);
        let path = format!(
            "/search/issues?q=is%3Aissue+is%3Aopen+assignee%3A%40me&per_page={limit}&sort=updated"
        );
        let res: SearchResult = rest_get(&path).await?;
        Ok(payload(render_items(
            &res.items,
            ctx.shape.unwrap_or(Shape::LinkedTextBlock),
            true,
        )))
    }
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
            widget_id: "assigned-issues".into(),
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

    #[test]
    fn options_default_to_none() {
        assert!(Options::default().limit.is_none());
    }

    #[test]
    fn options_deserialize_limit() {
        let raw: toml::Value = toml::from_str("limit = 7").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.limit, Some(7));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("limit = 7\nbogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn fetcher_metadata_and_samples_cover_supported_shapes() {
        let fetcher = GithubAssignedIssues;
        assert_eq!(fetcher.name(), "github_assigned_issues");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(
            fetcher
                .description()
                .contains("assigned to the authenticated user")
        );
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(fetcher.option_schemas().len(), 1);
        assert_eq!(fetcher.option_schemas()[0].name, "limit");
        assert_eq!(fetcher.option_schemas()[0].default, Some("10"));

        let Some(Body::LinkedTextBlock(linked)) = fetcher.sample_body(Shape::LinkedTextBlock)
        else {
            panic!("expected linked text block sample");
        };
        assert_eq!(
            linked.items[0].text,
            "unhappychoice/splashboard#41 meta: widget catalog & roadmap"
        );
        assert_eq!(
            linked.items[1].url.as_deref(),
            Some("https://github.com/unhappychoice/splashboard/issues/17")
        );

        let Some(Body::TextBlock(text)) = fetcher.sample_body(Shape::TextBlock) else {
            panic!("expected text block sample");
        };
        assert_eq!(text.lines[1], "unhappychoice/splashboard#17 theme system");

        let Some(Body::Entries(entries)) = fetcher.sample_body(Shape::Entries) else {
            panic!("expected entries sample");
        };
        assert_eq!(entries.items[0].key, "splashboard #41");
        assert_eq!(entries.items[1].value.as_deref(), Some("theme system"));

        let Some(Body::Timeline(timeline)) = fetcher.sample_body(Shape::Timeline) else {
            panic!("expected timeline sample");
        };
        assert_eq!(timeline.events[0].title, "splashboard#41");
        assert_eq!(timeline.events[1].detail.as_deref(), Some("theme system"));
        assert!(fetcher.sample_body(Shape::Text).is_none());
    }

    #[test]
    fn cache_key_is_stable_for_identical_context_and_changes_with_shape() {
        let fetcher = GithubAssignedIssues;
        let linked = ctx(
            Some("limit = 10"),
            Some(Shape::LinkedTextBlock),
            Some("compact"),
        );
        let linked_again = ctx(
            Some("limit = 10"),
            Some(Shape::LinkedTextBlock),
            Some("compact"),
        );
        let timeline = ctx(Some("limit = 10"), Some(Shape::Timeline), Some("compact"));

        assert_eq!(fetcher.cache_key(&linked), fetcher.cache_key(&linked_again));
        assert_ne!(fetcher.cache_key(&linked), fetcher.cache_key(&timeline));
    }

    #[test]
    fn fetch_rejects_invalid_options_before_auth_lookup() {
        let fetcher = GithubAssignedIssues;
        let err = run_async(fetcher.fetch(&ctx(Some("limit = \"many\""), None, None)))
            .expect_err("invalid options should fail");
        let FetchError::Failed(message) = err else {
            panic!("expected fetch failure");
        };
        assert!(message.contains("invalid options"));
    }

    #[test]
    fn fetch_without_token_surfaces_auth_error_for_default_limit() {
        let _guard = EnvGuard::set(&[("GH_TOKEN", None), ("GITHUB_TOKEN", None)]);
        let fetcher = GithubAssignedIssues;
        let err = run_async(fetcher.fetch(&ctx(None, Some(Shape::Entries), None)))
            .expect_err("missing token should fail");
        let FetchError::Failed(message) = err else {
            panic!("expected fetch failure");
        };
        assert_eq!(message, "GH_TOKEN / GITHUB_TOKEN not set");
    }

    #[test]
    fn fetch_without_token_surfaces_auth_error_for_clamped_limit() {
        let _guard = EnvGuard::set(&[("GH_TOKEN", None), ("GITHUB_TOKEN", None)]);
        let fetcher = GithubAssignedIssues;
        let err = run_async(fetcher.fetch(&ctx(
            Some("limit = 50"),
            Some(Shape::Timeline),
            Some("compact"),
        )))
        .expect_err("missing token should fail");
        let FetchError::Failed(message) = err else {
            panic!("expected fetch failure");
        };
        assert_eq!(message, "GH_TOKEN / GITHUB_TOKEN not set");
    }
}
