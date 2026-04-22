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
use super::common::{RepoSlug, cache_key, parse_options, payload, placeholder, resolve_repo};

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
        let opts: Options = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return Ok(placeholder(&msg)),
        };
        let slug = match resolve_repo(opts.repo.as_deref()) {
            Ok(s) => s,
            Err(e) => return Ok(placeholder(&e.to_string())),
        };
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
