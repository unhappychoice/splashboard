//! Shared row model + shape builders for the `deal_*` family.
//!
//! Each sibling (`deal_free_games`, `deal_steam_daily`, `deal_games`) parses its upstream
//! into a `Vec<DealRow>` and dispatches by shape through the builders below. Keeping the
//! row vocabulary uniform means a renderer swapping between siblings sees consistent
//! `[Store] Title  $price (X% off)` output regardless of source.

use chrono::{DateTime, Utc};

use crate::fetcher::thumbnails;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, ImageLinkedItem, ImageLinkedListData,
    LinkedLine, LinkedTextBlockData, MarkdownTextBlockData, Status, TextBlockData, TextData,
    TimelineData, TimelineEvent,
};
use crate::render::Shape;

/// Cap multi-row shapes so a runaway upstream can't swamp a single widget slot.
pub(super) const MAX_ROWS: usize = 20;
/// Bars / entries labels longer than this get an ellipsis. Picked to fit a typical sidebar.
const LABEL_MAX_CHARS: usize = 40;

#[derive(Debug, Clone)]
pub(super) struct DealRow {
    pub title: String,
    pub image_url: Option<String>,
    pub sale_price: Option<String>,
    pub original_price: Option<String>,
    pub discount_pct: Option<u32>,
    pub store: Option<String>,
    pub link: String,
    pub published: Option<DateTime<Utc>>,
}

impl DealRow {
    /// `"Title  $4.99 (50% off from $9.99)"`, with each piece dropping out when absent.
    pub(super) fn label(&self) -> String {
        let mut s = self.title.clone();
        match (&self.sale_price, &self.original_price, self.discount_pct) {
            (Some(price), Some(orig), Some(pct)) => {
                s.push_str(&format!("  {price} ({pct}% off from {orig})"));
            }
            (Some(price), _, Some(pct)) => s.push_str(&format!("  {price} ({pct}% off)")),
            (Some(price), _, None) => s.push_str(&format!("  {price}")),
            (None, _, Some(pct)) => s.push_str(&format!("  {pct}% off")),
            (None, _, None) => {}
        }
        s
    }

    /// Same as [`label`](Self::label) but prefixed with `[Store]` when known.
    pub(super) fn label_with_store(&self) -> String {
        match &self.store {
            Some(store) => format!("[{store}] {}", self.label()),
            None => self.label(),
        }
    }

    /// Short subtitle for `Timeline` / `ImageLinkedList` rows: `"Store · $X · 50% off"`.
    fn subtitle(&self) -> Option<String> {
        let pieces: Vec<String> = [
            self.store.clone(),
            self.sale_price.clone(),
            self.discount_pct.map(|p| format!("{p}% off")),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!pieces.is_empty()).then(|| pieces.join(" · "))
    }
}

/// Dispatch a non-image shape to its body builder. `ImageLinkedList` and `Image` are
/// handled separately because they need to download thumbnails async.
pub(super) fn body_for_shape(rows: &[DealRow], shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::LinkedTextBlock => linked_text_block_body(rows),
        Shape::TextBlock => text_block_body(rows),
        Shape::MarkdownTextBlock => markdown_body(rows),
        Shape::Text => text_body(rows),
        Shape::Entries => entries_body(rows),
        Shape::Bars => bars_body(rows),
        Shape::Badge => badge_body(rows),
        Shape::Timeline => timeline_body(rows),
        _ => return None,
    })
}

pub(super) fn linked_text_block_body(rows: &[DealRow]) -> Body {
    Body::LinkedTextBlock(LinkedTextBlockData {
        items: rows
            .iter()
            .map(|r| LinkedLine {
                text: r.label_with_store(),
                url: Some(r.link.clone()),
            })
            .collect(),
    })
}

pub(super) fn text_block_body(rows: &[DealRow]) -> Body {
    Body::TextBlock(TextBlockData {
        lines: rows.iter().map(|r| r.label_with_store()).collect(),
    })
}

pub(super) fn markdown_body(rows: &[DealRow]) -> Body {
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: rows
            .iter()
            .map(|r| format!("- [{}]({})", r.label_with_store(), r.link))
            .collect::<Vec<_>>()
            .join("\n"),
    })
}

pub(super) fn text_body(rows: &[DealRow]) -> Body {
    let value = top_row(rows)
        .map(|r| r.label_with_store())
        .unwrap_or_default();
    Body::Text(TextData { value })
}

pub(super) fn entries_body(rows: &[DealRow]) -> Body {
    Body::Entries(EntriesData {
        items: rows
            .iter()
            .map(|r| Entry {
                key: truncate(&r.title, LABEL_MAX_CHARS),
                value: Some(entry_value(r)),
                status: Some(status_for_pct(r.discount_pct.unwrap_or(0))),
            })
            .collect(),
    })
}

pub(super) fn bars_body(rows: &[DealRow]) -> Body {
    let mut sorted: Vec<&DealRow> = rows.iter().collect();
    sorted.sort_by_key(|r| std::cmp::Reverse(r.discount_pct.unwrap_or(0)));
    Body::Bars(BarsData {
        bars: sorted
            .into_iter()
            .map(|r| Bar {
                label: truncate(&r.title, LABEL_MAX_CHARS),
                value: r.discount_pct.unwrap_or(0) as u64,
                value_label: None,
            })
            .collect(),
    })
}

pub(super) fn badge_body(rows: &[DealRow]) -> Body {
    let best = top_row(rows).and_then(|r| r.discount_pct).unwrap_or(0);
    let (status, label) = match best {
        100 => (Status::Ok, "free this week".to_string()),
        n if n >= 50 => (Status::Ok, format!("{n}% off")),
        n if n > 0 => (Status::Warn, format!("{n}% off")),
        _ => (Status::Warn, "no deals".to_string()),
    };
    Body::Badge(BadgeData { status, label })
}

pub(super) fn timeline_body(rows: &[DealRow]) -> Body {
    Body::Timeline(TimelineData {
        events: rows
            .iter()
            .map(|r| TimelineEvent {
                timestamp: r.published.map(|d| d.timestamp()).unwrap_or(0),
                title: r.title.clone(),
                detail: r.subtitle(),
                status: Some(status_for_pct(r.discount_pct.unwrap_or(0))),
            })
            .collect(),
    })
}

pub(super) async fn image_linked_body(rows: &[DealRow]) -> Body {
    let urls: Vec<Option<String>> = rows.iter().map(|r| r.image_url.clone()).collect();
    let paths = thumbnails::download_many(&urls).await;
    Body::ImageLinkedList(ImageLinkedListData {
        items: rows
            .iter()
            .zip(paths)
            .map(|(r, path)| ImageLinkedItem {
                title: r.title.clone(),
                url: Some(r.link.clone()),
                thumbnail_path: path.map(|p| p.to_string_lossy().into_owned()),
                subtitle: r.subtitle(),
            })
            .collect(),
    })
}

fn entry_value(r: &DealRow) -> String {
    match (&r.sale_price, r.discount_pct) {
        (Some(price), Some(pct)) => format!("{price} ({pct}% off)"),
        (Some(price), None) => price.clone(),
        (None, Some(pct)) => format!("{pct}% off"),
        (None, None) => "—".into(),
    }
}

fn top_row(rows: &[DealRow]) -> Option<&DealRow> {
    rows.iter().max_by_key(|r| r.discount_pct.unwrap_or(0))
}

fn status_for_pct(pct: u32) -> Status {
    if pct >= 50 { Status::Ok } else { Status::Warn }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars()
            .take(max - 1)
            .chain(std::iter::once('…'))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(title: &str, pct: Option<u32>, store: Option<&str>) -> DealRow {
        DealRow {
            title: title.into(),
            image_url: None,
            sale_price: pct.map(|_| "$4.99".into()),
            original_price: pct.map(|_| "$9.99".into()),
            discount_pct: pct,
            store: store.map(str::to_string),
            link: "https://example.com/deal".into(),
            published: None,
        }
    }

    #[test]
    fn label_omits_missing_fields() {
        let bare = row("Bare", None, None);
        assert_eq!(bare.label(), "Bare");
    }

    #[test]
    fn label_with_full_pricing_includes_all_pieces() {
        let r = row("Half Life", Some(50), None);
        assert!(r.label().contains("Half Life"));
        assert!(r.label().contains("$4.99"));
        assert!(r.label().contains("50% off"));
        assert!(r.label().contains("$9.99"));
    }

    #[test]
    fn label_with_store_prefixes_when_known() {
        let r = row("X", Some(40), Some("Steam"));
        assert!(r.label_with_store().starts_with("[Steam]"));
    }

    #[test]
    fn entries_body_carries_discount_status() {
        let rows = vec![row("A", Some(80), Some("Steam")), row("B", Some(20), None)];
        let Body::Entries(data) = entries_body(&rows) else {
            panic!("expected entries");
        };
        assert_eq!(data.items[0].status, Some(Status::Ok));
        assert_eq!(data.items[1].status, Some(Status::Warn));
    }

    #[test]
    fn bars_body_sorts_descending_by_discount() {
        let rows = vec![row("low", Some(20), None), row("high", Some(80), None)];
        let Body::Bars(data) = bars_body(&rows) else {
            panic!("expected bars");
        };
        assert_eq!(data.bars[0].label, "high");
        assert_eq!(data.bars[0].value, 80);
    }

    #[test]
    fn badge_flags_free_with_dedicated_label() {
        let rows = vec![row("g", Some(100), None)];
        let Body::Badge(data) = badge_body(&rows) else {
            panic!("expected badge");
        };
        assert_eq!(data.status, Status::Ok);
        assert!(data.label.contains("free"));
    }

    #[test]
    fn badge_warns_when_only_small_discounts_present() {
        let rows = vec![row("g", Some(10), None)];
        let Body::Badge(data) = badge_body(&rows) else {
            panic!("expected badge");
        };
        assert_eq!(data.status, Status::Warn);
    }

    #[test]
    fn badge_empty_rows_reports_no_deals() {
        let Body::Badge(data) = badge_body(&[]) else {
            panic!("expected badge");
        };
        assert!(data.label.contains("no deals"));
    }

    #[test]
    fn text_body_picks_largest_discount_as_headline() {
        let rows = vec![row("small", Some(10), None), row("big", Some(70), None)];
        let Body::Text(data) = text_body(&rows) else {
            panic!("expected text");
        };
        assert!(data.value.contains("big"));
    }

    #[test]
    fn timeline_uses_publish_timestamp() {
        let mut r = row("g", Some(50), Some("Epic"));
        r.published = Some(chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 5, 1, 12, 0, 0).unwrap());
        let Body::Timeline(data) = timeline_body(&[r]) else {
            panic!("expected timeline");
        };
        assert!(data.events[0].timestamp > 0);
        assert!(data.events[0].detail.as_ref().unwrap().contains("Epic"));
    }

    #[test]
    fn markdown_body_emits_bullet_list_with_links() {
        let r = row("g", Some(60), None);
        let Body::MarkdownTextBlock(data) = markdown_body(&[r]) else {
            panic!("expected markdown");
        };
        assert!(data.value.starts_with("- ["));
        assert!(data.value.contains("(https://example.com/deal)"));
    }

    #[test]
    fn truncate_appends_ellipsis_when_over_limit() {
        let long = "x".repeat(LABEL_MAX_CHARS + 5);
        let t = truncate(&long, LABEL_MAX_CHARS);
        assert_eq!(t.chars().count(), LABEL_MAX_CHARS);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn body_for_shape_returns_none_for_unsupported_shapes() {
        assert!(body_for_shape(&[], Shape::Heatmap).is_none());
        assert!(body_for_shape(&[], Shape::Calendar).is_none());
        assert!(body_for_shape(&[], Shape::Ratio).is_none());
    }
}
