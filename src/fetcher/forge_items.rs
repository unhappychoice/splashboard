//! Shared row model + multi-shape renderer for forge fetchers (`github_*`, `gitlab_*`).
//!
//! Both forges return per-PR/MR/issue rows with the same essential fields: a slug label
//! (`owner/repo#42` or `group/proj!42`), a title, a canonical URL, an author avatar URL, an
//! `updated_at` timestamp, and an activity counter (comments / notes / upvotes). Each family
//! parses its own JSON DTO into [`ForgeRow`] and hands the slice to [`render_forge_rows`],
//! which materialises any of the 8 list-shaped variants:
//!
//! `Text` / `TextBlock` / `MarkdownTextBlock` / `LinkedTextBlock` / `ImageLinkedList` /
//! `Entries` / `Bars` / `Timeline`
//!
//! Plus [`render_count_badge`] for the count-pill summary (`"5 open"` Warn / `"0 open ✓"` Ok).
//!
//! Sharing the row → body translation keeps `github_*` and `gitlab_*` from drifting on row
//! formatting, label conventions, or shape coverage as new shapes get added.

use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, ImageLinkedItem, ImageLinkedListData,
    LinkedLine, LinkedTextBlockData, MarkdownTextBlockData, Status, TextBlockData, TextData,
    TimelineData, TimelineEvent,
};
use crate::render::Shape;

/// One forge-side issue / PR / MR. The fetcher fills these from its own DTO; the renderer
/// only sees this shape, so adding a new shape variant means one switch arm here instead of
/// touching every fetcher file.
#[derive(Debug, Clone, Default)]
pub struct ForgeRow {
    /// Compact identifier, e.g. `"owner/repo#42"` (GitHub) or `"group/proj!42"` (GitLab MR).
    /// Forge-family helpers compose this so the label conventions stay in one place per family.
    pub label: String,
    pub title: String,
    /// Canonical web URL. `None` collapses the OSC 8 wrap on `LinkedTextBlock` /
    /// `ImageLinkedList`, leaving the row as plain text.
    pub url: Option<String>,
    /// Author avatar source URL. Only consumed by `ImageLinkedList` rendering, and only after
    /// the fetcher resolves it to a local cache path via [`super::thumbnails::download_many`].
    pub avatar_url: Option<String>,
    /// Resolved local thumbnail path. Pre-fill only for the `ImageLinkedList` shape — other
    /// shapes don't use it, so leaving it `None` saves a thumbnail-download roundtrip.
    pub avatar_path: Option<String>,
    /// Unix seconds UTC. Used by `Timeline`; rendered as relative `"3h ago"` at draw time.
    pub updated_at_unix: i64,
    /// Activity weight (comments / notes / upvotes). `Bars` uses this so the bar height
    /// surfaces "where the discussion is" rather than re-encoding row order.
    pub activity_count: u64,
}

/// Render rows into the requested shape. Falls back to `TextBlock` for shape variants the
/// rows can't meaningfully express (catches accidental misroutes; the runtime guards against
/// it upstream too).
pub fn render_forge_rows(rows: &[ForgeRow], shape: Shape) -> Body {
    match shape {
        Shape::Text => render_text(rows),
        Shape::TextBlock => render_text_block(rows),
        Shape::MarkdownTextBlock => render_markdown(rows),
        Shape::LinkedTextBlock => render_linked(rows),
        Shape::ImageLinkedList => render_image_linked(rows),
        Shape::Entries => render_entries(rows),
        Shape::Bars => render_bars(rows),
        Shape::Timeline => render_timeline(rows),
        _ => render_text_block(rows),
    }
}

/// Render a count-pill badge: warm tone when there's open work, calm tone when the queue is
/// empty. Pass the singular and plural noun separately so verb-shaped labels ("to review",
/// "to merge") don't get the generic `+s` and end up reading as "2 to reviews".
pub fn render_count_badge(count: u64, singular: &str, plural: &str) -> Body {
    let (status, suffix) = if count == 0 {
        (Status::Ok, " ✓")
    } else {
        (Status::Warn, "")
    };
    let noun = if count == 1 { singular } else { plural };
    Body::Badge(BadgeData {
        status,
        label: format!("{count} {noun}{suffix}"),
    })
}

fn render_text(rows: &[ForgeRow]) -> Body {
    let value = rows
        .first()
        .map(|r| format!("{} {}", r.label, r.title))
        .unwrap_or_default();
    Body::Text(TextData { value })
}

fn render_text_block(rows: &[ForgeRow]) -> Body {
    Body::TextBlock(TextBlockData {
        lines: rows
            .iter()
            .map(|r| format!("{} {}", r.label, r.title))
            .collect(),
    })
}

fn render_markdown(rows: &[ForgeRow]) -> Body {
    let value = rows
        .iter()
        .map(|r| {
            let label = escape_markdown_inline(&r.label);
            let title = escape_markdown_inline(&r.title);
            match &r.url {
                Some(url) => format!("- **{label}** [{title}]({})", escape_markdown_url(url)),
                None => format!("- **{label}** {title}"),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    Body::MarkdownTextBlock(MarkdownTextBlockData { value })
}

/// Escape the small set of Markdown inline metacharacters likely to appear in forge titles
/// and labels: `\`, `` ` ``, `*`, `_`, `[`, `]`. `<` and `&` are not handled because
/// `text_markdown` consumes the value as Markdown source and CommonMark treats them as
/// literal text unless followed by a valid HTML tag — which forge titles never are.
fn escape_markdown_inline(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

/// URL-side escape limited to the two characters that can break a Markdown link target:
/// closing paren ends the `(...)`, backslash is the escape glyph itself. Everything else is
/// already URL-encoded by the forge.
fn escape_markdown_url(url: &str) -> String {
    url.replace('\\', "%5C").replace(')', "%29")
}

fn render_linked(rows: &[ForgeRow]) -> Body {
    Body::LinkedTextBlock(LinkedTextBlockData {
        items: rows
            .iter()
            .map(|r| LinkedLine {
                text: format!("{} {}", r.label, r.title),
                url: r.url.clone(),
            })
            .collect(),
    })
}

fn render_image_linked(rows: &[ForgeRow]) -> Body {
    Body::ImageLinkedList(ImageLinkedListData {
        items: rows
            .iter()
            .map(|r| ImageLinkedItem {
                title: format!("{} {}", r.label, r.title),
                url: r.url.clone(),
                thumbnail_path: r.avatar_path.clone(),
                subtitle: None,
            })
            .collect(),
    })
}

fn render_entries(rows: &[ForgeRow]) -> Body {
    Body::Entries(EntriesData {
        items: rows
            .iter()
            .map(|r| Entry {
                key: r.label.clone(),
                value: Some(r.title.clone()),
                status: None,
            })
            .collect(),
    })
}

fn render_bars(rows: &[ForgeRow]) -> Body {
    Body::Bars(BarsData {
        bars: rows
            .iter()
            .map(|r| Bar {
                label: r.label.clone(),
                value: r.activity_count,
                value_label: None,
            })
            .collect(),
    })
}

fn render_timeline(rows: &[ForgeRow]) -> Body {
    Body::Timeline(TimelineData {
        events: rows
            .iter()
            .map(|r| TimelineEvent {
                timestamp: r.updated_at_unix,
                title: r.label.clone(),
                detail: Some(r.title.clone()),
                status: None,
            })
            .collect(),
    })
}

/// Catalog of shapes the multi-shape list fetchers (`*_my_prs`, `*_review_requests`,
/// `*_repo_prs/mrs`, `*_repo_issues`) expose. Default is `LinkedTextBlock`. Centralised here so
/// both forge families share a single table — adding `MarkdownTextBlock` here surfaces it
/// across both at once.
pub const LIST_SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::ImageLinkedList,
    Shape::Entries,
    Shape::Text,
    Shape::Bars,
    Shape::Badge,
    Shape::Timeline,
];

/// Async dispatcher for list-shaped forge fetchers. Branches on `shape`:
///
/// - `Badge` → count-pill summary using the caller's singular / plural noun;
/// - `ImageLinkedList` → resolves each row's `avatar_url` through the shared thumbnails cache
///   so the renderer reads from local files;
/// - anything else → straight through [`render_forge_rows`].
///
/// One helper for both github and gitlab so the shape coverage stays uniform.
pub async fn dispatch_rows_async(
    mut rows: Vec<ForgeRow>,
    shape: Shape,
    badge_singular: &str,
    badge_plural: &str,
) -> Body {
    if shape == Shape::Badge {
        return render_count_badge(rows.len() as u64, badge_singular, badge_plural);
    }
    if shape == Shape::ImageLinkedList {
        let urls: Vec<Option<String>> = rows.iter().map(|r| r.avatar_url.clone()).collect();
        let paths = crate::fetcher::thumbnails::download_many(&urls).await;
        rows.iter_mut().zip(paths).for_each(|(row, path)| {
            row.avatar_path = path.map(|p| p.to_string_lossy().into_owned());
        });
    }
    render_forge_rows(&rows, shape)
}

/// Sync sibling of [`dispatch_rows_async`] used by `sample_body`. Skips thumbnail resolution
/// (samples don't carry real `avatar_url` values) and returns `None` for shapes outside
/// [`LIST_SHAPES`] so docs generation never accidentally promotes an unsupported variant.
pub fn dispatch_sample(
    rows: &[ForgeRow],
    shape: Shape,
    badge_singular: &str,
    badge_plural: &str,
) -> Option<Body> {
    if shape == Shape::Badge {
        return Some(render_count_badge(
            rows.len() as u64,
            badge_singular,
            badge_plural,
        ));
    }
    if !LIST_SHAPES.contains(&shape) {
        return None;
    }
    Some(render_forge_rows(rows, shape))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<ForgeRow> {
        vec![
            ForgeRow {
                label: "splashboard#54".into(),
                title: "feat(docs): widget catalogue".into(),
                url: Some("https://github.com/u/splashboard/pull/54".into()),
                avatar_url: Some("https://avatars.example/u54.png".into()),
                avatar_path: Some("/tmp/u54.png".into()),
                updated_at_unix: 1_774_000_000,
                activity_count: 7,
            },
            ForgeRow {
                label: "splashboard#51".into(),
                title: "feat(fetcher): split clock options".into(),
                url: None,
                avatar_url: None,
                avatar_path: None,
                updated_at_unix: 1_773_800_000,
                activity_count: 2,
            },
        ]
    }

    #[test]
    fn text_picks_the_first_row_headline() {
        let Body::Text(t) = render_forge_rows(&rows(), Shape::Text) else {
            panic!("expected text");
        };
        assert!(t.value.starts_with("splashboard#54"));
    }

    #[test]
    fn text_block_emits_one_line_per_row() {
        let Body::TextBlock(b) = render_forge_rows(&rows(), Shape::TextBlock) else {
            panic!("expected text block");
        };
        assert_eq!(b.lines.len(), 2);
        assert!(b.lines[1].contains("split clock options"));
    }

    #[test]
    fn markdown_links_only_when_url_is_present() {
        let Body::MarkdownTextBlock(b) = render_forge_rows(&rows(), Shape::MarkdownTextBlock)
        else {
            panic!("expected markdown");
        };
        assert!(
            b.value
                .contains("[feat(docs): widget catalogue](https://github.com")
        );
        // Row without url still renders, just without the link syntax.
        assert!(b.value.contains("- **splashboard#51** feat(fetcher)"));
    }

    #[test]
    fn markdown_escapes_inline_metacharacters_in_label_and_title() {
        // A title like `fix ](oops)` used to escape the link target prematurely; `*_wip_*`
        // used to render as bold-italic text instead of literal underscores and asterisks.
        // Both are user-supplied (forge issue / PR titles) and must round-trip as text, not
        // markup.
        let rows = vec![ForgeRow {
            label: "splashboard#99".into(),
            title: "fix ](oops) and *_wip_*".into(),
            url: Some("https://example.com/path?x=1)y".into()),
            avatar_url: None,
            avatar_path: None,
            updated_at_unix: 1_700_000_000,
            activity_count: 0,
        }];
        let Body::MarkdownTextBlock(b) = render_forge_rows(&rows, Shape::MarkdownTextBlock) else {
            panic!("expected markdown");
        };
        // `]` is escaped so it can't close the surrounding markdown link prematurely;
        // `*` / `_` are escaped so the title stays literal rather than rendering bold /
        // italic. Parens are intentionally not escaped because they only carry markup
        // significance inside the link target itself, not in the visible link text.
        assert!(
            b.value.contains("fix \\](oops) and \\*\\_wip\\_\\*"),
            "title not escaped: {}",
            b.value
        );
        // URL keeps its closing paren escaped so it can't truncate the markdown link target.
        assert!(
            b.value.contains("1%29y"),
            "url paren not escaped: {}",
            b.value
        );
    }

    #[test]
    fn escape_markdown_inline_handles_each_documented_glyph() {
        assert_eq!(
            escape_markdown_inline("a\\b`c*d_e[f]g"),
            "a\\\\b\\`c\\*d\\_e\\[f\\]g"
        );
        assert_eq!(escape_markdown_inline("plain text"), "plain text");
    }

    #[test]
    fn linked_carries_optional_url_through() {
        let Body::LinkedTextBlock(b) = render_forge_rows(&rows(), Shape::LinkedTextBlock) else {
            panic!("expected linked text block");
        };
        assert_eq!(b.items.len(), 2);
        assert!(b.items[0].url.is_some());
        assert!(b.items[1].url.is_none());
    }

    #[test]
    fn image_linked_passes_thumbnail_paths_through() {
        let Body::ImageLinkedList(b) = render_forge_rows(&rows(), Shape::ImageLinkedList) else {
            panic!("expected image linked list");
        };
        assert_eq!(b.items[0].thumbnail_path.as_deref(), Some("/tmp/u54.png"));
        assert!(b.items[1].thumbnail_path.is_none());
    }

    #[test]
    fn entries_maps_label_to_title() {
        let Body::Entries(b) = render_forge_rows(&rows(), Shape::Entries) else {
            panic!("expected entries");
        };
        assert_eq!(b.items[0].key, "splashboard#54");
        assert_eq!(
            b.items[1].value.as_deref(),
            Some("feat(fetcher): split clock options")
        );
    }

    #[test]
    fn bars_carries_activity_count_as_value() {
        let Body::Bars(b) = render_forge_rows(&rows(), Shape::Bars) else {
            panic!("expected bars");
        };
        assert_eq!(b.bars[0].value, 7);
        assert_eq!(b.bars[1].value, 2);
    }

    #[test]
    fn timeline_uses_updated_at_unix() {
        let Body::Timeline(t) = render_forge_rows(&rows(), Shape::Timeline) else {
            panic!("expected timeline");
        };
        assert_eq!(t.events[0].timestamp, 1_774_000_000);
        assert_eq!(
            t.events[0].detail.as_deref(),
            Some("feat(docs): widget catalogue")
        );
    }

    #[test]
    fn unsupported_shape_falls_back_to_text_block() {
        // Ratio / NumberSeries / Image / etc. shouldn't be routed here — the runtime guards
        // upstream — but if one slips through the fallback should still be intelligible.
        let Body::TextBlock(_) = render_forge_rows(&rows(), Shape::Ratio) else {
            panic!("expected fallback to text block");
        };
    }

    #[test]
    fn count_badge_flips_status_at_zero() {
        let Body::Badge(b) = render_count_badge(0, "open MR", "open MRs") else {
            panic!("expected badge");
        };
        assert_eq!(b.status, Status::Ok);
        assert_eq!(b.label, "0 open MRs ✓");

        let Body::Badge(b) = render_count_badge(1, "open MR", "open MRs") else {
            panic!("expected badge");
        };
        assert_eq!(b.status, Status::Warn);
        assert_eq!(b.label, "1 open MR");

        let Body::Badge(b) = render_count_badge(5, "open MR", "open MRs") else {
            panic!("expected badge");
        };
        assert_eq!(b.label, "5 open MRs");
    }

    #[test]
    fn count_badge_keeps_verb_form_intact_across_counts() {
        // Verb-shaped labels like "to review" want the same form for 1 and N — no bare 's'
        // suffix dropped onto the verb.
        let Body::Badge(b) = render_count_badge(1, "to review", "to review") else {
            panic!("expected badge");
        };
        assert_eq!(b.label, "1 to review");
        let Body::Badge(b) = render_count_badge(3, "to review", "to review") else {
            panic!("expected badge");
        };
        assert_eq!(b.label, "3 to review");
    }

    #[test]
    fn list_shapes_covers_the_eight_documented_variants() {
        assert_eq!(LIST_SHAPES.len(), 9);
        assert_eq!(LIST_SHAPES[0], Shape::LinkedTextBlock);
        assert!(LIST_SHAPES.contains(&Shape::ImageLinkedList));
        assert!(LIST_SHAPES.contains(&Shape::Badge));
    }

    #[tokio::test]
    async fn dispatch_rows_async_handles_badge_and_passes_through_others() {
        // Badge short-circuits the row pipeline, so the noun pair is honoured.
        let Body::Badge(b) = dispatch_rows_async(rows(), Shape::Badge, "open PR", "open PRs").await
        else {
            panic!("expected badge");
        };
        assert_eq!(b.label, "2 open PRs");

        // Non-Badge / non-ImageLinkedList shapes pass straight through to the row renderer.
        let Body::TextBlock(t) =
            dispatch_rows_async(rows(), Shape::TextBlock, "open PR", "open PRs").await
        else {
            panic!("expected text block");
        };
        assert_eq!(t.lines.len(), 2);
    }

    #[test]
    fn dispatch_sample_returns_none_for_unsupported_shapes() {
        let rows = rows();
        assert!(dispatch_sample(&rows, Shape::Ratio, "open PR", "open PRs").is_none());

        let Some(Body::Badge(b)) = dispatch_sample(&rows, Shape::Badge, "open PR", "open PRs")
        else {
            panic!("expected badge");
        };
        assert_eq!(b.label, "2 open PRs");

        let Some(Body::Entries(e)) = dispatch_sample(&rows, Shape::Entries, "open PR", "open PRs")
        else {
            panic!("expected entries");
        };
        assert_eq!(e.items.len(), 2);
    }

    #[test]
    fn empty_rows_collapse_to_empty_bodies() {
        let Body::Text(t) = render_forge_rows(&[], Shape::Text) else {
            panic!("expected text");
        };
        assert!(t.value.is_empty());

        let Body::TextBlock(b) = render_forge_rows(&[], Shape::TextBlock) else {
            panic!("expected text block");
        };
        assert!(b.lines.is_empty());

        let Body::LinkedTextBlock(b) = render_forge_rows(&[], Shape::LinkedTextBlock) else {
            panic!("expected linked text block");
        };
        assert!(b.items.is_empty());
    }
}
