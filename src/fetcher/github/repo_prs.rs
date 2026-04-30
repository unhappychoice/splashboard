//! `github_repo_prs` — open pull requests for a specific repo. Repo comes from the `repo`
//! option or (fallback) the git remote of the current directory.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{RepoSlug, cache_key, parse_options, payload, resolve_repo};
use super::items::{IssueItem, render_items};

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
        default: Some("git remote of cwd"),
        description: "Repository to list PRs for. Falls back to the current directory's github remote.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Maximum number of PRs to show.",
    },
];

pub struct GithubRepoPrs;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for GithubRepoPrs {
    fn name(&self) -> &str {
        "github_repo_prs"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Open pull requests for a target repo, sorted by most recently updated. Use `github_my_prs` instead for PRs authored by the authenticated user across all repos."
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
                    "#54 feat(docs): generate widget catalogue",
                    Some("https://github.com/unhappychoice/splashboard/pull/54"),
                ),
                (
                    "#51 feat(fetcher): split clock options",
                    Some("https://github.com/unhappychoice/splashboard/pull/51"),
                ),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "#54 feat(docs): generate widget catalogue",
                "#51 feat(fetcher): split clock options",
            ]),
            Shape::Entries => samples::entries(&[
                ("#54", "feat(docs): widget catalogue"),
                ("#51", "feat(fetcher): split clock options"),
            ]),
            Shape::Timeline => samples::timeline(&[
                (1_774_000_000, "#54", Some("feat(docs): widget catalogue")),
                (
                    1_773_800_000,
                    "#51",
                    Some("feat(fetcher): split clock options"),
                ),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let slug = resolve_repo(opts.repo.as_deref())?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 30);
        let path = format!(
            "/repos/{}/{}/pulls?state=open&sort=updated&direction=desc&per_page={limit}",
            slug.owner, slug.name
        );
        let items: Vec<IssueItem> = rest_get(&path).await?;
        Ok(payload(render_items(
            &items,
            ctx.shape.unwrap_or(Shape::LinkedTextBlock),
            false,
        )))
    }
}

fn repo_for_key(ctx: &FetchContext) -> String {
    ctx.options
        .as_ref()
        .and_then(|v| v.get("repo"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| resolve_repo(None).ok().map(|s: RepoSlug| s.as_path()))
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
            widget_id: "repo-prs".into(),
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
    }

    #[test]
    fn options_deserialize_repo_and_limit() {
        let raw: toml::Value = toml::from_str("repo = \"foo/bar\"\nlimit = 7").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.repo.as_deref(), Some("foo/bar"));
        assert_eq!(opts.limit, Some(7));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("repo = \"foo/bar\"\nbogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn fetcher_metadata_and_samples_cover_supported_shapes() {
        let fetcher = GithubRepoPrs;
        assert_eq!(fetcher.name(), "github_repo_prs");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("target repo"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.option_schemas().len(), 2);
        assert_eq!(fetcher.option_schemas()[0].name, "repo");
        assert_eq!(fetcher.option_schemas()[1].name, "limit");

        let Some(Body::LinkedTextBlock(linked)) = fetcher.sample_body(Shape::LinkedTextBlock)
        else {
            panic!("expected linked text block sample");
        };
        assert_eq!(
            linked.items[0].text,
            "#54 feat(docs): generate widget catalogue"
        );
        assert_eq!(
            linked.items[1].url.as_deref(),
            Some("https://github.com/unhappychoice/splashboard/pull/51")
        );

        let Some(Body::TextBlock(text)) = fetcher.sample_body(Shape::TextBlock) else {
            panic!("expected text block sample");
        };
        assert_eq!(text.lines[1], "#51 feat(fetcher): split clock options");

        let Some(Body::Entries(entries)) = fetcher.sample_body(Shape::Entries) else {
            panic!("expected entries sample");
        };
        assert_eq!(entries.items[0].key, "#54");
        assert_eq!(
            entries.items[1].value.as_deref(),
            Some("feat(fetcher): split clock options")
        );

        let Some(Body::Timeline(timeline)) = fetcher.sample_body(Shape::Timeline) else {
            panic!("expected timeline sample");
        };
        assert_eq!(timeline.events[0].title, "#54");
        assert_eq!(
            timeline.events[1].detail.as_deref(),
            Some("feat(fetcher): split clock options")
        );
        assert!(fetcher.sample_body(Shape::Text).is_none());
    }

    #[test]
    fn cache_key_changes_with_repo_option() {
        let fetcher = GithubRepoPrs;
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
    fn repo_for_key_falls_back_to_cwd_remote_when_available() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let expected = resolve_repo(None).unwrap().as_path();
        assert_eq!(repo_for_key(&ctx(None, Some(Shape::Entries))), expected);
    }

    #[test]
    fn fetch_rejects_invalid_repo_before_auth_lookup() {
        let fetcher = GithubRepoPrs;
        let err = run_async(fetcher.fetch(&ctx(
            Some("repo = \"not-a-slug\"\nlimit = 99"),
            Some(Shape::Entries),
        )))
        .unwrap_err();
        assert!(matches!(err, FetchError::Failed(msg) if msg.contains("invalid repo option")));
    }

    #[test]
    fn fetch_surfaces_missing_token_without_network_response() {
        let _guard = EnvGuard::set(&[("GH_TOKEN", None), ("GITHUB_TOKEN", None)]);
        let fetcher = GithubRepoPrs;
        let err = run_async(fetcher.fetch(&ctx(
            Some("repo = \"foo/bar\"\nlimit = 99"),
            Some(Shape::Entries),
        )))
        .unwrap_err();
        assert!(
            matches!(err, FetchError::Failed(msg) if msg.contains("GH_TOKEN / GITHUB_TOKEN not set"))
        );
    }
}
