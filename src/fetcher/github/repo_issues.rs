//! `github_repo_issues` — open issues for a specific repo. `/repos/{o}/{r}/issues` returns PRs
//! too; we filter them out client-side (the `pull_request` marker is the canonical signal).

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
        description: "Repository to list issues for. Falls back to the current directory's github remote.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Maximum number of issues to show.",
    },
];

pub struct GithubRepoIssues;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for GithubRepoIssues {
    fn name(&self) -> &str {
        "github_repo_issues"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Open issues for a target repo, sorted by most recently updated, with pull requests filtered out client-side. Use `github_assigned_issues` for the personal queue across all repos."
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
                    "#41 meta: widget catalog & roadmap",
                    Some("https://github.com/unhappychoice/splashboard/issues/41"),
                ),
                (
                    "#17 theme system",
                    Some("https://github.com/unhappychoice/splashboard/issues/17"),
                ),
            ]),
            Shape::TextBlock => {
                samples::text_block(&["#41 meta: widget catalog & roadmap", "#17 theme system"])
            }
            Shape::Entries => {
                samples::entries(&[("#41", "meta: widget catalog"), ("#17", "theme system")])
            }
            Shape::Timeline => samples::timeline(&[
                (1_774_000_000, "#41", Some("meta: widget catalog")),
                (1_773_500_000, "#17", Some("theme system")),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let slug = resolve_repo(opts.repo.as_deref())?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 30);
        // Over-fetch to compensate for pulls mixed into the response — the endpoint has no
        // server-side "issues only" flag. Cap at 100 (max per_page) so the fix-up never has to
        // paginate twice for a reasonable `limit`.
        let fetch_size = (limit * 2).min(100);
        let path = format!(
            "/repos/{}/{}/issues?state=open&sort=updated&direction=desc&per_page={fetch_size}",
            slug.owner, slug.name
        );
        let items: Vec<RepoIssueItem> = rest_get(&path).await?;
        let issues: Vec<IssueItem> = items
            .into_iter()
            .filter(|i| i.pull_request.is_none())
            .take(limit as usize)
            .map(|i| i.inner)
            .collect();
        Ok(payload(render_items(
            &issues,
            ctx.shape.unwrap_or(Shape::LinkedTextBlock),
            false,
        )))
    }
}

#[derive(Debug, Deserialize)]
struct RepoIssueItem {
    #[serde(default)]
    pull_request: Option<serde_json::Value>,
    #[serde(flatten)]
    inner: IssueItem,
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
            widget_id: "repo-issues".into(),
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
        let fetcher = GithubRepoIssues;
        assert_eq!(fetcher.name(), "github_repo_issues");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("pull requests filtered out"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.option_schemas().len(), 2);
        assert_eq!(fetcher.option_schemas()[0].name, "repo");
        assert_eq!(fetcher.option_schemas()[1].name, "limit");

        let Some(Body::LinkedTextBlock(linked)) = fetcher.sample_body(Shape::LinkedTextBlock)
        else {
            panic!("expected linked text block sample");
        };
        assert_eq!(linked.items[0].text, "#41 meta: widget catalog & roadmap");
        assert_eq!(
            linked.items[1].url.as_deref(),
            Some("https://github.com/unhappychoice/splashboard/issues/17")
        );

        let Some(Body::TextBlock(text)) = fetcher.sample_body(Shape::TextBlock) else {
            panic!("expected text block sample");
        };
        assert_eq!(text.lines[1], "#17 theme system");

        let Some(Body::Entries(entries)) = fetcher.sample_body(Shape::Entries) else {
            panic!("expected entries sample");
        };
        assert_eq!(entries.items[0].key, "#41");
        assert_eq!(entries.items[1].value.as_deref(), Some("theme system"));

        let Some(Body::Timeline(timeline)) = fetcher.sample_body(Shape::Timeline) else {
            panic!("expected timeline sample");
        };
        assert_eq!(timeline.events[0].title, "#41");
        assert_eq!(timeline.events[1].detail.as_deref(), Some("theme system"));
        assert!(fetcher.sample_body(Shape::Text).is_none());
    }

    #[test]
    fn repo_issue_items_filter_out_pull_requests_before_rendering() {
        let raw = r#"[
            {
                "title": "Keep real issues",
                "number": 41,
                "updated_at": "2026-04-22T10:15:30Z",
                "html_url": "https://github.com/foo/bar/issues/41",
                "state": "open"
            },
            {
                "title": "Drop pull requests",
                "number": 42,
                "updated_at": "2026-04-22T11:15:30Z",
                "html_url": "https://github.com/foo/bar/pull/42",
                "state": "open",
                "pull_request": {}
            }
        ]"#;
        let items: Vec<RepoIssueItem> = serde_json::from_str(raw).unwrap();
        let issues: Vec<IssueItem> = items
            .into_iter()
            .filter(|item| item.pull_request.is_none())
            .map(|item| item.inner)
            .collect();

        let Body::TextBlock(text) = render_items(&issues, Shape::TextBlock, false) else {
            panic!("expected text block");
        };
        assert_eq!(text.lines, vec!["#41 Keep real issues"]);
    }

    #[test]
    fn cache_key_changes_with_repo_option() {
        let fetcher = GithubRepoIssues;
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
        let fetcher = GithubRepoIssues;
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
        let fetcher = GithubRepoIssues;
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
