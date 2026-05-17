//! GitLab MR / issue DTOs and adapters into [`forge_items::ForgeRow`]. Two row variants
//! (`include_repo = true` for cross-project user-scope feeds, `false` for repo-scope feeds)
//! drive whether `references.full` or `references.short` is used as the label — same toggle
//! convention as the `github_*` family.

use serde::Deserialize;

use crate::fetcher::forge_items::ForgeRow;
use crate::fetcher::gitlab::common::parse_timestamp;

#[derive(Debug, Deserialize, Default, Clone)]
pub struct GitlabIssueLike {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub web_url: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub user_notes_count: u64,
    #[serde(default)]
    pub references: GitlabReferences,
    #[serde(default)]
    pub author: Option<GitlabAuthor>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct GitlabReferences {
    #[serde(default)]
    pub short: String,
    #[serde(default)]
    pub full: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct GitlabAuthor {
    #[serde(default)]
    pub avatar_url: Option<String>,
}

/// Convert a GitLab MR / issue payload into a renderer-ready row. The `references` block on the
/// API response already carries the pre-formatted `group/proj!42` (MR) and `group/proj#75`
/// (issue) labels, so we don't have to re-stitch project + iid ourselves.
pub fn to_forge_row(item: &GitlabIssueLike, include_repo: bool) -> ForgeRow {
    let label = if include_repo {
        item.references.full.clone()
    } else {
        item.references.short.clone()
    };
    ForgeRow {
        label: if label.is_empty() { "?".into() } else { label },
        title: item.title.clone(),
        url: if item.web_url.is_empty() {
            None
        } else {
            Some(item.web_url.clone())
        },
        avatar_url: item.author.as_ref().and_then(|a| a.avatar_url.clone()),
        avatar_path: None,
        updated_at_unix: parse_timestamp(&item.updated_at),
        activity_count: item.user_notes_count,
    }
}

pub fn to_forge_rows(items: &[GitlabIssueLike], include_repo: bool) -> Vec<ForgeRow> {
    items
        .iter()
        .map(|i| to_forge_row(i, include_repo))
        .collect()
}

/// Build a tiny [`ForgeRow`] set with stable values for `sample_body`. Sharing the helper keeps
/// every list-shape fetcher (`gitlab_my_mrs`, `gitlab_repo_issues`, …) printing the same
/// row-formatting conventions in the docs.
pub fn sample_rows(rows: &[(&str, &str, Option<&str>, u64, i64)]) -> Vec<ForgeRow> {
    rows.iter()
        .map(|(label, title, url, activity, ts)| ForgeRow {
            label: (*label).into(),
            title: (*title).into(),
            url: url.map(String::from),
            avatar_url: None,
            avatar_path: None,
            updated_at_unix: *ts,
            activity_count: *activity,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> GitlabIssueLike {
        GitlabIssueLike {
            title: "feat(gitlab): forge family".into(),
            web_url: "https://gitlab.com/g/p/-/merge_requests/42".into(),
            updated_at: "2026-04-22T10:15:30Z".into(),
            user_notes_count: 4,
            references: GitlabReferences {
                short: "!42".into(),
                full: "g/p!42".into(),
            },
            author: Some(GitlabAuthor {
                avatar_url: Some("https://avatars.example/u.png".into()),
            }),
        }
    }

    #[test]
    fn include_repo_picks_full_reference() {
        let row = to_forge_row(&item(), true);
        assert_eq!(row.label, "g/p!42");
        assert_eq!(row.title, "feat(gitlab): forge family");
        assert_eq!(
            row.url.as_deref(),
            Some("https://gitlab.com/g/p/-/merge_requests/42")
        );
        assert_eq!(
            row.avatar_url.as_deref(),
            Some("https://avatars.example/u.png")
        );
        assert_eq!(row.updated_at_unix, 1_776_852_930);
        assert_eq!(row.activity_count, 4);
    }

    #[test]
    fn exclude_repo_uses_short_reference() {
        let row = to_forge_row(&item(), false);
        assert_eq!(row.label, "!42");
    }

    #[test]
    fn missing_references_fall_back_to_question_mark() {
        let mut item = item();
        item.references.full.clear();
        item.references.short.clear();
        let row = to_forge_row(&item, true);
        assert_eq!(row.label, "?");
    }

    #[test]
    fn missing_url_collapses_to_none() {
        let mut item = item();
        item.web_url.clear();
        let row = to_forge_row(&item, true);
        assert!(row.url.is_none());
    }

    #[test]
    fn missing_author_avatar_stays_none() {
        let mut item = item();
        item.author = None;
        let row = to_forge_row(&item, true);
        assert!(row.avatar_url.is_none());
    }

    #[test]
    fn to_forge_rows_maps_each_item() {
        let rows = to_forge_rows(&[item(), item()], false);
        assert_eq!(rows.len(), 2);
        rows.iter().for_each(|r| assert_eq!(r.label, "!42"));
    }

    #[test]
    fn deserialize_tolerates_partial_payloads() {
        let raw = r#"{"title":"x","references":{"short":"!1","full":"g/p!1"}}"#;
        let item: GitlabIssueLike = serde_json::from_str(raw).unwrap();
        assert_eq!(item.title, "x");
        assert!(item.web_url.is_empty());
        assert_eq!(item.user_notes_count, 0);
        assert!(item.author.is_none());
    }
}
