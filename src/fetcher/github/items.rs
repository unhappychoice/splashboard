//! Shared response types for the issue / PR family. The REST search endpoints and the
//! per-repo PR/issue endpoints return almost the same item shape; they share this struct so
//! each fetcher can focus on its own URL composition and rendering.
//!
//! Rendering goes through [`crate::fetcher::forge_items::render_forge_rows`] so the github and
//! gitlab families produce structurally identical output for every shape — adding a new shape
//! variant lights up on both families simultaneously.

use serde::Deserialize;

use crate::fetcher::forge_items::{self, ForgeRow};
use crate::payload::Body;
use crate::render::Shape;

use super::client::api_repos_prefix;
use super::common::{RepoSlug, parse_timestamp};

#[derive(Debug, Deserialize, Default)]
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
    /// Comments count, fed into the `Bars` shape as a per-row activity weight.
    #[serde(default)]
    pub comments: u64,
    /// Author block — only `avatar_url` is used today (for the `ImageLinkedList` shape).
    #[serde(default)]
    pub user: Option<GithubUserRef>,
}

#[derive(Debug, Deserialize, Default)]
pub struct GithubUserRef {
    #[serde(default)]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SearchResult {
    #[serde(default)]
    pub items: Vec<IssueItem>,
}

/// Extracts `owner/name` from a GitHub API repository URL. The prefix tracks the configured
/// host via `client::api_repos_prefix()`.
pub fn repo_from_url(url: &str) -> Option<RepoSlug> {
    let rest = url.strip_prefix(api_repos_prefix())?;
    RepoSlug::parse(rest)
}

/// Sync-only path for fetchers whose `shapes()` set doesn't include `Badge` /
/// `ImageLinkedList` (currently `good_first_issues`). Both of those variants need
/// preprocessing that this signature can't reach: `Badge` needs the row count + a noun pair
/// for `forge_items::render_count_badge`, and `ImageLinkedList` needs avatar URLs resolved
/// through `thumbnails::download_many` before render. Fetchers that accept those shapes go
/// through `forge_items::dispatch_rows_async` instead.
///
/// When `include_repo` is true, each label is prefixed with `owner/name#42` (used by
/// user-scope fetchers whose results span many repos). Per-repo fetchers pass `false` so the
/// row stays at `#42`.
pub fn render_items(items: &[IssueItem], shape: Shape, include_repo: bool) -> Body {
    debug_assert!(
        !matches!(shape, Shape::Badge | Shape::ImageLinkedList),
        "render_items is sync-only; route {shape:?} through forge_items::dispatch_rows_async"
    );
    let rows = to_forge_rows(items, include_repo);
    forge_items::render_forge_rows(&rows, shape)
}

/// Lower-level converter so fetchers can pre-process rows (e.g. resolving thumbnails for
/// `ImageLinkedList`) before handing them to the shared renderer.
pub fn to_forge_rows(items: &[IssueItem], include_repo: bool) -> Vec<ForgeRow> {
    items
        .iter()
        .map(|i| to_forge_row(i, include_repo))
        .collect()
}

/// Build a tiny [`ForgeRow`] set with stable values for `sample_body`. Mirrors the gitlab
/// helper so both families share the same row-formatting conventions in docs.
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

fn to_forge_row(i: &IssueItem, include_repo: bool) -> ForgeRow {
    let label = if include_repo {
        let repo = repo_from_url(&i.repository_url)
            .map(|s| s.as_path())
            .unwrap_or_else(|| "?".into());
        format!("{repo}#{}", i.number)
    } else {
        format!("#{}", i.number)
    };
    ForgeRow {
        label,
        title: i.title.clone(),
        url: if i.html_url.is_empty() {
            None
        } else {
            Some(i.html_url.clone())
        },
        avatar_url: i.user.as_ref().and_then(|u| u.avatar_url.clone()),
        avatar_path: None,
        updated_at_unix: parse_timestamp(&i.updated_at),
        activity_count: i.comments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn issue_item() -> IssueItem {
        IssueItem {
            title: "Fix cached splash mismatch".into(),
            number: 42,
            repository_url: "https://api.github.com/repos/unhappychoice/splashboard".into(),
            updated_at: "2026-04-22T10:15:30Z".into(),
            html_url: "https://github.com/unhappychoice/splashboard/issues/42".into(),
            state: "open".into(),
            comments: 3,
            user: Some(GithubUserRef {
                avatar_url: Some("https://avatars.example/u.png".into()),
            }),
        }
    }

    fn body_json(body: Body) -> Value {
        serde_json::to_value(body).unwrap()
    }

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

    #[test]
    fn repo_from_url_rejects_incomplete_slug() {
        assert!(repo_from_url("https://api.github.com/repos/foo").is_none());
    }

    #[test]
    fn to_forge_row_carries_comments_and_avatar_through() {
        let row = to_forge_row(&issue_item(), true);
        assert_eq!(row.label, "unhappychoice/splashboard#42");
        assert_eq!(row.activity_count, 3);
        assert_eq!(
            row.avatar_url.as_deref(),
            Some("https://avatars.example/u.png")
        );
        assert!(row.avatar_path.is_none());
    }

    #[test]
    fn to_forge_row_without_repo_uses_number_only_label() {
        let row = to_forge_row(&issue_item(), false);
        assert_eq!(row.label, "#42");
    }

    #[test]
    fn to_forge_row_falls_back_to_question_mark_for_bad_repo_url() {
        let mut item = issue_item();
        item.repository_url = "https://api.github.com/not-a-repo".into();
        let row = to_forge_row(&item, true);
        assert_eq!(row.label, "?#42");
    }

    #[test]
    fn to_forge_row_empty_html_url_collapses_to_none() {
        let mut item = issue_item();
        item.html_url.clear();
        let row = to_forge_row(&item, false);
        assert!(row.url.is_none());
    }

    #[test]
    fn render_entries_include_repo_name_and_title() {
        let value = body_json(render_items(&[issue_item()], Shape::Entries, true));
        assert_eq!(value["shape"], "entries");
        assert_eq!(
            value["data"]["items"][0]["key"],
            "unhappychoice/splashboard#42"
        );
        assert_eq!(
            value["data"]["items"][0]["value"],
            "Fix cached splash mismatch"
        );
        assert!(value["data"]["items"][0]["status"].is_null());
    }

    #[test]
    fn render_linked_text_block_uses_short_label_and_optional_url() {
        let mut item = issue_item();
        item.html_url.clear();

        let Body::LinkedTextBlock(data) = render_items(&[item], Shape::LinkedTextBlock, false)
        else {
            panic!("expected linked text block");
        };
        assert_eq!(data.items.len(), 1);
        assert_eq!(data.items[0].text, "#42 Fix cached splash mismatch");
        assert!(data.items[0].url.is_none());
    }

    #[test]
    fn render_linked_text_block_prefixes_repo_path_and_uses_html_url() {
        let value = body_json(render_items(&[issue_item()], Shape::LinkedTextBlock, true));
        assert_eq!(value["shape"], "linked_text_block");
        assert_eq!(
            value["data"]["items"][0]["text"],
            "unhappychoice/splashboard#42 Fix cached splash mismatch"
        );
        assert_eq!(
            value["data"]["items"][0]["url"],
            "https://github.com/unhappychoice/splashboard/issues/42"
        );
    }

    #[test]
    fn render_timeline_uses_repo_fallback_and_parsed_timestamp() {
        let mut item = issue_item();
        item.repository_url = "https://api.github.com/not-a-repo".into();

        let Body::Timeline(data) = render_items(&[item], Shape::Timeline, true) else {
            panic!("expected timeline");
        };
        assert_eq!(data.events.len(), 1);
        assert_eq!(data.events[0].timestamp, 1_776_852_930);
        assert_eq!(data.events[0].title, "?#42");
        assert_eq!(
            data.events[0].detail.as_deref(),
            Some("Fix cached splash mismatch")
        );
        assert!(data.events[0].status.is_none());
    }

    #[test]
    fn render_text_block_is_the_fallback_shape() {
        let Body::TextBlock(data) = render_items(&[issue_item()], Shape::TextBlock, false) else {
            panic!("expected text block");
        };
        assert_eq!(data.lines, vec!["#42 Fix cached splash mismatch"]);
    }

    #[test]
    fn render_entries_without_repo_prefix_use_number_only() {
        let value = body_json(render_items(&[issue_item()], Shape::Entries, false));
        assert_eq!(value["shape"], "entries");
        assert_eq!(value["data"]["items"][0]["key"], "#42");
        assert_eq!(
            value["data"]["items"][0]["value"],
            "Fix cached splash mismatch"
        );
    }

    #[test]
    fn render_entries_fall_back_to_question_mark_for_bad_repo_url() {
        let mut item = issue_item();
        item.repository_url = "https://api.github.com/not-a-repo".into();
        let value = body_json(render_items(&[item], Shape::Entries, true));
        assert_eq!(value["shape"], "entries");
        assert_eq!(value["data"]["items"][0]["key"], "?#42");
    }

    #[test]
    fn render_bars_uses_comments_count_as_value() {
        let Body::Bars(data) = render_items(&[issue_item()], Shape::Bars, false) else {
            panic!("expected bars");
        };
        assert_eq!(data.bars[0].value, 3);
    }

    #[test]
    fn render_markdown_links_when_url_present() {
        let Body::MarkdownTextBlock(data) =
            render_items(&[issue_item()], Shape::MarkdownTextBlock, true)
        else {
            panic!("expected markdown");
        };
        assert!(data.value.contains("[Fix cached splash mismatch]"));
    }

    #[test]
    #[should_panic(expected = "render_items is sync-only")]
    fn render_items_rejects_image_linked_list_in_debug() {
        // `ImageLinkedList` needs avatar resolution through the async thumbnails downloader,
        // so the sync helper refuses it rather than silently emitting a row with a blank
        // thumbnail column. Fetchers accepting that shape route through
        // `forge_items::dispatch_rows_async` instead.
        let _ = render_items(&[issue_item()], Shape::ImageLinkedList, true);
    }

    #[test]
    #[should_panic(expected = "render_items is sync-only")]
    fn render_items_rejects_badge_in_debug() {
        // Same reasoning for `Badge`: it needs the row count + a noun pair, neither of which
        // are reachable through this signature.
        let _ = render_items(&[issue_item()], Shape::Badge, false);
    }

    #[test]
    fn search_result_defaults_missing_items_to_empty() {
        let result: SearchResult = serde_json::from_str("{}").unwrap();
        assert!(result.items.is_empty());
    }

    #[test]
    fn issue_item_tolerates_partial_payloads_for_new_fields() {
        // Older fixtures and tests construct IssueItem without `comments` / `user`. Both
        // default cleanly so we don't break callers that pre-date the shape expansion.
        let item: IssueItem = serde_json::from_str(r#"{"title":"x","number":1}"#).unwrap();
        assert_eq!(item.comments, 0);
        assert!(item.user.is_none());
    }
}
