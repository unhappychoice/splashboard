use async_trait::async_trait;
use gix::prelude::ObjectIdExt;

use crate::payload::{Body, Payload, TimelineData, TimelineEvent};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, open_repo, payload, repo_cache_key, text_block_body};

const SHAPES: &[Shape] = &[Shape::Timeline, Shape::TextBlock];
const DEFAULT_LIMIT: usize = 10;

/// Newest commits on HEAD, walked via gix rev-walk. The `format` field is parsed as a commit
/// count (`"5"`, `"20"`) — absent or unparseable falls back to 10. `TextBlock` renders
/// `<short> <summary>` one per line so users who want a flat list instead of the timeline
/// renderer's relative-ago labels can still wire it up.
pub struct GitRecentCommits;

#[async_trait]
impl Fetcher for GitRecentCommits {
    fn name(&self) -> &str {
        "git_recent_commits"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Newest commits on HEAD, defaulting to ten (override with `format = \"N\"`). `Timeline` shows relative-ago labels; `TextBlock` is a flat `<short> <summary>` list."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Timeline
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        repo_cache_key(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Timeline => samples::timeline(&[
                (1_700_000_000, "feat(render): add heatmap", Some("a1b2c3d")),
                (1_699_990_000, "fix(fetcher): tz fallback", Some("d4e5f6a")),
                (1_699_900_000, "chore: bump ratatui", Some("e7f8a9b")),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "a1b2c3d feat(render): add heatmap",
                "d4e5f6a fix(fetcher): tz fallback",
                "e7f8a9b chore: bump ratatui",
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        let limit = parse_limit(ctx.format.as_deref());
        let commits = recent_commits(&repo, limit)?;
        Ok(payload(render_body(
            commits,
            ctx.shape.unwrap_or(Shape::Timeline),
        )))
    }
}

struct CommitInfo {
    short_hash: String,
    summary: String,
    timestamp: i64,
}

fn recent_commits(repo: &gix::Repository, limit: usize) -> Result<Vec<CommitInfo>, FetchError> {
    let Ok(head_id) = repo.head_id() else {
        return Ok(Vec::new());
    };
    let walker = repo.rev_walk([head_id.detach()]).all().map_err(fail)?;
    let mut out = Vec::with_capacity(limit);
    for info in walker.take(limit) {
        let Ok(info) = info else { continue };
        let Ok(commit) = repo.find_commit(info.id) else {
            continue;
        };
        let summary = commit
            .message()
            .ok()
            .map(|m| m.summary().to_string())
            .unwrap_or_default();
        let timestamp = commit.time().map(|t| t.seconds).unwrap_or(0);
        let short_hash = info
            .id
            .attach(repo)
            .shorten()
            .map(|p| p.to_string())
            .unwrap_or_default();
        out.push(CommitInfo {
            short_hash,
            summary,
            timestamp,
        });
    }
    Ok(out)
}

fn parse_limit(format: Option<&str>) -> usize {
    format
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_LIMIT)
}

fn render_body(commits: Vec<CommitInfo>, shape: Shape) -> Body {
    match shape {
        Shape::TextBlock => text_block_body(
            commits
                .into_iter()
                .map(|c| format!("{} {}", c.short_hash, c.summary))
                .collect(),
        ),
        _ => Body::Timeline(TimelineData {
            events: commits
                .into_iter()
                .map(|c| TimelineEvent {
                    timestamp: c.timestamp,
                    title: c.summary,
                    detail: Some(c.short_hash),
                    status: None,
                })
                .collect(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{commit, make_repo};
    use super::*;

    #[test]
    fn empty_repo_returns_empty_timeline() {
        let (_tmp, repo) = make_repo();
        let commits = recent_commits(&repo, 10).unwrap();
        assert!(commits.is_empty());
    }

    #[test]
    fn recent_commits_are_newest_first() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "one");
        commit(&repo, "two");
        commit(&repo, "three");
        let commits = recent_commits(&repo, 10).unwrap();
        let titles: Vec<_> = commits.iter().map(|c| c.summary.clone()).collect();
        assert_eq!(titles, vec!["three", "two", "one"]);
    }

    #[test]
    fn limit_truncates() {
        let (_tmp, repo) = make_repo();
        for i in 0..5 {
            commit(&repo, &format!("c{i}"));
        }
        let commits = recent_commits(&repo, 2).unwrap();
        assert_eq!(commits.len(), 2);
    }

    #[test]
    fn timeline_shape_emits_events_with_short_hash_detail() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "x");
        let body = render_body(recent_commits(&repo, 10).unwrap(), Shape::Timeline);
        match body {
            Body::Timeline(d) => {
                assert_eq!(d.events.len(), 1);
                assert_eq!(d.events[0].title, "x");
                assert!(d.events[0].detail.as_deref().unwrap().len() >= 7);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn text_block_shape_prefixes_short_hash() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "hello");
        let body = render_body(recent_commits(&repo, 10).unwrap(), Shape::TextBlock);
        match body {
            Body::TextBlock(d) => {
                assert_eq!(d.lines.len(), 1);
                assert!(d.lines[0].ends_with(" hello"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_limit_default() {
        assert_eq!(parse_limit(None), DEFAULT_LIMIT);
        assert_eq!(parse_limit(Some("")), DEFAULT_LIMIT);
        assert_eq!(parse_limit(Some("abc")), DEFAULT_LIMIT);
        assert_eq!(parse_limit(Some("0")), DEFAULT_LIMIT);
    }

    #[test]
    fn parse_limit_parses_number() {
        assert_eq!(parse_limit(Some("3")), 3);
        assert_eq!(parse_limit(Some(" 7 ")), 7);
    }
}
