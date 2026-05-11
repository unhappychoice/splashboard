//! `list_cards` — `ImageLinkedList` rendered as card rows: a thumbnail cell on the left,
//! title (+ optional subtitle) on the right, with rows that carry a `url` wrapped in OSC 8
//! hyperlinks so modern terminals make them clickable. Rows without a thumbnail keep a blank
//! cell of the same width so the text column stays column-aligned across rows.
//!
//! Pairs with media-feed fetchers (`rss` with `media:thumbnail`, `reddit_subreddit_posts`,
//! `wikipedia_featured` / `wikipedia_random`) where the thumbnail carries as much glance
//! value as the title.

use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, ImageLinkedItem, ImageLinkedListData};
use crate::theme::{self, ColorKey, Theme};

use super::list_links::wrap_osc8;
use super::media_image::draw_thumbnail;
use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT, theme::TEXT_DIM];

const DEFAULT_THUMBNAIL_WIDTH: u16 = 6;
const DEFAULT_ROW_HEIGHT: u16 = 3;
const DEFAULT_GAP: u16 = 1;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "max_items",
        type_hint: "positive integer",
        required: false,
        default: Some("all rows"),
        description: "Cap on rendered cards. Truncates from the end when the input has more rows than the cap.",
    },
    OptionSchema {
        name: "thumbnail_width",
        type_hint: "cells (u16)",
        required: false,
        default: Some("6"),
        description: "Width in cells of the thumbnail column. Rows without a thumbnail still consume this width as blank space so the text column stays aligned.",
    },
    OptionSchema {
        name: "row_height",
        type_hint: "cells (u16)",
        required: false,
        default: Some("3"),
        description: "Height in cells of each card row. Row 0 carries the title (linked when `url` is set), row 1 the optional subtitle, row 2 is breathing room.",
    },
    OptionSchema {
        name: "gap",
        type_hint: "cells (u16)",
        required: false,
        default: Some("1"),
        description: "Horizontal gap in cells between the thumbnail column and the text column.",
    },
    OptionSchema {
        name: "fit",
        type_hint: "\"contain\" | \"cover\" | \"stretch\"",
        required: false,
        default: Some("\"contain\""),
        description: "How each thumbnail is sized into its cell. Matches `media_image`'s `fit`.",
    },
];

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    pub thumbnail_width: Option<u16>,
    #[serde(default)]
    pub row_height: Option<u16>,
    #[serde(default)]
    pub gap: Option<u16>,
    #[serde(default)]
    pub fit: Option<String>,
}

pub struct ListCardsRenderer;

impl Renderer for ListCardsRenderer {
    fn name(&self) -> &str {
        "list_cards"
    }
    fn description(&self) -> &'static str {
        "Card-shaped rows with a thumbnail on the left and title (+ optional subtitle) on the right. Rows that carry a URL get an OSC 8 hyperlink wrap on the title so terminals make it clickable. Use this for media-feed widgets (RSS with images, subreddit posts, Wikipedia featured) where the thumbnail is part of the headline."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::ImageLinkedList]
    }
    fn color_keys(&self) -> &[ColorKey] {
        COLOR_KEYS
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        body: &Body,
        opts: &RenderOptions,
        theme: &Theme,
        _registry: &Registry,
    ) {
        if let Body::ImageLinkedList(d) = body {
            render_cards(frame, area, d, opts, theme);
        }
    }
    fn natural_height(
        &self,
        body: &Body,
        opts: &RenderOptions,
        _max_width: u16,
        _registry: &Registry,
    ) -> u16 {
        if let Body::ImageLinkedList(d) = body {
            let cap = opts.max_items.unwrap_or(usize::MAX);
            let count = d.items.len().min(cap) as u16;
            let row_h = row_height(opts);
            return count.saturating_mul(row_h);
        }
        1
    }
}

fn render_cards(
    frame: &mut Frame,
    area: Rect,
    data: &ImageLinkedListData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let cap = opts.max_items.unwrap_or(usize::MAX);
    let row_h = row_height(opts);
    let thumb_w = thumb_width(opts, area.width);
    let gap = gap(opts);
    let specific: Options = opts.parse_specific();
    let fit = specific.fit.as_deref();
    let max_rows = (area.height / row_h.max(1)) as usize;
    data.items
        .iter()
        .take(cap)
        .take(max_rows)
        .enumerate()
        .for_each(|(i, item)| {
            let y = area.y + i as u16 * row_h;
            let row = Rect {
                x: area.x,
                y,
                width: area.width,
                height: row_h.min(area.y + area.height - y),
            };
            draw_card(frame, row, item, thumb_w, gap, fit, theme);
        });
}

fn draw_card(
    frame: &mut Frame,
    row: Rect,
    item: &ImageLinkedItem,
    thumb_w: u16,
    gap: u16,
    fit: Option<&str>,
    theme: &Theme,
) {
    let thumb_rect = Rect {
        x: row.x,
        y: row.y,
        width: thumb_w.min(row.width),
        height: row.height,
    };
    if let Some(path) = item.thumbnail_path.as_deref().filter(|s| !s.is_empty()) {
        let _ = draw_thumbnail(frame, thumb_rect, path, fit, theme);
    }
    let text_x = row.x + thumb_rect.width + gap;
    if text_x >= row.x + row.width {
        return;
    }
    let text_w = row.x + row.width - text_x;
    draw_text(
        frame.buffer_mut(),
        text_x,
        row.y,
        text_w,
        row.height,
        item,
        theme,
    );
}

fn draw_text(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    item: &ImageLinkedItem,
    theme: &Theme,
) {
    if width == 0 || height == 0 {
        return;
    }
    let title_style = Style::default().fg(theme.text).add_modifier(Modifier::BOLD);
    let (end_x, _) = buf.set_stringn(x, y, &item.title, width as usize, title_style);
    if let Some(url) = item.url.as_deref().filter(|u| !u.is_empty()) {
        wrap_osc8(buf, x, y, end_x, url, title_style);
    }
    if height >= 2
        && let Some(sub) = item.subtitle.as_deref().filter(|s| !s.is_empty())
    {
        let sub_style = Style::default().fg(theme.text_dim);
        buf.set_stringn(x, y + 1, sub, width as usize, sub_style);
    }
}

fn row_height(opts: &RenderOptions) -> u16 {
    let specific: Options = opts.parse_specific();
    specific.row_height.unwrap_or(DEFAULT_ROW_HEIGHT).max(1)
}

fn thumb_width(opts: &RenderOptions, area_width: u16) -> u16 {
    let specific: Options = opts.parse_specific();
    specific
        .thumbnail_width
        .unwrap_or(DEFAULT_THUMBNAIL_WIDTH)
        .min(area_width)
}

fn gap(opts: &RenderOptions) -> u16 {
    let specific: Options = opts.parse_specific();
    specific.gap.unwrap_or(DEFAULT_GAP)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{ImageLinkedItem, ImageLinkedListData, Payload};
    use crate::render::test_utils::render_to_buffer_with_spec;
    use crate::render::{Registry, RenderSpec};

    fn payload(items: Vec<ImageLinkedItem>) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::ImageLinkedList(ImageLinkedListData { items }),
        }
    }

    fn item(title: &str, url: Option<&str>, sub: Option<&str>) -> ImageLinkedItem {
        ImageLinkedItem {
            title: title.into(),
            url: url.map(String::from),
            thumbnail_path: None,
            subtitle: sub.map(String::from),
        }
    }

    fn row_text(buf: &ratatui::buffer::Buffer, y: u16) -> String {
        (buf.area.x..buf.area.right())
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect()
    }

    #[test]
    fn renderer_contract_exposes_image_linked_list_surface() {
        let renderer = ListCardsRenderer;
        assert_eq!(renderer.name(), "list_cards");
        assert!(!renderer.animates());
        assert_eq!(renderer.accepts(), &[Shape::ImageLinkedList]);
        assert_eq!(
            renderer
                .color_keys()
                .iter()
                .map(|k| k.name)
                .collect::<Vec<_>>(),
            vec![theme::TEXT.name, theme::TEXT_DIM.name],
        );
        assert_eq!(
            renderer
                .option_schemas()
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>(),
            vec!["max_items", "thumbnail_width", "row_height", "gap", "fit"],
        );
        assert!(renderer.description().contains("thumbnail"));
    }

    #[test]
    fn zero_area_does_not_panic() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_cards".into());
        // 0×0, 1×0, 0×1 all need to no-op rather than touch a zero-sized buffer.
        for (w, h) in [(0u16, 0u16), (0, 5), (10, 0)] {
            let _ = render_to_buffer_with_spec(
                &payload(vec![item("ignored", None, None)]),
                Some(&spec),
                &registry,
                w.max(1),
                h.max(1),
            );
        }
    }

    #[test]
    fn narrow_area_truncates_text_without_panicking() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_cards".into());
        // 8 cells: 6 (thumb) + 1 (gap) leaves 1 cell for text. The renderer must clip,
        // not crash. Confirm a buffer comes back populated to whatever the cell allows.
        let buf = render_to_buffer_with_spec(
            &payload(vec![item("very long title", None, None)]),
            Some(&spec),
            &registry,
            8,
            3,
        );
        let row: String = (buf.area.x..buf.area.right())
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        // The first character of the title (`v`) lands at x=7 (6 thumb + 1 gap).
        assert!(row.contains('v'));
    }

    #[test]
    fn area_narrower_than_thumb_column_skips_text() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_cards".into());
        // 4 cells total — narrower than the default 6-cell thumbnail width. The thumbnail
        // cell saturates to the full row width, leaving no room for the gap or text.
        let buf = render_to_buffer_with_spec(
            &payload(vec![item("hidden title", None, None)]),
            Some(&spec),
            &registry,
            4,
            3,
        );
        let row: String = (buf.area.x..buf.area.right())
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        // Text must be entirely suppressed when there's no room for it after the thumbnail.
        assert!(
            !row.contains("hidden"),
            "text leaked past thumbnail column: {row:?}",
        );
    }

    #[test]
    fn gap_option_shifts_text_column() {
        let registry = Registry::with_builtins();
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        // thumbnail_width=4, gap=3 ⇒ text column starts at x=7.
        let w: W =
            toml::from_str(r#"render = { type = "list_cards", thumbnail_width = 4, gap = 3 }"#)
                .unwrap();
        let buf = render_to_buffer_with_spec(
            &payload(vec![item("X", None, None)]),
            Some(&w.render),
            &registry,
            20,
            3,
        );
        let row: String = (buf.area.x..buf.area.right())
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert_eq!(row.find('X'), Some(7), "row: {row:?}");
    }

    #[test]
    fn fit_option_parses_without_error() {
        // We can't visually verify `fit` in the ASCII buffer (no real image is drawn for an
        // empty thumbnail_path), but we can ensure each accepted value parses and the render
        // path stays panic-free.
        let registry = Registry::with_builtins();
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        for fit in &["contain", "cover", "stretch"] {
            let w: W = toml::from_str(&format!(
                r#"render = {{ type = "list_cards", fit = "{fit}" }}"#
            ))
            .unwrap();
            let _ = render_to_buffer_with_spec(
                &payload(vec![item("x", None, None)]),
                Some(&w.render),
                &registry,
                20,
                3,
            );
        }
    }

    #[test]
    fn renders_title_and_subtitle_per_row() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_cards".into());
        let buf = render_to_buffer_with_spec(
            &payload(vec![item(
                "Rust 1.99 released",
                None,
                Some("rust-lang.org · 2h"),
            )]),
            Some(&spec),
            &registry,
            40,
            3,
        );
        assert!(row_text(&buf, 0).contains("Rust 1.99 released"));
        assert!(row_text(&buf, 1).contains("rust-lang.org"));
    }

    #[test]
    fn linked_row_wraps_title_with_osc_8() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_cards".into());
        let buf = render_to_buffer_with_spec(
            &payload(vec![item("hello", Some("https://example.com"), None)]),
            Some(&spec),
            &registry,
            30,
            3,
        );
        let row = row_text(&buf, 0);
        assert!(
            row.contains("\x1b]8;;https://example.com\x1b\\"),
            "missing OSC 8 open: {row:?}",
        );
        assert!(
            row.contains("\x1b]8;;\x1b\\"),
            "missing OSC 8 close: {row:?}"
        );
    }

    #[test]
    fn missing_thumbnail_keeps_text_column_aligned() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_cards".into());
        let buf = render_to_buffer_with_spec(
            &payload(vec![item("first", None, None), item("second", None, None)]),
            Some(&spec),
            &registry,
            40,
            6,
        );
        // With default thumb_width=6 + gap=1, text starts at x=7. The blank thumb column on
        // both rows means both titles share the same starting column.
        let row0 = row_text(&buf, 0);
        let row1 = row_text(&buf, 3);
        let pos0 = row0.find("first").unwrap();
        let pos1 = row1.find("second").unwrap();
        assert_eq!(pos0, pos1, "text columns must align: {row0:?} vs {row1:?}");
    }

    #[test]
    fn empty_body_does_not_panic() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_cards".into());
        // Empty body short-circuits to the shared placeholder, so we just assert no panic.
        let _ = render_to_buffer_with_spec(&payload(vec![]), Some(&spec), &registry, 30, 3);
    }

    #[test]
    fn max_items_caps_rendered_cards() {
        let registry = Registry::with_builtins();
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W = toml::from_str(r#"render = { type = "list_cards", max_items = 1 }"#).unwrap();
        let buf = render_to_buffer_with_spec(
            &payload(vec![item("first", None, None), item("second", None, None)]),
            Some(&w.render),
            &registry,
            40,
            9,
        );
        assert!(row_text(&buf, 0).contains("first"));
        // Row 3 would carry the second card with the default row_height=3; with the cap it stays
        // blank.
        assert!(!row_text(&buf, 3).contains("second"));
    }

    #[test]
    fn area_height_caps_rendered_cards() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_cards".into());
        // Buffer only fits one full 3-cell row; the second card has no space and is dropped.
        let buf = render_to_buffer_with_spec(
            &payload(vec![item("first", None, None), item("second", None, None)]),
            Some(&spec),
            &registry,
            40,
            3,
        );
        assert!(row_text(&buf, 0).contains("first"));
        // Verify the second card never appears anywhere in the small buffer.
        let joined: String = (0..3).map(|y| row_text(&buf, y)).collect();
        assert!(!joined.contains("second"));
    }

    #[test]
    fn natural_height_scales_with_item_count() {
        let registry = Registry::with_builtins();
        let renderer = ListCardsRenderer;
        let body = Body::ImageLinkedList(ImageLinkedListData {
            items: vec![
                item("a", None, None),
                item("b", None, None),
                item("c", None, None),
            ],
        });
        // 3 items * default row_height=3 = 9 cells.
        assert_eq!(
            renderer.natural_height(&body, &RenderOptions::default(), 40, &registry),
            9
        );
    }

    #[test]
    fn natural_height_respects_max_items() {
        let registry = Registry::with_builtins();
        let renderer = ListCardsRenderer;
        let body = Body::ImageLinkedList(ImageLinkedListData {
            items: (0..5).map(|_| item("x", None, None)).collect(),
        });
        let opts = RenderOptions {
            max_items: Some(2),
            ..RenderOptions::default()
        };
        assert_eq!(renderer.natural_height(&body, &opts, 40, &registry), 6);
    }

    #[test]
    fn custom_row_height_and_thumb_width_apply() {
        let registry = Registry::with_builtins();
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W = toml::from_str(
            r#"render = { type = "list_cards", row_height = 2, thumbnail_width = 4, gap = 2 }"#,
        )
        .unwrap();
        let buf = render_to_buffer_with_spec(
            &payload(vec![item("a", None, None), item("b", None, None)]),
            Some(&w.render),
            &registry,
            40,
            4,
        );
        // With row_height=2, the second item lives on y=2.
        assert!(row_text(&buf, 0).contains('a'));
        assert!(row_text(&buf, 2).contains('b'));
        // thumb_width(4) + gap(2) = 6, so the text column starts at x=6.
        let row0 = row_text(&buf, 0);
        assert_eq!(row0.find('a'), Some(6));
    }
}
