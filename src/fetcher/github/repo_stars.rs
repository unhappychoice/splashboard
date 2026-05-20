//! `github_repo_stars` — stargazer count (plus forks / watchers / open issues as `Entries`).
//! Uses `/repos/{o}/{r}`.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, Status,
    TextBlockData, TextData,
};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{RepoSlug, cache_key, parse_options, payload, resolve_repo};

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::Badge,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "repo",
    type_hint: "\"owner/name\"",
    required: false,
    default: Some("git remote of cwd"),
    description: "Repository to query. Falls back to the current directory's github remote.",
}];

pub struct GithubRepoStars;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub repo: Option<String>,
}

#[async_trait]
impl Fetcher for GithubRepoStars {
    fn name(&self) -> &str {
        "github_repo_stars"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Social counters for a repo: stargazers, forks, watchers, and open-issue count. Use `github_repo` instead for the identity fields (slug, description, license)."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60
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
        if !SHAPES.contains(&shape) {
            return None;
        }
        let info = RepoInfo {
            stargazers_count: 142,
            forks_count: 9,
            subscribers_count: 12,
            open_issues_count: 7,
        };
        Some(render_body(&info, shape))
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let slug = resolve_repo(opts.repo.as_deref())?;
        let path = format!("/repos/{}/{}", slug.owner, slug.name);
        let repo: RepoInfo = rest_get(&path).await?;
        Ok(payload(render_body(
            &repo,
            ctx.shape.unwrap_or(Shape::Text),
        )))
    }
}

fn render_body(info: &RepoInfo, shape: Shape) -> Body {
    match shape {
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: vec![
                format!("★ {}", info.stargazers_count),
                format!("🍴 {}", info.forks_count),
                format!("👁 {}", info.subscribers_count),
                format!("🔓 {}", info.open_issues_count),
            ],
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: format!(
                "- **★ stars** {}\n- **🍴 forks** {}\n- **👁 watchers** {}\n- **🔓 open issues** {}",
                info.stargazers_count,
                info.forks_count,
                info.subscribers_count,
                info.open_issues_count
            ),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: vec![
                entry("stars", &info.stargazers_count.to_string()),
                entry("forks", &info.forks_count.to_string()),
                entry("watchers", &info.subscribers_count.to_string()),
                entry("open_issues", &info.open_issues_count.to_string()),
            ],
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: vec![
                Bar {
                    label: "stars".into(),
                    value: info.stargazers_count,
                    value_label: None,
                },
                Bar {
                    label: "forks".into(),
                    value: info.forks_count,
                    value_label: None,
                },
                Bar {
                    label: "watchers".into(),
                    value: info.subscribers_count,
                    value_label: None,
                },
                Bar {
                    label: "open_issues".into(),
                    value: info.open_issues_count,
                    value_label: None,
                },
            ],
        }),
        Shape::Badge => Body::Badge(BadgeData {
            status: Status::Ok,
            label: format!("★ {}", info.stargazers_count),
        }),
        _ => Body::Text(TextData {
            value: format!("★ {}", info.stargazers_count),
        }),
    }
}

#[derive(Debug, Deserialize)]
struct RepoInfo {
    #[serde(default)]
    stargazers_count: u64,
    #[serde(default)]
    forks_count: u64,
    #[serde(default)]
    subscribers_count: u64,
    #[serde(default)]
    open_issues_count: u64,
}

fn entry(key: &str, value: &str) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value.into()),
        status: None,
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

    #[test]
    fn options_deserialize_repo_and_reject_unknown_keys() {
        let opts: Options = toml::from_str("repo = \"foo/bar\"").unwrap();
        assert_eq!(opts.repo.as_deref(), Some("foo/bar"));
        assert!(toml::from_str::<Options>("extra = 1").is_err());
    }

    #[test]
    fn fetcher_metadata_cache_key_and_samples_match_contract() {
        let fetcher = GithubRepoStars;
        let left = fetcher.cache_key(&FetchContext {
            options: Some(toml::from_str("repo = \"foo/bar\"").unwrap()),
            timeout: Duration::from_secs(1),
            ..Default::default()
        });
        let right = fetcher.cache_key(&FetchContext {
            options: Some(toml::from_str("repo = \"foo/baz\"").unwrap()),
            timeout: Duration::from_secs(1),
            ..Default::default()
        });

        assert_eq!(fetcher.name(), "github_repo_stars");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("watchers"));
        assert_eq!(
            fetcher.shapes(),
            &[
                Shape::Text,
                Shape::TextBlock,
                Shape::MarkdownTextBlock,
                Shape::Entries,
                Shape::Bars,
                Shape::Badge,
            ]
        );
        assert_eq!(fetcher.default_shape(), Shape::Text);
        assert_eq!(fetcher.option_schemas().len(), 1);
        assert_eq!(fetcher.option_schemas()[0].name, "repo");
        assert_eq!(fetcher.option_schemas()[0].type_hint, "\"owner/name\"");
        assert!(!fetcher.option_schemas()[0].required);
        assert_eq!(
            fetcher.option_schemas()[0].default,
            Some("git remote of cwd")
        );
        assert_ne!(left, right);
        assert!(left.starts_with("github_repo_stars-"));

        let Some(Body::Text(text)) = fetcher.sample_body(Shape::Text) else {
            panic!("expected text sample");
        };
        assert_eq!(text.value, "★ 142");

        let Some(Body::Entries(entries)) = fetcher.sample_body(Shape::Entries) else {
            panic!("expected entries sample");
        };
        assert_eq!(entries.items.len(), 4);
        assert_eq!(entries.items[0].key, "stars");
        assert_eq!(entries.items[3].key, "open_issues");

        let Some(Body::TextBlock(t)) = fetcher.sample_body(Shape::TextBlock) else {
            panic!("expected text block sample");
        };
        assert_eq!(t.lines[0], "★ 142");

        let Some(Body::MarkdownTextBlock(m)) = fetcher.sample_body(Shape::MarkdownTextBlock) else {
            panic!("expected markdown sample");
        };
        assert!(m.value.contains("- **★ stars** 142"));

        let Some(Body::Bars(b)) = fetcher.sample_body(Shape::Bars) else {
            panic!("expected bars sample");
        };
        assert_eq!(b.bars[0].value, 142);

        let Some(Body::Badge(bd)) = fetcher.sample_body(Shape::Badge) else {
            panic!("expected badge sample");
        };
        assert_eq!(bd.status, crate::payload::Status::Ok);
        assert_eq!(bd.label, "★ 142");

        assert!(fetcher.sample_body(Shape::Timeline).is_none());
    }

    #[test]
    fn repo_info_and_entry_helpers_preserve_counts_and_keys() {
        let info: RepoInfo = serde_json::from_str(r#"{"forks_count":9}"#).unwrap();
        assert_eq!(info.stargazers_count, 0);
        assert_eq!(info.forks_count, 9);
        assert_eq!(info.subscribers_count, 0);
        assert_eq!(info.open_issues_count, 0);

        let row = entry("stars", "142");
        assert_eq!(row.key, "stars");
        assert_eq!(row.value.as_deref(), Some("142"));
        assert!(row.status.is_none());
    }

    #[test]
    fn repo_for_key_prefers_explicit_repo_and_falls_back_to_cwd_remote() {
        let explicit = FetchContext {
            options: Some(toml::from_str("repo = \"foo/bar\"").unwrap()),
            ..Default::default()
        };
        assert_eq!(repo_for_key(&explicit), "foo/bar");
        assert_eq!(
            repo_for_key(&FetchContext::default()),
            resolve_repo(None).unwrap().as_path()
        );
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_options_before_repo_resolution() {
        let err = GithubRepoStars
            .fetch(&FetchContext {
                options: Some(toml::from_str("extra = 1").unwrap()),
                timeout: Duration::from_secs(1),
                ..Default::default()
            })
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            FetchError::Failed(message) if message.contains("invalid options")
        ));
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_repo_option_before_network_request() {
        let err = GithubRepoStars
            .fetch(&FetchContext {
                options: Some(toml::from_str("repo = \"nope\"").unwrap()),
                timeout: Duration::from_secs(1),
                ..Default::default()
            })
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            FetchError::Failed(message) if message.contains("invalid repo option")
        ));
    }

    #[tokio::test]
    async fn fetch_without_token_returns_auth_error_after_repo_resolution() {
        let _guard = EnvGuard::set(&[("GH_TOKEN", None), ("GITHUB_TOKEN", None)]);
        let err = GithubRepoStars
            .fetch(&FetchContext {
                timeout: Duration::from_secs(1),
                shape: Some(Shape::Entries),
                ..Default::default()
            })
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            FetchError::Failed(message) if message.contains("GH_TOKEN / GITHUB_TOKEN not set")
        ));
    }
}
