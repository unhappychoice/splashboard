//! Shared response types for the issue / PR family. The REST search endpoints and the
//! per-repo PR/issue endpoints return almost the same item shape; they share this struct so
//! each fetcher can focus on its own URL composition and rendering.

use serde::Deserialize;

use crate::payload::{Body, EntriesData, Entry, TextBlockData, TimelineData, TimelineEvent};
use crate::render::Shape;

use super::common::{RepoSlug, parse_timestamp};

#[derive(Debug, Deserialize)]
pub struct IssueItem {
    pub title: String,
    pub number: u64,
    /// Only present on the `/search/issues` response. Per-repo endpoints omit this.
    #[serde(default)]
    pub repository_url: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub state: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchResult {
    #[serde(default)]
    pub items: Vec<IssueItem>,
}

/// Extracts `owner/name` from a GitHub API repository URL
/// (`https://api.github.com/repos/owner/name`).
pub fn repo_from_url(url: &str) -> Option<RepoSlug> {
    let rest = url.strip_prefix("https://api.github.com/repos/")?;
    RepoSlug::parse(rest)
}

/// Renders issue / PR items to one of `TextBlock` / `Entries` / `Timeline`. When
/// `include_repo` is true, each line/event is prefixed with `owner/name` — used by user-scope
/// fetchers that return items across many repos. Per-repo fetchers pass `false` so the repo
/// name isn't repeated on every row.
pub fn render_items(items: &[IssueItem], shape: Shape, include_repo: bool) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: items
                .iter()
                .map(|i| Entry {
                    key: entries_key(i, include_repo),
                    value: Some(i.title.clone()),
                    status: None,
                })
                .collect(),
        }),
        Shape::Timeline => Body::Timeline(TimelineData {
            events: items
                .iter()
                .map(|i| TimelineEvent {
                    timestamp: parse_timestamp(&i.updated_at),
                    title: short_label(i, include_repo),
                    detail: Some(i.title.clone()),
                    status: None,
                })
                .collect(),
        }),
        _ => Body::TextBlock(TextBlockData {
            lines: items
                .iter()
                .map(|i| format!("{} {}", short_label(i, include_repo), i.title))
                .collect(),
        }),
    }
}

fn short_label(i: &IssueItem, include_repo: bool) -> String {
    if include_repo {
        let repo = repo_from_url(&i.repository_url)
            .map(|s| s.as_path())
            .unwrap_or_else(|| "?".into());
        format!("{repo}#{}", i.number)
    } else {
        format!("#{}", i.number)
    }
}

fn entries_key(i: &IssueItem, include_repo: bool) -> String {
    if include_repo {
        let name = repo_from_url(&i.repository_url)
            .map(|s| s.name)
            .unwrap_or_else(|| "?".into());
        format!("{name} #{}", i.number)
    } else {
        format!("#{}", i.number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_from_url_extracts_slug() {
        let s = repo_from_url("https://api.github.com/repos/foo/bar").unwrap();
        assert_eq!(s.owner, "foo");
        assert_eq!(s.name, "bar");
    }

    #[test]
    fn repo_from_url_rejects_non_api_host() {
        assert!(repo_from_url("https://github.com/foo/bar").is_none());
    }
}
