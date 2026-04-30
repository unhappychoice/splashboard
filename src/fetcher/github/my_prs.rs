//! `github_my_prs` — open pull requests authored by the authenticated user across every repo.
//! Uses `/search/issues?q=is:pr+is:open+author:@me`. The query is fixed (no config-controlled
//! path) so the fetcher is `Safe`.

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
    description: "Maximum number of PRs to show.",
}];

pub struct GithubMyPrs;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for GithubMyPrs {
    fn name(&self) -> &str {
        "github_my_prs"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Open pull requests authored by the authenticated user across every repo. Use `github_repo_prs` instead to list PRs against one specific repo regardless of who opened them."
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
                    "unhappychoice/splashboard#54 feat(docs): generate widget catalogue",
                    Some("https://github.com/unhappychoice/splashboard/pull/54"),
                ),
                (
                    "unhappychoice/splashboard#51 feat(fetcher): split clock options",
                    Some("https://github.com/unhappychoice/splashboard/pull/51"),
                ),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "unhappychoice/splashboard#54 feat(docs): generate widget catalogue",
                "unhappychoice/splashboard#51 feat(fetcher): split clock options",
            ]),
            Shape::Entries => samples::entries(&[
                ("splashboard #54", "feat(docs): widget catalogue"),
                ("splashboard #51", "feat(fetcher): split clock options"),
            ]),
            Shape::Timeline => samples::timeline(&[
                (
                    1_774_000_000,
                    "splashboard#54",
                    Some("feat(docs): widget catalogue"),
                ),
                (
                    1_773_800_000,
                    "splashboard#51",
                    Some("feat(fetcher): split clock options"),
                ),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 30);
        let path = format!(
            "/search/issues?q=is%3Apr+is%3Aopen+author%3A%40me&per_page={limit}&sort=updated"
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
            widget_id: "my-prs".into(),
            format: format.map(String::from),
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
        let opts = Options::default();
        assert!(opts.limit.is_none());
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
        let fetcher = GithubMyPrs;
        assert_eq!(fetcher.name(), "github_my_prs");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("authenticated user"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.option_schemas().len(), 1);
        assert_eq!(fetcher.option_schemas()[0].name, "limit");
        assert_eq!(fetcher.option_schemas()[0].default, Some("10"));
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);

        let Some(Body::LinkedTextBlock(linked)) = fetcher.sample_body(Shape::LinkedTextBlock)
        else {
            panic!("expected linked text block sample");
        };
        assert_eq!(
            linked.items[0].text,
            "unhappychoice/splashboard#54 feat(docs): generate widget catalogue"
        );
        assert_eq!(
            linked.items[1].url.as_deref(),
            Some("https://github.com/unhappychoice/splashboard/pull/51")
        );

        let Some(Body::TextBlock(text)) = fetcher.sample_body(Shape::TextBlock) else {
            panic!("expected text block sample");
        };
        assert_eq!(
            text.lines[1],
            "unhappychoice/splashboard#51 feat(fetcher): split clock options"
        );

        let Some(Body::Entries(entries)) = fetcher.sample_body(Shape::Entries) else {
            panic!("expected entries sample");
        };
        assert_eq!(entries.items[0].key, "splashboard #54");
        assert_eq!(
            entries.items[1].value.as_deref(),
            Some("feat(fetcher): split clock options")
        );

        let Some(Body::Timeline(timeline)) = fetcher.sample_body(Shape::Timeline) else {
            panic!("expected timeline sample");
        };
        assert_eq!(timeline.events[0].title, "splashboard#54");
        assert_eq!(
            timeline.events[1].detail.as_deref(),
            Some("feat(fetcher): split clock options")
        );
        assert!(fetcher.sample_body(Shape::Text).is_none());
    }

    #[test]
    fn cache_key_is_stable_for_identical_context_and_changes_with_shape() {
        let fetcher = GithubMyPrs;
        let linked = ctx(
            Some("limit = 3"),
            Some(Shape::LinkedTextBlock),
            Some("compact"),
        );
        let linked_again = ctx(
            Some("limit = 3"),
            Some(Shape::LinkedTextBlock),
            Some("compact"),
        );
        let entries = ctx(Some("limit = 3"), Some(Shape::Entries), Some("compact"));

        assert_eq!(fetcher.cache_key(&linked), fetcher.cache_key(&linked_again));
        assert_ne!(fetcher.cache_key(&linked), fetcher.cache_key(&entries));
    }

    #[test]
    fn fetch_rejects_invalid_options_before_auth_lookup() {
        let fetcher = GithubMyPrs;
        let err = run_async(fetcher.fetch(&ctx(Some("limit = \"many\""), None, None)))
            .expect_err("invalid options should fail");
        let FetchError::Failed(message) = err else {
            panic!("expected fetch failure");
        };
        assert!(message.contains("invalid options"));
    }

    #[test]
    fn fetch_without_token_surfaces_auth_error() {
        let _guard = EnvGuard::set(&[("GH_TOKEN", None), ("GITHUB_TOKEN", None)]);
        let fetcher = GithubMyPrs;
        let err = run_async(fetcher.fetch(&ctx(Some("limit = 50"), None, None)))
            .expect_err("missing token should fail");
        let FetchError::Failed(message) = err else {
            panic!("expected fetch failure");
        };
        assert_eq!(message, "GH_TOKEN / GITHUB_TOKEN not set");
    }
}
