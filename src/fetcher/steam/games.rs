//! Shared shape body builders for the `steam_recently_played` / `steam_owned_games` /
//! `steam_charts` siblings.
//!
//! All three surface a ranked list of games where each row carries an appid, a display name,
//! a numeric value (minutes played / concurrent players), and a store URL derived from the
//! appid. Normalising into [`GameRow`] lets every sibling stay around the catalog's
//! ~100-lines-per-fetcher guideline.

use crate::fetcher::steam::common::{format_count, header_image_url, players_word, store_url};
use crate::fetcher::thumbnails;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, ImageData, ImageLinkedItem,
    ImageLinkedListData, LinkedLine, LinkedTextBlockData, MarkdownTextBlockData, NumberSeriesData,
    RatioData, Status, TextBlockData, TextData,
};

/// Normalised view of one game in a ranked list. `value` carries minutes for playtime-flavoured
/// fetchers (recently-played, owned-games) and concurrent-player count for the chart fetcher.
/// `value_label` lets each fetcher format the right side of the row appropriately ("80h",
/// "1.2k players") without leaking format strings into the helpers.
#[derive(Debug, Clone)]
pub struct GameRow {
    pub rank: usize,
    pub appid: u32,
    pub name: String,
    pub value: u64,
    pub value_label: String,
}

impl GameRow {
    pub fn store_url(&self) -> String {
        store_url(self.appid)
    }

    pub fn header_image(&self) -> String {
        header_image_url(self.appid)
    }
}

pub fn headline(rows: &[GameRow], empty_label: &str) -> String {
    match rows.first() {
        Some(top) => format!("#{} {}  ({})", top.rank, top.name, top.value_label),
        None => empty_label.into(),
    }
}

pub fn text_body(rows: &[GameRow], empty_label: &str) -> Body {
    Body::Text(TextData {
        value: headline(rows, empty_label),
    })
}

pub fn text_block_body(rows: &[GameRow], empty_label: &str) -> Body {
    Body::TextBlock(TextBlockData {
        lines: lines(rows, empty_label),
    })
}

fn lines(rows: &[GameRow], empty_label: &str) -> Vec<String> {
    if rows.is_empty() {
        return vec![empty_label.into()];
    }
    rows.iter()
        .map(|r| format!("#{} {}  {}", r.rank, r.name, r.value_label))
        .collect()
}

pub fn markdown_body(rows: &[GameRow], empty_label: &str) -> Body {
    let value = if rows.is_empty() {
        format!("_{empty_label}_")
    } else {
        rows.iter()
            .map(|r| format!("- **#{} {}** — {}", r.rank, r.name, r.value_label))
            .collect::<Vec<_>>()
            .join("\n")
    };
    Body::MarkdownTextBlock(MarkdownTextBlockData { value })
}

pub fn linked_text_body(rows: &[GameRow], empty_label: &str) -> Body {
    let items: Vec<LinkedLine> = if rows.is_empty() {
        vec![LinkedLine {
            text: empty_label.into(),
            url: None,
        }]
    } else {
        rows.iter()
            .map(|r| LinkedLine {
                text: format!("#{} {}  {}", r.rank, r.name, r.value_label),
                url: Some(r.store_url()),
            })
            .collect()
    };
    Body::LinkedTextBlock(LinkedTextBlockData { items })
}

pub async fn image_linked_body(rows: &[GameRow]) -> Body {
    let urls: Vec<Option<String>> = rows.iter().map(|r| Some(r.header_image())).collect();
    let paths = thumbnails::download_many(&urls).await;
    let items = rows
        .iter()
        .zip(paths)
        .map(|(r, path)| ImageLinkedItem {
            title: format!("#{} {}", r.rank, r.name),
            url: Some(r.store_url()),
            thumbnail_path: path.map(|p| p.to_string_lossy().into_owned()),
            subtitle: Some(r.value_label.clone()),
        })
        .collect();
    Body::ImageLinkedList(ImageLinkedListData { items })
}

pub fn entries_body(rows: &[GameRow], empty_label: &str) -> Body {
    let items: Vec<Entry> = if rows.is_empty() {
        vec![Entry {
            key: "—".into(),
            value: Some(empty_label.into()),
            status: None,
        }]
    } else {
        rows.iter()
            .map(|r| Entry {
                key: format!("#{} {}", r.rank, r.name),
                value: Some(r.value_label.clone()),
                status: None,
            })
            .collect()
    };
    Body::Entries(EntriesData { items })
}

/// Top game's share of the total value across the surfaced window. `denominator` carries the
/// summed value so a gauge can spell "80h of 142h" if it wants.
pub fn ratio_body(rows: &[GameRow]) -> Body {
    let total: u64 = rows.iter().map(|r| r.value).sum();
    let (value, label, denominator) = match rows.first() {
        Some(top) if total > 0 => (
            top.value as f64 / total as f64,
            Some(top.name.clone()),
            Some(total),
        ),
        _ => (0.0, None, None),
    };
    Body::Ratio(RatioData {
        value,
        label,
        denominator,
    })
}

pub fn number_series_body(rows: &[GameRow]) -> Body {
    Body::NumberSeries(NumberSeriesData {
        values: rows.iter().map(|r| r.value).collect(),
    })
}

pub fn bars_body(rows: &[GameRow]) -> Body {
    Body::Bars(BarsData {
        bars: rows
            .iter()
            .map(|r| Bar {
                label: r.name.clone(),
                value: r.value,
                // Carry the fetcher's formatted unit so `list_ranking` shows `"80h"` or
                // `"2024-07-03"` in the value column instead of the raw `u64`.
                value_label: Some(r.value_label.clone()),
            })
            .collect(),
    })
}

/// Top game's header image as a standalone Image body — the value-add is the cover art, not
/// a list. Falls back to an empty path when the list is empty so the empty-state placeholder
/// kicks in at `render_payload`.
pub async fn image_body(rows: &[GameRow]) -> Body {
    let path = match rows.first() {
        Some(top) => thumbnails::download_to_cache(&top.header_image())
            .await
            .ok()
            .flatten()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        None => String::new(),
    };
    Body::Image(ImageData { path })
}

/// Promotes the top game to the badge label; falls back to a quiet-window pill so the widget
/// stays visible when the user has no recent playtime / the chart is empty. `unit_label`
/// distinguishes "Xh this week" (playtime fetchers) from "1.2k players" (chart fetcher) — the
/// caller picks the unit the top row's value is denominated in.
pub fn badge_body(rows: &[GameRow], unit_label: &str, empty_label: &str) -> Body {
    let (status, label) = match rows.first() {
        Some(top) => (Status::Ok, format!("#1 {} · {}", top.name, top.value_label)),
        None => (Status::Warn, format!("{empty_label} ({unit_label})")),
    };
    Body::Badge(BadgeData { status, label })
}

/// `"42 players"` (singular when applicable) — exposed so the chart fetcher can build a
/// `value_label` without re-deriving the pluralisation rule.
pub fn players_label(n: u64) -> String {
    format!("{} {}", format_count(n), players_word(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(rank: usize, appid: u32, name: &str, value: u64, label: &str) -> GameRow {
        GameRow {
            rank,
            appid,
            name: name.into(),
            value,
            value_label: label.into(),
        }
    }

    #[test]
    fn store_url_and_header_image_are_appid_derived() {
        let r = row(1, 730, "CS2", 0, "");
        assert!(r.store_url().contains("/app/730/"));
        assert!(r.header_image().contains("/730/header.jpg"));
    }

    #[test]
    fn headline_reports_top_row_with_rank_prefix() {
        let rows = vec![
            row(1, 730, "CS2", 60, "60h"),
            row(2, 570, "Dota 2", 30, "30h"),
        ];
        assert_eq!(headline(&rows, "empty"), "#1 CS2  (60h)");
    }

    #[test]
    fn headline_falls_back_to_empty_label() {
        assert_eq!(headline(&[], "quiet"), "quiet");
    }

    #[test]
    fn linked_text_body_carries_store_url_per_row() {
        let rows = vec![row(1, 730, "CS2", 60, "60h")];
        let Body::LinkedTextBlock(b) = linked_text_body(&rows, "empty") else {
            panic!("expected linked_text_block");
        };
        assert_eq!(b.items.len(), 1);
        assert!(b.items[0].url.as_deref().unwrap().contains("/app/730/"));
        assert!(b.items[0].text.contains("CS2"));
    }

    #[test]
    fn linked_text_body_emits_unlinked_empty_row_on_empty_input() {
        let Body::LinkedTextBlock(b) = linked_text_body(&[], "empty") else {
            panic!("expected linked_text_block");
        };
        assert_eq!(b.items.len(), 1);
        assert!(b.items[0].url.is_none());
    }

    #[test]
    fn ratio_body_is_zero_when_rows_are_empty() {
        let Body::Ratio(r) = ratio_body(&[]) else {
            panic!("expected ratio");
        };
        assert_eq!(r.value, 0.0);
        assert!(r.label.is_none());
        assert!(r.denominator.is_none());
    }

    #[test]
    fn ratio_body_carries_top_share_and_total_as_denominator() {
        let rows = vec![
            row(1, 1, "A", 60, "60"),
            row(2, 2, "B", 30, "30"),
            row(3, 3, "C", 10, "10"),
        ];
        let Body::Ratio(r) = ratio_body(&rows) else {
            panic!("expected ratio");
        };
        assert!((r.value - 0.6).abs() < 1e-9);
        assert_eq!(r.label.as_deref(), Some("A"));
        assert_eq!(r.denominator, Some(100));
    }

    #[test]
    fn ratio_body_returns_zero_when_total_is_zero() {
        let rows = vec![row(1, 1, "A", 0, "0"), row(2, 2, "B", 0, "0")];
        let Body::Ratio(r) = ratio_body(&rows) else {
            panic!("expected ratio");
        };
        assert_eq!(r.value, 0.0);
        assert!(r.label.is_none());
    }

    #[test]
    fn number_series_lifts_values_in_order() {
        let rows = vec![row(1, 1, "A", 30, "30"), row(2, 2, "B", 20, "20")];
        let Body::NumberSeries(s) = number_series_body(&rows) else {
            panic!("expected number_series");
        };
        assert_eq!(s.values, vec![30, 20]);
    }

    #[test]
    fn bars_body_uses_name_as_label_and_value_as_height() {
        let rows = vec![row(1, 1, "CS2", 60, "60h")];
        let Body::Bars(b) = bars_body(&rows) else {
            panic!("expected bars");
        };
        assert_eq!(b.bars[0].label, "CS2");
        assert_eq!(b.bars[0].value, 60);
    }

    #[test]
    fn entries_body_uses_value_label_column() {
        let rows = vec![row(1, 1, "CS2", 60, "60h")];
        let Body::Entries(e) = entries_body(&rows, "empty") else {
            panic!("expected entries");
        };
        assert_eq!(e.items[0].key, "#1 CS2");
        assert_eq!(e.items[0].value.as_deref(), Some("60h"));
    }

    #[test]
    fn badge_body_promotes_top_row_and_warns_on_empty() {
        let rows = vec![row(1, 1, "CS2", 60, "60h")];
        let Body::Badge(b) = badge_body(&rows, "this week", "no playtime") else {
            panic!("expected badge");
        };
        assert_eq!(b.status, Status::Ok);
        assert!(b.label.contains("CS2"));
        assert!(b.label.contains("60h"));

        let Body::Badge(empty) = badge_body(&[], "this week", "no playtime") else {
            panic!("expected badge");
        };
        assert_eq!(empty.status, Status::Warn);
        assert!(empty.label.contains("this week"));
    }

    #[test]
    fn players_label_pluralises_only_for_non_singular() {
        assert_eq!(players_label(0), "0 players");
        assert_eq!(players_label(1), "1 player");
        assert_eq!(players_label(1500), "1.5k players");
    }

    #[test]
    fn markdown_body_italicises_empty_label() {
        let Body::MarkdownTextBlock(m) = markdown_body(&[], "quiet") else {
            panic!("expected markdown");
        };
        assert_eq!(m.value, "_quiet_");
    }
}
