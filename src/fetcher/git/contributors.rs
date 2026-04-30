use std::collections::HashMap;

use async_trait::async_trait;
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;

use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, TextBlockData,
};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, open_repo, payload, repo_cache_key, text_body};

const SHAPES: &[Shape] = &[
    Shape::Bars,
    Shape::Entries,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Text,
];
const DEFAULT_DAYS: u64 = 30;
const MAX_ENTRIES: usize = 10;

/// Commit authors over the last N days (default 30, configurable via `format = "N"`). Ranked
/// by commit count, truncated to the top 10 so a busy repo doesn't blow up the widget. `Bars`
/// is the default (visual ranking); `Entries` emits `name: count` rows; `TextBlock` lists
/// `"alice  42"` per line; `MarkdownTextBlock` renders the same ranking as a markdown list;
/// `Text` collapses to `"alice (23), bob (11)"` style.
pub struct GitContributors;

#[async_trait]
impl Fetcher for GitContributors {
    fn name(&self) -> &str {
        "git_contributors"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Top commit authors over the last N days (default 30, override with `format = \"N\"`), ranked by commit count and capped at ten. Bars/Entries/TextBlock/MarkdownTextBlock all carry the ranking; Text is the headline summary."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Bars
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        repo_cache_key(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Bars => {
                samples::bars(&[("alice", 42), ("bob", 28), ("charlie", 17), ("dave", 9)])
            }
            Shape::Entries => samples::entries(&[
                ("alice", "42"),
                ("bob", "28"),
                ("charlie", "17"),
                ("dave", "9"),
            ]),
            Shape::TextBlock => {
                samples::text_block(&["alice  42", "bob  28", "charlie  17", "dave  9"])
            }
            Shape::MarkdownTextBlock => samples::markdown(
                "1. **alice** — 42\n2. **bob** — 28\n3. **charlie** — 17\n4. **dave** — 9",
            ),
            Shape::Text => samples::text("alice (42), bob (28), charlie (17), dave (9)"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        let days = parse_days(ctx.format.as_deref());
        let ranked = contributors(&repo, days, ctx.timezone.as_deref())?;
        Ok(payload(render_body(
            ranked,
            ctx.shape.unwrap_or(Shape::Bars),
        )))
    }
}

fn contributors(
    repo: &gix::Repository,
    days: u64,
    timezone: Option<&str>,
) -> Result<Vec<(String, u64)>, FetchError> {
    let Ok(head_id) = repo.head_id() else {
        return Ok(Vec::new());
    };
    let cutoff = crate::time::now_in(timezone).timestamp() - (days as i64) * 86_400;
    let walker = repo
        .rev_walk([head_id.detach()])
        .sorting(Sorting::ByCommitTimeCutoff {
            seconds: cutoff,
            order: CommitTimeOrder::NewestFirst,
        })
        .all()
        .map_err(fail)?;
    let mut counts: HashMap<String, u64> = HashMap::new();
    for info in walker {
        let Ok(info) = info else { continue };
        let Ok(commit) = repo.find_commit(info.id) else {
            continue;
        };
        let Ok(author) = commit.author() else {
            continue;
        };
        let name = author.name.to_string();
        *counts.entry(name).or_insert(0) += 1;
    }
    let mut ranked: Vec<_> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(MAX_ENTRIES);
    Ok(ranked)
}

fn parse_days(format: Option<&str>) -> u64 {
    format
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_DAYS)
}

fn render_body(ranked: Vec<(String, u64)>, shape: Shape) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: ranked
                .into_iter()
                .map(|(name, count)| Entry {
                    key: name,
                    value: Some(count.to_string()),
                    status: None,
                })
                .collect(),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: ranked
                .into_iter()
                .map(|(name, count)| format!("{name}  {count}"))
                .collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: ranked
                .into_iter()
                .enumerate()
                .map(|(i, (name, count))| format!("{}. **{name}** — {count}", i + 1))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Text => {
            if ranked.is_empty() {
                text_body("")
            } else {
                let line = ranked
                    .into_iter()
                    .map(|(name, count)| format!("{name} ({count})"))
                    .collect::<Vec<_>>()
                    .join(", ");
                text_body(line)
            }
        }
        _ => Body::Bars(BarsData {
            bars: ranked
                .into_iter()
                .map(|(name, count)| Bar {
                    label: name,
                    value: count,
                })
                .collect(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;

    use super::super::test_support::{commit_as, make_repo};
    use super::*;

    fn run_async<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn ctx(shape: Option<Shape>, format: Option<&str>) -> FetchContext {
        FetchContext {
            shape,
            format: format.map(str::to_string),
            ..FetchContext::default()
        }
    }

    #[test]
    fn fetcher_contract_and_samples_cover_supported_shapes() {
        let fetcher = GitContributors;
        let text_key = fetcher.cache_key(&ctx(Some(Shape::Text), Some("7")));
        let bars_key = fetcher.cache_key(&ctx(Some(Shape::Bars), Some("30")));

        assert_eq!(fetcher.name(), "git_contributors");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("Top commit authors"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::Bars);
        assert!(text_key.starts_with("git_contributors-"));
        assert_ne!(text_key, bars_key);
        assert_eq!(
            fetcher.sample_body(Shape::Bars),
            Some(samples::bars(&[
                ("alice", 42),
                ("bob", 28),
                ("charlie", 17),
                ("dave", 9),
            ]))
        );
        assert_eq!(
            fetcher.sample_body(Shape::Entries),
            Some(samples::entries(&[
                ("alice", "42"),
                ("bob", "28"),
                ("charlie", "17"),
                ("dave", "9"),
            ]))
        );
        assert_eq!(
            fetcher.sample_body(Shape::TextBlock),
            Some(samples::text_block(&[
                "alice  42",
                "bob  28",
                "charlie  17",
                "dave  9",
            ]))
        );
        assert_eq!(
            fetcher.sample_body(Shape::MarkdownTextBlock),
            Some(samples::markdown(
                "1. **alice** — 42\n2. **bob** — 28\n3. **charlie** — 17\n4. **dave** — 9",
            ))
        );
        assert_eq!(
            fetcher.sample_body(Shape::Text),
            Some(samples::text(
                "alice (42), bob (28), charlie (17), dave (9)"
            ))
        );
        assert!(fetcher.sample_body(Shape::Badge).is_none());
    }

    #[test]
    fn empty_repo_returns_empty() {
        let (_tmp, repo) = make_repo();
        assert!(contributors(&repo, 30, None).unwrap().is_empty());
    }

    #[test]
    fn tallies_authors_in_rank_order() {
        let (_tmp, repo) = make_repo();
        commit_as(&repo, "a1", "alice", "a@example.com");
        commit_as(&repo, "b1", "bob", "b@example.com");
        commit_as(&repo, "a2", "alice", "a@example.com");
        commit_as(&repo, "a3", "alice", "a@example.com");
        let ranked = contributors(&repo, 30, None).unwrap();
        assert_eq!(ranked, vec![("alice".into(), 3u64), ("bob".into(), 1u64)]);
    }

    #[test]
    fn bars_shape_from_ranking() {
        assert_eq!(
            render_body(vec![("alice".into(), 3), ("bob".into(), 1)], Shape::Bars),
            Body::Bars(BarsData {
                bars: vec![
                    Bar {
                        label: "alice".into(),
                        value: 3,
                    },
                    Bar {
                        label: "bob".into(),
                        value: 1,
                    },
                ],
            })
        );
    }

    #[test]
    fn entries_shape_uses_name_count_rows() {
        assert_eq!(
            render_body(vec![("alice".into(), 3), ("bob".into(), 1)], Shape::Entries),
            Body::Entries(EntriesData {
                items: vec![
                    Entry {
                        key: "alice".into(),
                        value: Some("3".into()),
                        status: None,
                    },
                    Entry {
                        key: "bob".into(),
                        value: Some("1".into()),
                        status: None,
                    },
                ],
            })
        );
    }

    #[test]
    fn text_shape_empty_when_empty() {
        assert_eq!(render_body(Vec::new(), Shape::Text), text_body(""));
    }

    #[test]
    fn text_shape_joins_with_counts() {
        assert_eq!(
            render_body(vec![("alice".into(), 3), ("bob".into(), 1)], Shape::Text),
            text_body("alice (3), bob (1)")
        );
    }

    #[test]
    fn text_block_lists_one_row_per_contributor() {
        assert_eq!(
            render_body(
                vec![("alice".into(), 3), ("bob".into(), 1)],
                Shape::TextBlock
            ),
            Body::TextBlock(TextBlockData {
                lines: vec!["alice  3".into(), "bob  1".into()],
            })
        );
    }

    #[test]
    fn markdown_text_block_emits_numbered_ranking() {
        assert_eq!(
            render_body(
                vec![("alice".into(), 3), ("bob".into(), 1)],
                Shape::MarkdownTextBlock,
            ),
            Body::MarkdownTextBlock(MarkdownTextBlockData {
                value: "1. **alice** — 3\n2. **bob** — 1".into(),
            })
        );
    }

    #[test]
    fn ranking_tie_breaks_alphabetically_and_truncates() {
        let (_tmp, repo) = make_repo();
        [
            "zoe", "amy", "luz", "bob", "kai", "eve", "mia", "ian", "ned", "uma", "zzz",
        ]
        .into_iter()
        .for_each(|name| commit_as(&repo, name, name, &format!("{name}@example.com")));

        let ranked = contributors(&repo, 30, None).unwrap();
        let names: Vec<_> = ranked.into_iter().map(|(name, _)| name).collect();

        assert_eq!(names.len(), MAX_ENTRIES);
        assert_eq!(
            names,
            vec![
                "amy", "bob", "eve", "ian", "kai", "luz", "mia", "ned", "uma", "zoe",
            ]
        );
    }

    #[test]
    fn fetch_reads_cwd_repo_for_default_and_requested_shapes() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (_tmp, repo) = make_repo();
        commit_as(&repo, "first", "alice", "a@example.com");
        commit_as(&repo, "second", "bob", "b@example.com");
        let workdir = repo.workdir().unwrap().to_path_buf();
        let prev_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&workdir).unwrap();

        let bars = run_async(GitContributors.fetch(&ctx(None, Some("3650"))));
        let entries = run_async(GitContributors.fetch(&ctx(Some(Shape::Entries), Some("3650"))));

        std::env::set_current_dir(prev_cwd).unwrap();

        assert!(matches!(
            bars.unwrap().body,
            Body::Bars(BarsData { bars }) if bars.len() == 2
        ));
        assert!(matches!(
            entries.unwrap().body,
            Body::Entries(EntriesData { items })
                if items.len() == 2 && items.iter().all(|item| item.value.is_some())
        ));
    }

    #[test]
    fn parse_days_defaults_and_parses() {
        assert_eq!(parse_days(None), DEFAULT_DAYS);
        assert_eq!(parse_days(Some("0")), DEFAULT_DAYS);
        assert_eq!(parse_days(Some(" 14 ")), 14);
        assert_eq!(parse_days(Some("7")), 7);
    }
}
