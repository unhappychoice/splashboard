//! `github_contributors_monthly` — who's shipping this month, ranked by commit count. Uses the
//! stats endpoint, which returns 202 while GitHub computes the aggregate; we return `Err` so
//! the runtime surfaces a transient warning placeholder and re-checks on the next refresh
//! rather than blocking the render.

use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData,
    MarkdownTextBlockData, Payload, TextBlockData, TextData,
};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::{http, resolve_token};
use super::common::{RepoSlug, cache_key, parse_options, payload, resolve_repo};

const SHAPES: &[Shape] = &[
    Shape::Bars,
    Shape::LinkedTextBlock,
    Shape::Entries,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Text,
];
const DEFAULT_LIMIT: u32 = 10;

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
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Maximum number of contributors to show.",
    },
    OptionSchema {
        name: "days",
        type_hint: "integer (7..=365)",
        required: false,
        default: Some("30"),
        description: "Window size (in days) the monthly total is summed over.",
    },
];

pub struct GithubContributorsMonthly;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub days: Option<u32>,
}

#[async_trait]
impl Fetcher for GithubContributorsMonthly {
    fn name(&self) -> &str {
        "github_contributors_monthly"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Top contributors to a repo over the last N days, ranked by commit count. Bars / Entries / LinkedTextBlock / TextBlock / MarkdownTextBlock all carry the ranking; Text collapses to a `\"@alice +42 / @bob +27 / …\"` headline."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Bars
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
            Shape::Bars => samples::bars(&[("alice", 42), ("bob", 27), ("charlie", 11)]),
            Shape::Entries => {
                samples::entries(&[("alice", "42"), ("bob", "27"), ("charlie", "11")])
            }
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                ("alice  42", Some("https://github.com/alice")),
                ("bob  27", Some("https://github.com/bob")),
                ("charlie  11", Some("https://github.com/charlie")),
            ]),
            Shape::TextBlock => samples::text_block(&["alice  42", "bob  27", "charlie  11"]),
            Shape::MarkdownTextBlock => {
                samples::markdown("1. **@alice** — 42\n2. **@bob** — 27\n3. **@charlie** — 11")
            }
            Shape::Text => samples::text("@alice +42 · @bob +27 · @charlie +11"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let slug = resolve_repo(opts.repo.as_deref())?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 30) as usize;
        let days = opts.days.unwrap_or(30).clamp(7, 365) as i64;
        match fetch_stats(&slug).await? {
            StatsOutcome::Computing => Err(FetchError::Failed(
                "github: stats warming up (refresh in a minute)".into(),
            )),
            StatsOutcome::Ready(stats) => {
                let rows = top_contributors(&stats, days, limit);
                Ok(payload(render_body(
                    &rows,
                    ctx.shape.unwrap_or(Shape::Bars),
                )))
            }
        }
    }
}

enum StatsOutcome {
    Computing,
    Ready(Vec<Contributor>),
}

#[derive(Debug, Deserialize)]
struct Contributor {
    author: Option<Author>,
    #[serde(default)]
    weeks: Vec<Week>,
}

#[derive(Debug, Deserialize)]
struct Author {
    login: String,
}

#[derive(Debug, Deserialize)]
struct Week {
    /// Unix seconds marking the start of the week (Sunday).
    w: i64,
    /// Commits during the week.
    #[serde(default)]
    c: u64,
}

/// `/stats/contributors` responds with 202 the first time while GitHub computes the aggregate.
/// Returning `StatsOutcome::Computing` lets the caller render a warming-up placeholder instead
/// of blocking — the next refresh picks up the ready data.
async fn fetch_stats(slug: &RepoSlug) -> Result<StatsOutcome, FetchError> {
    let token = resolve_token()?;
    let url = format!(
        "https://api.github.com/repos/{}/{}/stats/contributors",
        slug.owner, slug.name
    );
    let res = http()
        .get(&url)
        .bearer_auth(&token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("github stats: {e}")))?;
    let status = res.status();
    if status.as_u16() == 202 {
        return Ok(StatsOutcome::Computing);
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("github stats body: {e}")))?;
    if !status.is_success() {
        return Err(FetchError::Failed(format!("github stats {status}")));
    }
    let list: Vec<Contributor> = serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("github stats parse: {e}")))?;
    Ok(StatsOutcome::Ready(list))
}

fn top_contributors(stats: &[Contributor], days: i64, limit: usize) -> Vec<(String, u64)> {
    let cutoff = Utc::now().timestamp() - Duration::days(days).num_seconds();
    let mut rows: Vec<(String, u64)> = stats
        .iter()
        .filter_map(|c| {
            let login = c.author.as_ref()?.login.clone();
            let commits: u64 = c.weeks.iter().filter(|w| w.w >= cutoff).map(|w| w.c).sum();
            (commits > 0).then_some((login, commits))
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1));
    rows.truncate(limit);
    rows
}

fn render_body(rows: &[(String, u64)], shape: Shape) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: rows
                .iter()
                .map(|(n, c)| Entry {
                    key: n.clone(),
                    value: Some(c.to_string()),
                    status: None,
                })
                .collect(),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: rows
                .iter()
                .map(|(n, c)| LinkedLine {
                    text: format!("{n}  {c}"),
                    url: Some(format!("https://github.com/{n}")),
                })
                .collect(),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: rows.iter().map(|(n, c)| format!("{n}  {c}")).collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: rows
                .iter()
                .enumerate()
                .map(|(i, (n, c))| format!("{}. **@{n}** — {c}", i + 1))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Text => Body::Text(TextData {
            value: rows
                .iter()
                .map(|(n, c)| format!("@{n} +{c}"))
                .collect::<Vec<_>>()
                .join(" · "),
        }),
        _ => Body::Bars(BarsData {
            bars: rows
                .iter()
                .map(|(n, c)| Bar {
                    label: n.clone(),
                    value: *c,
                })
                .collect(),
        }),
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
    use std::time::Duration as StdDuration;

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
                    let prev = std::env::var(key).ok();
                    match value {
                        Some(value) => unsafe { std::env::set_var(key, value) },
                        None => unsafe { std::env::remove_var(key) },
                    }
                    (*key, prev)
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
            widget_id: "contributors-monthly".into(),
            format: Some("compact".into()),
            timeout: StdDuration::from_secs(1),
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

    fn rows() -> Vec<(String, u64)> {
        vec![("alice".into(), 42), ("bob".into(), 27)]
    }

    fn contributor(login: Option<&str>, weeks: &[(i64, u64)]) -> Contributor {
        Contributor {
            author: login.map(|login| Author {
                login: login.into(),
            }),
            weeks: weeks.iter().map(|(w, c)| Week { w: *w, c: *c }).collect(),
        }
    }

    #[test]
    fn options_default_to_none() {
        let opts = Options::default();
        assert!(opts.repo.is_none());
        assert!(opts.limit.is_none());
        assert!(opts.days.is_none());
    }

    #[test]
    fn options_deserialize_repo_limit_and_days() {
        let raw: toml::Value = toml::from_str("repo = \"foo/bar\"\nlimit = 7\ndays = 14").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.repo.as_deref(), Some("foo/bar"));
        assert_eq!(opts.limit, Some(7));
        assert_eq!(opts.days, Some(14));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("repo = \"foo/bar\"\nbogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn fetcher_metadata_and_samples_cover_supported_shapes() {
        let fetcher = GithubContributorsMonthly;
        assert_eq!(fetcher.name(), "github_contributors_monthly");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("Top contributors"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::Bars);
        assert_eq!(fetcher.option_schemas().len(), 3);
        assert_eq!(fetcher.option_schemas()[0].name, "repo");
        assert_eq!(fetcher.option_schemas()[1].name, "limit");
        assert_eq!(fetcher.option_schemas()[2].name, "days");

        let Some(Body::Bars(bars)) = fetcher.sample_body(Shape::Bars) else {
            panic!("expected bars sample");
        };
        assert_eq!(bars.bars[0].label, "alice");
        assert_eq!(bars.bars[1].value, 27);

        let Some(Body::Entries(entries)) = fetcher.sample_body(Shape::Entries) else {
            panic!("expected entries sample");
        };
        assert_eq!(entries.items[0].key, "alice");
        assert_eq!(entries.items[2].value.as_deref(), Some("11"));

        let Some(Body::LinkedTextBlock(linked)) = fetcher.sample_body(Shape::LinkedTextBlock)
        else {
            panic!("expected linked text block sample");
        };
        assert_eq!(linked.items[0].text, "alice  42");
        assert_eq!(
            linked.items[2].url.as_deref(),
            Some("https://github.com/charlie")
        );

        let Some(Body::TextBlock(text)) = fetcher.sample_body(Shape::TextBlock) else {
            panic!("expected text block sample");
        };
        assert_eq!(text.lines[1], "bob  27");

        let Some(Body::MarkdownTextBlock(markdown)) = fetcher.sample_body(Shape::MarkdownTextBlock)
        else {
            panic!("expected markdown sample");
        };
        assert!(markdown.value.contains("1. **@alice** — 42"));

        let Some(Body::Text(text)) = fetcher.sample_body(Shape::Text) else {
            panic!("expected text sample");
        };
        assert_eq!(text.value, "@alice +42 · @bob +27 · @charlie +11");
        assert!(fetcher.sample_body(Shape::Timeline).is_none());
    }

    #[test]
    fn text_body_collapses_to_at_handles_with_plus_counts() {
        let body = render_body(&rows(), Shape::Text);
        let Body::Text(d) = body else {
            panic!("expected text")
        };
        assert_eq!(d.value, "@alice +42 · @bob +27");
    }

    #[test]
    fn markdown_text_block_lists_handles_and_counts() {
        let body = render_body(&rows(), Shape::MarkdownTextBlock);
        let Body::MarkdownTextBlock(d) = body else {
            panic!("expected markdown")
        };
        assert!(d.value.contains("1. **@alice** — 42"));
        assert!(d.value.contains("2. **@bob** — 27"));
    }

    #[test]
    fn text_block_has_one_line_per_contributor() {
        let body = render_body(&rows(), Shape::TextBlock);
        let Body::TextBlock(d) = body else {
            panic!("expected text block")
        };
        assert_eq!(d.lines.len(), 2);
    }

    #[test]
    fn entries_body_keeps_name_to_count_pairs() {
        let body = render_body(&rows(), Shape::Entries);
        let Body::Entries(entries) = body else {
            panic!("expected entries")
        };
        assert_eq!(entries.items[0].key, "alice");
        assert_eq!(entries.items[1].value.as_deref(), Some("27"));
    }

    #[test]
    fn linked_text_block_uses_profile_urls() {
        let body = render_body(&rows(), Shape::LinkedTextBlock);
        let Body::LinkedTextBlock(linked) = body else {
            panic!("expected linked text block")
        };
        assert_eq!(linked.items[0].text, "alice  42");
        assert_eq!(
            linked.items[1].url.as_deref(),
            Some("https://github.com/bob")
        );
    }

    #[test]
    fn bars_body_preserves_labels_and_values() {
        let body = render_body(&rows(), Shape::Bars);
        let Body::Bars(bars) = body else {
            panic!("expected bars")
        };
        assert_eq!(bars.bars[0].label, "alice");
        assert_eq!(bars.bars[1].value, 27);
    }

    #[test]
    fn top_contributors_filters_old_zero_and_anonymous_rows_then_sorts_and_truncates() {
        let now = Utc::now().timestamp();
        let stats = vec![
            contributor(
                Some("alice"),
                &[
                    (now - Duration::days(2).num_seconds(), 4),
                    (now - Duration::days(10).num_seconds(), 6),
                ],
            ),
            contributor(
                Some("bob"),
                &[
                    (now - Duration::days(5).num_seconds(), 7),
                    (now - Duration::days(40).num_seconds(), 20),
                ],
            ),
            contributor(Some("carol"), &[(now - Duration::days(1).num_seconds(), 0)]),
            contributor(None, &[(now - Duration::days(1).num_seconds(), 9)]),
            contributor(
                Some("dave"),
                &[(now - Duration::days(60).num_seconds(), 11)],
            ),
        ];

        let rows = top_contributors(&stats, 30, 2);

        assert_eq!(rows, vec![("alice".into(), 10), ("bob".into(), 7)]);
    }

    #[test]
    fn repo_for_key_prefers_explicit_repo_option() {
        assert_eq!(
            repo_for_key(&ctx(
                Some("repo = \"foo/bar\"\nlimit = 7\ndays = 14"),
                Some(Shape::Bars)
            )),
            "foo/bar"
        );
    }

    #[test]
    fn repo_for_key_falls_back_to_resolved_repo() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            repo_for_key(&ctx(None, Some(Shape::Bars))),
            resolve_repo(None).unwrap().as_path()
        );
    }

    #[test]
    fn cache_key_changes_with_repo_option() {
        let fetcher = GithubContributorsMonthly;
        let a = fetcher.cache_key(&ctx(Some("repo = \"foo/bar\""), Some(Shape::Bars)));
        let b = fetcher.cache_key(&ctx(Some("repo = \"foo/baz\""), Some(Shape::Bars)));
        assert_ne!(a, b);
    }

    #[test]
    fn fetch_rejects_invalid_repo_before_auth_lookup() {
        let fetcher = GithubContributorsMonthly;
        let err = run_async(fetcher.fetch(&ctx(
            Some("repo = \"not-a-slug\"\nlimit = 99\ndays = 999"),
            Some(Shape::Entries),
        )))
        .unwrap_err();
        assert!(matches!(err, FetchError::Failed(msg) if msg.contains("invalid repo option")));
    }

    #[test]
    fn fetch_surfaces_missing_token_without_network_response() {
        let _guard = EnvGuard::set(&[("GH_TOKEN", None), ("GITHUB_TOKEN", None)]);
        let fetcher = GithubContributorsMonthly;
        let err = run_async(fetcher.fetch(&ctx(
            Some("repo = \"foo/bar\"\nlimit = 99\ndays = 999"),
            Some(Shape::Entries),
        )))
        .unwrap_err();
        assert!(
            matches!(err, FetchError::Failed(msg) if msg.contains("GH_TOKEN / GITHUB_TOKEN not set"))
        );
    }
}
