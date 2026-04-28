//! `github_good_first_issues` — open issues labelled `good-first-issue`. Scoped to a single
//! repo when `repo` is set, otherwise a global search.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{RepoSlug, cache_key, parse_options, payload};
use super::items::{SearchResult, render_items};

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::TextBlock,
    Shape::Entries,
    Shape::Timeline,
];
const DEFAULT_LIMIT: u32 = 10;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "repo",
        type_hint: "\"owner/name\"",
        required: false,
        default: None,
        description: "Restrict the search to one repo. Omit to search across all of GitHub.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Maximum number of issues to show.",
    },
    OptionSchema {
        name: "label",
        type_hint: "string",
        required: false,
        default: Some("\"good first issue\""),
        description: "Label to search for. Defaults to the canonical good-first-issue label.",
    },
];

pub struct GithubGoodFirstIssues;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub label: Option<String>,
}

#[async_trait]
impl Fetcher for GithubGoodFirstIssues {
    fn name(&self) -> &str {
        "github_good_first_issues"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Open issues labelled good-first-issue, either across all of GitHub or scoped to one repo via the `repo` option. Aimed at onboarding contributors and idle-browsing for something to pick up."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let extra = repo_for_key(ctx);
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "ratatui/ratatui#999 improve docs for Paragraph",
                    Some("https://github.com/ratatui/ratatui/issues/999"),
                ),
                (
                    "tokio-rs/console#123 add quickstart section",
                    Some("https://github.com/tokio-rs/console/issues/123"),
                ),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "ratatui/ratatui#999 improve docs for Paragraph",
                "tokio-rs/console#123 add quickstart section",
            ]),
            Shape::Entries => samples::entries(&[
                ("ratatui #999", "improve docs for Paragraph"),
                ("console #123", "add quickstart section"),
            ]),
            Shape::Timeline => samples::timeline(&[
                (
                    1_774_000_000,
                    "ratatui/ratatui#999",
                    Some("improve docs for Paragraph"),
                ),
                (
                    1_773_500_000,
                    "tokio-rs/console#123",
                    Some("add quickstart section"),
                ),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 30);
        let label = opts
            .label
            .as_deref()
            .unwrap_or("good first issue")
            .replace(' ', "+");
        let mut query = format!("label%3A%22{label}%22+is%3Aissue+is%3Aopen");
        let include_repo = opts.repo.is_none();
        if let Some(raw) = opts.repo.as_deref() {
            let slug = RepoSlug::parse(raw)
                .ok_or_else(|| FetchError::Failed(format!("invalid repo option: {raw:?}")))?;
            query.push_str(&format!("+repo%3A{}%2F{}", slug.owner, slug.name));
        }
        let path = format!("/search/issues?q={query}&sort=updated&per_page={limit}");
        let res: SearchResult = rest_get(&path).await?;
        Ok(payload(render_items(
            &res.items,
            ctx.shape.unwrap_or(Shape::LinkedTextBlock),
            include_repo,
        )))
    }
}

fn repo_for_key(ctx: &FetchContext) -> String {
    ctx.options
        .as_ref()
        .and_then(|v| v.get("repo"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_default()
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

    fn ctx(options: Option<&str>, shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "good-first".into(),
            format: Some("compact".into()),
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
        assert!(opts.repo.is_none());
        assert!(opts.limit.is_none());
        assert!(opts.label.is_none());
    }

    #[test]
    fn options_deserialize_repo_limit_and_label() {
        let raw: toml::Value =
            toml::from_str("repo = \"foo/bar\"\nlimit = 7\nlabel = \"help wanted\"").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.repo.as_deref(), Some("foo/bar"));
        assert_eq!(opts.limit, Some(7));
        assert_eq!(opts.label.as_deref(), Some("help wanted"));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("repo = \"foo/bar\"\nbogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn fetcher_metadata_and_samples_cover_supported_shapes() {
        let fetcher = GithubGoodFirstIssues;
        assert_eq!(fetcher.name(), "github_good_first_issues");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("good-first-issue"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.option_schemas().len(), 3);
        assert_eq!(fetcher.option_schemas()[0].name, "repo");
        assert_eq!(fetcher.option_schemas()[1].name, "limit");
        assert_eq!(fetcher.option_schemas()[2].name, "label");

        let Some(Body::LinkedTextBlock(linked)) = fetcher.sample_body(Shape::LinkedTextBlock)
        else {
            panic!("expected linked text block sample");
        };
        assert_eq!(
            linked.items[0].text,
            "ratatui/ratatui#999 improve docs for Paragraph"
        );
        assert_eq!(
            linked.items[1].url.as_deref(),
            Some("https://github.com/tokio-rs/console/issues/123")
        );

        let Some(Body::TextBlock(text)) = fetcher.sample_body(Shape::TextBlock) else {
            panic!("expected text block sample");
        };
        assert_eq!(text.lines[1], "tokio-rs/console#123 add quickstart section");

        let Some(Body::Entries(entries)) = fetcher.sample_body(Shape::Entries) else {
            panic!("expected entries sample");
        };
        assert_eq!(entries.items[0].key, "ratatui #999");
        assert_eq!(
            entries.items[1].value.as_deref(),
            Some("add quickstart section")
        );

        let Some(Body::Timeline(timeline)) = fetcher.sample_body(Shape::Timeline) else {
            panic!("expected timeline sample");
        };
        assert_eq!(timeline.events[0].title, "ratatui/ratatui#999");
        assert_eq!(
            timeline.events[1].detail.as_deref(),
            Some("add quickstart section")
        );
        assert!(fetcher.sample_body(Shape::Text).is_none());
    }

    #[test]
    fn cache_key_changes_with_repo_option() {
        let fetcher = GithubGoodFirstIssues;
        let a = fetcher.cache_key(&ctx(Some("repo = \"foo/bar\""), Some(Shape::Entries)));
        let b = fetcher.cache_key(&ctx(Some("repo = \"foo/baz\""), Some(Shape::Entries)));
        assert_ne!(a, b);
    }

    #[test]
    fn repo_for_key_prefers_explicit_repo_option() {
        assert_eq!(
            repo_for_key(&ctx(Some("repo = \"foo/bar\""), Some(Shape::Entries))),
            "foo/bar"
        );
    }

    #[test]
    fn repo_for_key_defaults_to_empty_string() {
        assert_eq!(repo_for_key(&ctx(None, Some(Shape::Entries))), "");
    }

    #[test]
    fn fetch_rejects_invalid_repo_before_auth_lookup() {
        let fetcher = GithubGoodFirstIssues;
        let err = run_async(fetcher.fetch(&ctx(
            Some("repo = \"not-a-slug\"\nlimit = 99\nlabel = \"help wanted\""),
            Some(Shape::Entries),
        )))
        .unwrap_err();
        assert!(matches!(err, FetchError::Failed(msg) if msg.contains("invalid repo option")));
    }

    #[test]
    fn fetch_global_search_surfaces_missing_token_without_network_response() {
        let _guard = EnvGuard::set(&[("GH_TOKEN", None), ("GITHUB_TOKEN", None)]);
        let fetcher = GithubGoodFirstIssues;
        let err = run_async(fetcher.fetch(&ctx(
            Some("limit = 99\nlabel = \"good first issue\""),
            Some(Shape::Entries),
        )))
        .unwrap_err();
        assert!(
            matches!(err, FetchError::Failed(msg) if msg.contains("GH_TOKEN / GITHUB_TOKEN not set"))
        );
    }

    #[test]
    fn fetch_repo_search_surfaces_missing_token_without_network_response() {
        let _guard = EnvGuard::set(&[("GH_TOKEN", None), ("GITHUB_TOKEN", None)]);
        let fetcher = GithubGoodFirstIssues;
        let err = run_async(fetcher.fetch(&ctx(
            Some("repo = \"foo/bar\"\nlimit = 99\nlabel = \"help wanted\""),
            Some(Shape::Entries),
        )))
        .unwrap_err();
        assert!(
            matches!(err, FetchError::Failed(msg) if msg.contains("GH_TOKEN / GITHUB_TOKEN not set"))
        );
    }
}
