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
                },
                Bar {
                    label: "forks".into(),
                    value: info.forks_count,
                },
                Bar {
                    label: "open_issues".into(),
                    value: info.open_issues_count,
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
}
