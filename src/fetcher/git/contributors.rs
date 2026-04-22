use std::collections::HashMap;

use async_trait::async_trait;
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;

use crate::payload::{Bar, BarsData, Body, EntriesData, Entry, Payload};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, lines_body, open_repo, payload, repo_cache_key};

const SHAPES: &[Shape] = &[Shape::Bars, Shape::Entries, Shape::Lines];
const DEFAULT_DAYS: u64 = 30;
const MAX_ENTRIES: usize = 10;

/// Commit authors over the last N days (default 30, configurable via `format = "N"`). Ranked
/// by commit count, truncated to the top 10 so a busy repo doesn't blow up the widget. `Bars`
/// is the default (visual ranking); `Entries` emits `name: count` rows; `Lines` collapses to
/// `"alice (23), bob (11)"` style.
pub struct GitContributors;

#[async_trait]
impl Fetcher for GitContributors {
    fn name(&self) -> &str {
        "git_contributors"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
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
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        let days = parse_days(ctx.format.as_deref());
        let ranked = contributors(&repo, days)?;
        Ok(payload(render_body(
            ranked,
            ctx.shape.unwrap_or(Shape::Bars),
        )))
    }
}

fn contributors(repo: &gix::Repository, days: u64) -> Result<Vec<(String, u64)>, FetchError> {
    let Ok(head_id) = repo.head_id() else {
        return Ok(Vec::new());
    };
    let cutoff = chrono::Utc::now().timestamp() - (days as i64) * 86_400;
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
        Shape::Lines => {
            if ranked.is_empty() {
                lines_body(Vec::new())
            } else {
                let line = ranked
                    .into_iter()
                    .map(|(name, count)| format!("{name} ({count})"))
                    .collect::<Vec<_>>()
                    .join(", ");
                lines_body(vec![line])
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
    use super::super::test_support::{commit_as, make_repo};
    use super::*;

    #[test]
    fn empty_repo_returns_empty() {
        let (_tmp, repo) = make_repo();
        assert!(contributors(&repo, 30).unwrap().is_empty());
    }

    #[test]
    fn tallies_authors_in_rank_order() {
        let (_tmp, repo) = make_repo();
        commit_as(&repo, "a1", "alice", "a@example.com");
        commit_as(&repo, "b1", "bob", "b@example.com");
        commit_as(&repo, "a2", "alice", "a@example.com");
        commit_as(&repo, "a3", "alice", "a@example.com");
        let ranked = contributors(&repo, 30).unwrap();
        assert_eq!(ranked, vec![("alice".into(), 3u64), ("bob".into(), 1u64)]);
    }

    #[test]
    fn bars_shape_from_ranking() {
        let body = render_body(vec![("alice".into(), 3), ("bob".into(), 1)], Shape::Bars);
        match body {
            Body::Bars(d) => {
                assert_eq!(d.bars.len(), 2);
                assert_eq!(d.bars[0].label, "alice");
                assert_eq!(d.bars[0].value, 3);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn lines_shape_empty_when_empty() {
        let body = render_body(Vec::new(), Shape::Lines);
        match body {
            Body::Lines(d) => assert!(d.lines.is_empty()),
            _ => panic!(),
        }
    }

    #[test]
    fn lines_shape_joins_with_counts() {
        let body = render_body(vec![("alice".into(), 3), ("bob".into(), 1)], Shape::Lines);
        match body {
            Body::Lines(d) => assert_eq!(d.lines, vec!["alice (3), bob (1)".to_string()]),
            _ => panic!(),
        }
    }

    #[test]
    fn parse_days_defaults_and_parses() {
        assert_eq!(parse_days(None), DEFAULT_DAYS);
        assert_eq!(parse_days(Some("0")), DEFAULT_DAYS);
        assert_eq!(parse_days(Some("7")), 7);
    }
}
