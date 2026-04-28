//! `github_recent_releases` — latest releases for a repo.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData, Payload, TextBlockData,
    TimelineData, TimelineEvent,
};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{RepoSlug, cache_key, parse_options, parse_timestamp, payload, resolve_repo};

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::TextBlock,
    Shape::Entries,
    Shape::Timeline,
];
const DEFAULT_LIMIT: u32 = 5;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "repo",
        type_hint: "\"owner/name\"",
        required: false,
        default: Some("git remote of cwd"),
        description: "Repository to query.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=20)",
        required: false,
        default: Some("5"),
        description: "Maximum number of releases to show.",
    },
];

pub struct GithubRecentReleases;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for GithubRecentReleases {
    fn name(&self) -> &str {
        "github_recent_releases"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Latest published releases for a repo with their tags and dates. Good for the cd-into-project view of what shipped recently."
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
                    "v0.3.0  2026-04-10",
                    Some("https://github.com/unhappychoice/splashboard/releases/tag/v0.3.0"),
                ),
                (
                    "v0.2.1  2026-03-05",
                    Some("https://github.com/unhappychoice/splashboard/releases/tag/v0.2.1"),
                ),
            ]),
            Shape::TextBlock => samples::text_block(&["v0.3.0  2026-04-10", "v0.2.1  2026-03-05"]),
            Shape::Entries => {
                samples::entries(&[("v0.3.0", "2026-04-10"), ("v0.2.1", "2026-03-05")])
            }
            Shape::Timeline => samples::timeline(&[
                (1_776_000_000, "v0.3.0", Some("latest")),
                (1_773_800_000, "v0.2.1", None),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let slug = resolve_repo(opts.repo.as_deref())?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 20);
        let path = format!(
            "/repos/{}/{}/releases?per_page={limit}",
            slug.owner, slug.name
        );
        let items: Vec<Release> = rest_get(&path).await?;
        Ok(payload(render_body(
            &items,
            ctx.shape.unwrap_or(Shape::LinkedTextBlock),
        )))
    }
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    html_url: String,
}

fn render_body(items: &[Release], shape: Shape) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: items
                .iter()
                .map(|r| Entry {
                    key: r.tag_name.clone(),
                    value: Some(
                        r.published_at
                            .clone()
                            .unwrap_or_default()
                            .split('T')
                            .next()
                            .unwrap_or("")
                            .into(),
                    ),
                    status: None,
                })
                .collect(),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: items
                .iter()
                .map(|r| LinkedLine {
                    text: format!("{}  {}", r.tag_name, release_date(r)),
                    url: link_for(r),
                })
                .collect(),
        }),
        Shape::Timeline => Body::Timeline(TimelineData {
            events: items
                .iter()
                .map(|r| TimelineEvent {
                    timestamp: r.published_at.as_deref().map(parse_timestamp).unwrap_or(0),
                    title: r.tag_name.clone(),
                    detail: r.name.clone(),
                    status: None,
                })
                .collect(),
        }),
        _ => Body::TextBlock(TextBlockData {
            lines: items
                .iter()
                .map(|r| {
                    let date = r
                        .published_at
                        .as_deref()
                        .and_then(|d| d.split('T').next())
                        .unwrap_or("");
                    format!("{}  {date}", r.tag_name)
                })
                .collect(),
        }),
    }
}

fn release_date(r: &Release) -> String {
    r.published_at
        .as_deref()
        .and_then(|d| d.split('T').next())
        .unwrap_or("")
        .to_string()
}

fn link_for(r: &Release) -> Option<String> {
    if r.html_url.is_empty() {
        None
    } else {
        Some(r.html_url.clone())
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

    fn ctx(options: Option<&str>, shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "releases".into(),
            format: Some("compact".into()),
            timeout: Duration::from_secs(1),
            file_format: None,
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            timezone: None,
            locale: None,
        }
    }

    fn releases() -> Vec<Release> {
        vec![
            Release {
                tag_name: "v1.2.3".into(),
                name: Some("latest".into()),
                published_at: Some("2026-04-10T12:34:56Z".into()),
                html_url: "https://github.com/foo/bar/releases/tag/v1.2.3".into(),
            },
            Release {
                tag_name: "v1.2.2".into(),
                name: None,
                published_at: None,
                html_url: String::new(),
            },
        ]
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
        let fetcher = GithubRecentReleases;
        assert_eq!(fetcher.name(), "github_recent_releases");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("Latest published releases"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.option_schemas().len(), 2);
        assert_eq!(fetcher.option_schemas()[0].name, "repo");
        assert_eq!(fetcher.option_schemas()[1].name, "limit");

        let Some(Body::LinkedTextBlock(linked)) = fetcher.sample_body(Shape::LinkedTextBlock)
        else {
            panic!("expected linked text block sample");
        };
        assert_eq!(linked.items.len(), 2);
        assert_eq!(linked.items[0].text, "v0.3.0  2026-04-10");

        let Some(Body::TextBlock(text)) = fetcher.sample_body(Shape::TextBlock) else {
            panic!("expected text block sample");
        };
        assert_eq!(text.lines[1], "v0.2.1  2026-03-05");

        let Some(Body::Entries(entries)) = fetcher.sample_body(Shape::Entries) else {
            panic!("expected entries sample");
        };
        assert_eq!(entries.items[0].key, "v0.3.0");

        let Some(Body::Timeline(timeline)) = fetcher.sample_body(Shape::Timeline) else {
            panic!("expected timeline sample");
        };
        assert_eq!(timeline.events[0].detail.as_deref(), Some("latest"));
        assert!(fetcher.sample_body(Shape::Text).is_none());
    }

    #[test]
    fn entries_body_uses_tag_name_and_iso_date() {
        let body = render_body(&releases(), Shape::Entries);
        let Body::Entries(entries) = body else {
            panic!("expected entries");
        };
        assert_eq!(entries.items[0].key, "v1.2.3");
        assert_eq!(entries.items[0].value.as_deref(), Some("2026-04-10"));
        assert_eq!(entries.items[1].value.as_deref(), Some(""));
    }

    #[test]
    fn linked_text_block_uses_release_date_and_optional_link() {
        let body = render_body(&releases(), Shape::LinkedTextBlock);
        let Body::LinkedTextBlock(linked) = body else {
            panic!("expected linked text block");
        };
        assert_eq!(linked.items[0].text, "v1.2.3  2026-04-10");
        assert_eq!(
            linked.items[0].url.as_deref(),
            Some("https://github.com/foo/bar/releases/tag/v1.2.3")
        );
        assert_eq!(linked.items[1].text, "v1.2.2  ");
        assert!(linked.items[1].url.is_none());
    }

    #[test]
    fn timeline_body_preserves_optional_name_and_missing_timestamp() {
        let body = render_body(&releases(), Shape::Timeline);
        let Body::Timeline(timeline) = body else {
            panic!("expected timeline");
        };
        assert_eq!(
            timeline.events[0].timestamp,
            parse_timestamp("2026-04-10T12:34:56Z")
        );
        assert_eq!(timeline.events[0].detail.as_deref(), Some("latest"));
        assert_eq!(timeline.events[1].timestamp, 0);
        assert!(timeline.events[1].detail.is_none());
    }

    #[test]
    fn text_block_body_falls_back_to_plain_lines() {
        let body = render_body(&releases(), Shape::TextBlock);
        let Body::TextBlock(text) = body else {
            panic!("expected text block");
        };
        assert_eq!(
            text.lines,
            vec!["v1.2.3  2026-04-10".to_string(), "v1.2.2  ".to_string()]
        );
    }

    #[test]
    fn cache_key_changes_with_repo_option() {
        let fetcher = GithubRecentReleases;
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
}
