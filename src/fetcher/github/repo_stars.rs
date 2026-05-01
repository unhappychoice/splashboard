//! `github_repo_stars` — stargazer count (plus forks / watchers / open issues as `Entries`).
//! Uses `/repos/{o}/{r}`.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, EntriesData, Entry, Payload, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{RepoSlug, cache_key, parse_options, payload, resolve_repo};

const SHAPES: &[Shape] = &[Shape::Text, Shape::Entries];

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
            Shape::Text => samples::text("★ 142"),
            Shape::Entries => samples::entries(&[
                ("stars", "142"),
                ("forks", "9"),
                ("watchers", "12"),
                ("open_issues", "7"),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let slug = resolve_repo(opts.repo.as_deref())?;
        let path = format!("/repos/{}/{}", slug.owner, slug.name);
        let repo: RepoInfo = rest_get(&path).await?;
        let body = match ctx.shape.unwrap_or(Shape::Text) {
            Shape::Entries => Body::Entries(EntriesData {
                items: vec![
                    entry("stars", &repo.stargazers_count.to_string()),
                    entry("forks", &repo.forks_count.to_string()),
                    entry("watchers", &repo.subscribers_count.to_string()),
                    entry("open_issues", &repo.open_issues_count.to_string()),
                ],
            }),
            _ => Body::Text(TextData {
                value: format!("★ {}", repo.stargazers_count),
            }),
        };
        Ok(payload(body))
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
        assert_eq!(fetcher.shapes(), &[Shape::Text, Shape::Entries]);
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
