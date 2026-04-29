use std::collections::HashMap;
use std::convert::Infallible;
use std::ops::ControlFlow;

use async_trait::async_trait;
use gix::object::tree::diff::Change;
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;

use crate::payload::{Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, open_repo, payload, repo_cache_key, text_block_body, text_body};

const SHAPES: &[Shape] = &[
    Shape::Entries,
    Shape::Bars,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Text,
];
const DEFAULT_DAYS: u64 = 30;
const TOP_FILES: usize = 10;
const MAX_COMMITS: usize = 500;

/// Top files by change count over the last N days (default 30, configurable via `format = "N"`).
/// `Entries` is the default (`path: count` rows); `Bars` ranks visually; `TextBlock` emits one
/// `"path (count)"` per line; `MarkdownTextBlock` outputs the same ranking as a numbered markdown
/// list; `Text` collapses everything into a single-line basename summary
/// `"main.rs (42), lib.rs (31), …"`. Pairs with `code_largest_files` for "where's the action vs
/// where's the mass" framing.
pub struct GitChurn;

#[async_trait]
impl Fetcher for GitChurn {
    fn name(&self) -> &str {
        "git_churn"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Top files by change count over the last N days (default 30, override with `format = \"N\"`), capped at ten. Pairs with `code_largest_files` for 'where's the action vs where's the mass' framing."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Entries
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        repo_cache_key(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Entries => samples::entries(&[
                ("src/main.rs", "42"),
                ("src/lib.rs", "31"),
                ("src/render/mod.rs", "18"),
                ("src/fetcher/mod.rs", "12"),
            ]),
            Shape::Bars => samples::bars(&[
                ("src/main.rs", 42),
                ("src/lib.rs", 31),
                ("src/render/mod.rs", 18),
                ("src/fetcher/mod.rs", 12),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "src/main.rs (42)",
                "src/lib.rs (31)",
                "src/render/mod.rs (18)",
                "src/fetcher/mod.rs (12)",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "1. `src/main.rs` — 42\n2. `src/lib.rs` — 31\n3. `src/render/mod.rs` — 18\n4. `src/fetcher/mod.rs` — 12",
            ),
            Shape::Text => samples::text("main.rs (42), lib.rs (31), mod.rs (18)"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        let days = parse_days(ctx.format.as_deref());
        let ranked = churn(&repo, days, ctx.timezone.as_deref())?;
        Ok(payload(render_body(
            ranked,
            ctx.shape.unwrap_or(Shape::Entries),
        )))
    }
}

fn churn(
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

    let empty_tree = repo.empty_tree();
    let mut totals: HashMap<String, u64> = HashMap::new();

    for info in walker.take(MAX_COMMITS) {
        let Ok(info) = info else { continue };
        let Ok(commit) = repo.find_commit(info.id) else {
            continue;
        };
        let Ok(tree) = commit.tree() else { continue };
        let parent_tree = commit
            .parent_ids()
            .next()
            .and_then(|id| repo.find_commit(id.detach()).ok())
            .and_then(|c| c.tree().ok())
            .unwrap_or_else(|| empty_tree.clone());

        let Ok(mut changes) = parent_tree.changes() else {
            continue;
        };
        let _ = changes.for_each_to_obtain_tree(&tree, |change| {
            if let Some(path) = changed_path(&change) {
                *totals.entry(path).or_insert(0) += 1;
            }
            Ok::<_, Infallible>(ControlFlow::Continue(()))
        });
    }

    let mut ranked: Vec<_> = totals.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(TOP_FILES);
    Ok(ranked)
}

fn changed_path(change: &Change<'_, '_, '_>) -> Option<String> {
    let (loc, mode) = match change {
        Change::Addition {
            location,
            entry_mode,
            ..
        }
        | Change::Deletion {
            location,
            entry_mode,
            ..
        }
        | Change::Modification {
            location,
            entry_mode,
            ..
        }
        | Change::Rewrite {
            location,
            entry_mode,
            ..
        } => (*location, entry_mode),
    };
    if loc.is_empty() || mode.is_tree() {
        None
    } else {
        Some(loc.to_string())
    }
}

fn parse_days(format: Option<&str>) -> u64 {
    format
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_DAYS)
}

fn render_body(ranked: Vec<(String, u64)>, shape: Shape) -> Body {
    match shape {
        Shape::Bars => Body::Bars(BarsData {
            bars: ranked
                .into_iter()
                .map(|(path, count)| Bar {
                    label: path,
                    value: count,
                })
                .collect(),
        }),
        Shape::TextBlock => text_block_body(
            ranked
                .into_iter()
                .map(|(path, count)| format!("{path} ({count})"))
                .collect(),
        ),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: ranked
                .into_iter()
                .enumerate()
                .map(|(i, (path, count))| format!("{}. `{path}` — {count}", i + 1))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Text => {
            if ranked.is_empty() {
                text_body("")
            } else {
                let line = ranked
                    .into_iter()
                    .map(|(path, count)| format!("{} ({count})", short_path(&path)))
                    .collect::<Vec<_>>()
                    .join(", ");
                text_body(line)
            }
        }
        _ => Body::Entries(EntriesData {
            items: ranked
                .into_iter()
                .map(|(path, count)| Entry {
                    key: path,
                    value: Some(count.to_string()),
                    status: None,
                })
                .collect(),
        }),
    }
}

fn short_path(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

#[cfg(test)]
mod tests {
    use std::future::Future;

    use super::super::test_support::{commit_touching, make_repo};
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
        let fetcher = GitChurn;
        let default_key = fetcher.cache_key(&ctx(None, None));
        let text_key = fetcher.cache_key(&ctx(Some(Shape::Text), Some("7")));
        let bars_key = fetcher.cache_key(&ctx(Some(Shape::Bars), Some("30")));

        assert_eq!(fetcher.name(), "git_churn");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("Top files by change count"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::Entries);
        assert!(default_key.starts_with("git_churn-"));
        assert_ne!(default_key, text_key);
        assert_ne!(text_key, bars_key);
        assert_eq!(
            fetcher.sample_body(Shape::Entries),
            Some(samples::entries(&[
                ("src/main.rs", "42"),
                ("src/lib.rs", "31"),
                ("src/render/mod.rs", "18"),
                ("src/fetcher/mod.rs", "12"),
            ]))
        );
        assert_eq!(
            fetcher.sample_body(Shape::Bars),
            Some(samples::bars(&[
                ("src/main.rs", 42),
                ("src/lib.rs", 31),
                ("src/render/mod.rs", 18),
                ("src/fetcher/mod.rs", 12),
            ]))
        );
        assert_eq!(
            fetcher.sample_body(Shape::TextBlock),
            Some(samples::text_block(&[
                "src/main.rs (42)",
                "src/lib.rs (31)",
                "src/render/mod.rs (18)",
                "src/fetcher/mod.rs (12)",
            ]))
        );
        assert_eq!(
            fetcher.sample_body(Shape::MarkdownTextBlock),
            Some(samples::markdown(
                "1. `src/main.rs` — 42\n2. `src/lib.rs` — 31\n3. `src/render/mod.rs` — 18\n4. `src/fetcher/mod.rs` — 12",
            ))
        );
        assert_eq!(
            fetcher.sample_body(Shape::Text),
            Some(samples::text("main.rs (42), lib.rs (31), mod.rs (18)"))
        );
        assert!(fetcher.sample_body(Shape::Badge).is_none());
    }

    #[test]
    fn empty_repo_returns_empty() {
        let (_tmp, repo) = make_repo();
        assert!(churn(&repo, 30, None).unwrap().is_empty());
    }

    #[test]
    fn ranks_files_by_commit_count() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "a.rs", "a1");
        commit_touching(&repo, "a.rs", "a2");
        commit_touching(&repo, "a.rs", "a3");
        commit_touching(&repo, "b.rs", "b1");
        let ranked = churn(&repo, 30, None).unwrap();
        assert_eq!(ranked[0], ("a.rs".to_string(), 3));
        assert_eq!(ranked[1], ("b.rs".to_string(), 1));
    }

    #[test]
    fn truncates_to_top_n() {
        let (_tmp, repo) = make_repo();
        for i in 0..(TOP_FILES + 5) {
            commit_touching(&repo, &format!("f{i}.rs"), &format!("c{i}"));
        }
        let ranked = churn(&repo, 30, None).unwrap();
        assert_eq!(ranked.len(), TOP_FILES);
    }

    #[test]
    fn entries_shape_uses_full_path() {
        let body = render_body(vec![("src/foo/bar.rs".into(), 5)], Shape::Entries);
        match body {
            Body::Entries(d) => {
                assert_eq!(d.items[0].key, "src/foo/bar.rs");
                assert_eq!(d.items[0].value.as_deref(), Some("5"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn bars_shape_carries_full_path_and_count() {
        let body = render_body(
            vec![("src/main.rs".into(), 7), ("src/lib.rs".into(), 3)],
            Shape::Bars,
        );
        match body {
            Body::Bars(d) => {
                assert_eq!(d.bars.len(), 2);
                assert_eq!(d.bars[0].label, "src/main.rs");
                assert_eq!(d.bars[0].value, 7);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn text_block_shape_one_line_per_file() {
        let body = render_body(
            vec![("src/foo.rs".into(), 3), ("src/bar.rs".into(), 1)],
            Shape::TextBlock,
        );
        match body {
            Body::TextBlock(d) => {
                assert_eq!(d.lines, vec!["src/foo.rs (3)", "src/bar.rs (1)"]);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn text_block_shape_empty_when_empty() {
        let body = render_body(Vec::new(), Shape::TextBlock);
        match body {
            Body::TextBlock(d) => assert!(d.lines.is_empty()),
            _ => panic!(),
        }
    }

    #[test]
    fn text_shape_shortens_paths_and_joins() {
        let body = render_body(
            vec![("src/foo.rs".into(), 3), ("src/dir/bar.rs".into(), 1)],
            Shape::Text,
        );
        match body {
            Body::Text(d) => assert_eq!(d.value, "foo.rs (3), bar.rs (1)"),
            _ => panic!(),
        }
    }

    #[test]
    fn text_shape_empty_when_empty() {
        let body = render_body(Vec::new(), Shape::Text);
        match body {
            Body::Text(d) => assert!(d.value.is_empty()),
            _ => panic!(),
        }
    }

    #[test]
    fn markdown_text_block_emits_numbered_list_with_inline_code() {
        let body = render_body(
            vec![("src/foo.rs".into(), 3), ("src/bar.rs".into(), 1)],
            Shape::MarkdownTextBlock,
        );
        match body {
            Body::MarkdownTextBlock(d) => {
                assert!(d.value.contains("1. `src/foo.rs` — 3"));
                assert!(d.value.contains("2. `src/bar.rs` — 1"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn unsupported_shape_falls_back_to_entries() {
        let body = render_body(vec![("src/foo.rs".into(), 3)], Shape::Badge);
        assert_eq!(
            body,
            Body::Entries(EntriesData {
                items: vec![Entry {
                    key: "src/foo.rs".into(),
                    value: Some("3".into()),
                    status: None,
                }],
            })
        );
    }

    #[test]
    fn parse_days_defaults_and_parses() {
        assert_eq!(parse_days(None), DEFAULT_DAYS);
        assert_eq!(parse_days(Some("0")), DEFAULT_DAYS);
        assert_eq!(parse_days(Some(" invalid ")), DEFAULT_DAYS);
        assert_eq!(parse_days(Some(" 14 ")), 14);
        assert_eq!(parse_days(Some("7")), 7);
    }

    #[test]
    fn short_path_returns_basename() {
        assert_eq!(short_path("a.rs"), "a.rs");
        assert_eq!(short_path("src/foo/bar.rs"), "bar.rs");
    }

    #[test]
    fn fetch_reads_workspace_repo_for_default_and_requested_shapes() {
        let fetcher = GitChurn;
        let repo = open_repo().unwrap();
        let ranked = churn(&repo, DEFAULT_DAYS, None).unwrap();

        let default_payload = run_async(fetcher.fetch(&ctx(None, None))).unwrap();
        let bars_payload = run_async(fetcher.fetch(&ctx(Some(Shape::Bars), Some("30")))).unwrap();
        let text_payload = run_async(fetcher.fetch(&ctx(Some(Shape::Text), Some("30")))).unwrap();

        assert_eq!(
            default_payload,
            payload(render_body(ranked.clone(), Shape::Entries))
        );
        assert_eq!(
            bars_payload,
            payload(render_body(ranked.clone(), Shape::Bars))
        );
        assert_eq!(text_payload, payload(render_body(ranked, Shape::Text)));
    }
}
