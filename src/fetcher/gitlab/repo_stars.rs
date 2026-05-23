//! `gitlab_repo_stars` — stargazer count + forks / open-issues rollup for one GitLab project.
//! GitLab has no separate "watchers" concept, so the `watchers` row that ships with
//! `github_repo_stars` is omitted here.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, Status,
    TextBlockData, TextData,
};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::{default_host, rest_get};
use super::common::{ProjectPath, cache_key, parse_options, payload, resolve_project};
use super::my_mrs::resolve_host;

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::Badge,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "host",
        type_hint: "hostname",
        required: false,
        default: Some("gitlab.com"),
        description: "GitLab instance host. Use this for self-hosted GitLab.",
    },
    OptionSchema {
        name: "project",
        type_hint: "\"group/name\" or \"group/sub/name\"",
        required: false,
        default: Some("git remote of cwd"),
        description: "Project to query. Falls back to the current directory's GitLab remote.",
    },
];

pub struct GitlabRepoStars;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
}

#[async_trait]
impl Fetcher for GitlabRepoStars {
    fn name(&self) -> &str {
        "gitlab_repo_stars"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Social counters for a GitLab project: stargazers, forks, and open-issue count. GitLab has no separate watchers count, so that row is omitted relative to the GitHub sibling."
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
        cache_key(self.name(), ctx, &cache_extra(ctx))
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        if !SHAPES.contains(&shape) {
            return None;
        }
        let info = ProjectInfo {
            star_count: 142,
            forks_count: 9,
            open_issues_count: 7,
        };
        Some(render_body(&info, shape))
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let host = resolve_host(opts.host.as_deref())?;
        let project = resolve_project(opts.project.as_deref(), &host)?;
        let path = format!("/projects/{}", project.encoded());
        let info: ProjectInfo = rest_get(&host, &path).await?;
        let shape = ctx.shape.unwrap_or(Shape::Text);
        Ok(payload(render_body(&info, shape)))
    }
}

#[derive(Debug, Deserialize)]
struct ProjectInfo {
    #[serde(default)]
    star_count: u64,
    #[serde(default)]
    forks_count: u64,
    #[serde(default)]
    open_issues_count: u64,
}

fn render_body(info: &ProjectInfo, shape: Shape) -> Body {
    match shape {
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: vec![
                format!("★ {}", info.star_count),
                format!("🍴 {}", info.forks_count),
                format!("🔓 {}", info.open_issues_count),
            ],
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: format!(
                "- **★ stars** {}\n- **🍴 forks** {}\n- **🔓 open issues** {}",
                info.star_count, info.forks_count, info.open_issues_count
            ),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: vec![
                entry("stars", info.star_count),
                entry("forks", info.forks_count),
                entry("open_issues", info.open_issues_count),
            ],
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: vec![
                Bar {
                    label: "stars".into(),
                    value: info.star_count,
                    value_label: None,
                },
                Bar {
                    label: "forks".into(),
                    value: info.forks_count,
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
            label: format!("★ {}", info.star_count),
        }),
        _ => Body::Text(TextData {
            value: format!("★ {}", info.star_count),
        }),
    }
}

fn entry(key: &str, value: u64) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value.to_string()),
        status: None,
    }
}

fn cache_extra(ctx: &FetchContext) -> String {
    let host = ctx
        .options
        .as_ref()
        .and_then(|v| v.get("host"))
        .and_then(|v| v.as_str())
        .unwrap_or(default_host())
        .to_string();
    let project = ctx
        .options
        .as_ref()
        .and_then(|v| v.get("project"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| {
            resolve_project(None, &host)
                .ok()
                .map(|p: ProjectPath| p.as_str().to_string())
        })
        .unwrap_or_default();
    format!("{host}|{project}")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn options_deserialize_both_fields() {
        let opts: Options =
            toml::from_str("host = \"git.example.org\"\nproject = \"g/p\"").unwrap();
        assert_eq!(opts.host.as_deref(), Some("git.example.org"));
        assert_eq!(opts.project.as_deref(), Some("g/p"));
        assert!(toml::from_str::<Options>("bogus = 1").is_err());
    }

    #[test]
    fn shapes_table_lists_the_six_supported_variants() {
        let fetcher = GitlabRepoStars;
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
    }

    #[test]
    fn sample_text_carries_the_star_count_headline() {
        let Some(Body::Text(t)) = GitlabRepoStars.sample_body(Shape::Text) else {
            panic!("expected text");
        };
        assert_eq!(t.value, "★ 142");
    }

    #[test]
    fn sample_entries_drops_the_watchers_row() {
        let Some(Body::Entries(e)) = GitlabRepoStars.sample_body(Shape::Entries) else {
            panic!("expected entries");
        };
        assert_eq!(e.items.len(), 3);
        let keys: Vec<&str> = e.items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, vec!["stars", "forks", "open_issues"]);
        assert!(!keys.contains(&"watchers"));
    }

    #[test]
    fn sample_bars_value_matches_counts() {
        let Some(Body::Bars(b)) = GitlabRepoStars.sample_body(Shape::Bars) else {
            panic!("expected bars");
        };
        assert_eq!(b.bars[0].value, 142);
        assert_eq!(b.bars[1].value, 9);
        assert_eq!(b.bars[2].value, 7);
    }

    #[test]
    fn sample_badge_is_calm_ok_status_for_pure_info() {
        let Some(Body::Badge(b)) = GitlabRepoStars.sample_body(Shape::Badge) else {
            panic!("expected badge");
        };
        assert_eq!(b.status, Status::Ok);
        assert_eq!(b.label, "★ 142");
    }

    #[test]
    fn sample_markdown_uses_bullet_list_with_bold_labels() {
        let Some(Body::MarkdownTextBlock(m)) =
            GitlabRepoStars.sample_body(Shape::MarkdownTextBlock)
        else {
            panic!("expected markdown");
        };
        assert!(m.value.contains("- **★ stars** 142"));
        assert!(m.value.contains("- **🍴 forks** 9"));
    }

    #[test]
    fn render_body_falls_back_to_text_for_unknown_shapes() {
        let info = ProjectInfo {
            star_count: 5,
            forks_count: 0,
            open_issues_count: 0,
        };
        let Body::Text(t) = render_body(&info, Shape::Ratio) else {
            panic!("expected text fallback");
        };
        assert_eq!(t.value, "★ 5");
    }

    #[test]
    fn sample_text_block_lists_emoji_prefixed_counter_lines() {
        let Some(Body::TextBlock(b)) = GitlabRepoStars.sample_body(Shape::TextBlock) else {
            panic!("expected text block");
        };
        assert_eq!(b.lines, vec!["★ 142", "🍴 9", "🔓 7"]);
    }

    #[test]
    fn sample_body_returns_none_for_unsupported_shape() {
        assert!(GitlabRepoStars.sample_body(Shape::Ratio).is_none());
    }

    #[test]
    fn metadata_methods_have_content() {
        let fetcher = GitlabRepoStars;
        assert_eq!(fetcher.name(), "gitlab_repo_stars");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("GitLab"));
        assert_eq!(fetcher.refresh_interval(), 60 * 60);
    }

    #[test]
    fn option_schemas_lists_host_and_project() {
        let names: Vec<&str> = GitlabRepoStars
            .option_schemas()
            .iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["host", "project"]);
    }

    #[test]
    fn cache_key_is_name_prefixed_and_varies_with_project_option() {
        let fetcher = GitlabRepoStars;
        let a = fetcher.cache_key(&FetchContext {
            options: Some(toml::from_str("project = \"a/b\"").unwrap()),
            ..Default::default()
        });
        let b = fetcher.cache_key(&FetchContext {
            options: Some(toml::from_str("project = \"a/c\"").unwrap()),
            ..Default::default()
        });
        assert!(a.starts_with("gitlab_repo_stars-"));
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_varies_with_host_option() {
        let fetcher = GitlabRepoStars;
        let a = fetcher.cache_key(&FetchContext {
            options: Some(toml::from_str("host = \"git.one.org\"\nproject = \"a/b\"").unwrap()),
            ..Default::default()
        });
        let b = fetcher.cache_key(&FetchContext {
            options: Some(toml::from_str("host = \"git.two.org\"\nproject = \"a/b\"").unwrap()),
            ..Default::default()
        });
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_with_no_options_falls_back_to_remote_resolution() {
        // No `project` option: `cache_extra` walks the cwd git remote, which here is a GitHub
        // repo, so the gitlab.com lookup yields nothing and the key still resolves cleanly.
        let key = GitlabRepoStars.cache_key(&FetchContext::default());
        assert!(key.starts_with("gitlab_repo_stars-"));
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_option_keys_before_network_request() {
        let err = GitlabRepoStars
            .fetch(&FetchContext {
                options: Some(toml::from_str("bogus = 1").unwrap()),
                timeout: Duration::from_secs(1),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(m) if m.contains("invalid options")
        ));
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_host_before_network_request() {
        let err = GitlabRepoStars
            .fetch(&FetchContext {
                options: Some(toml::from_str("host = \"https://evil.example\"").unwrap()),
                timeout: Duration::from_secs(1),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(m) if m.contains("invalid gitlab host")
        ));
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_project_before_network_request() {
        let err = GitlabRepoStars
            .fetch(&FetchContext {
                options: Some(toml::from_str("project = \"not a path\"").unwrap()),
                timeout: Duration::from_secs(1),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(m) if m.contains("invalid project option")
        ));
    }
}
