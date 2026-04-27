//! `list_links` — `LinkedTextBlock` rendered one row per line, wrapped in OSC 8 hyperlinks where
//! the row carries a `url`. Rows without a url render as plain text.
//!
//! How the OSC 8 plumbing works: ratatui's `Buffer` stores per-cell symbol strings that the
//! crossterm backend prints verbatim. We collapse the whole linked row into the first cell's
//! symbol — `ESC ] 8 ; ; URL ESC \\ <text> ESC ] 8 ; ; ESC \\` — and mark the trailing cells with
//! `skip = true` so ratatui's diff doesn't try to redraw them. That keeps the OSC 8 sequence
//! atomic in the byte stream (no `MoveTo` interruption), and dodges `Buffer::diff`'s width
//! calculation, which would otherwise count the printable characters inside the escape sequence
//! as visible columns and incorrectly skip the cells immediately after the link.
//!
//! The `skip = true` trick only buys real-terminal correctness: `TestBackend` doesn't
//! simulate the terminal's char-rendering of the OSC 8 visible portion, so the skipped cells
//! stay at their default empty state in its buffer. The runtime's off-screen scrollback
//! capture (`render_full_into_buffer`) targets `TestBackend` and would inherit that broken
//! state — we wrap that draw in [`without_osc8`] so the renderer emits plain text for the
//! capture, while the on-screen `CrosstermBackend` pass keeps the OSC 8 wrap.

use std::cell::Cell as StdCell;

use ratatui::{Frame, buffer::Buffer, layout::Rect, style::Style};

use crate::options::OptionSchema;
use crate::payload::{Body, LinkedLine, LinkedTextBlockData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "max_items",
    type_hint: "positive integer",
    required: false,
    default: Some("all rows"),
    description: "Cap on rendered rows. Truncates from the end when the input has more rows than the cap.",
}];

/// Renders a `LinkedTextBlock` body one row per line, wrapping rows whose `url` is `Some(_)` in
/// an OSC 8 hyperlink so modern terminals (iTerm2, kitty, WezTerm, recent Windows Terminal, etc.)
/// surface the row as a clickable link. Rows without a url render as plain text. Pairs with feed
/// fetchers (`hackernews_top`, `github_my_prs`, `github_recent_releases`, …) where each row has a
/// canonical "open this" target.
pub struct ListLinksRenderer;

impl Renderer for ListLinksRenderer {
    fn name(&self) -> &str {
        "list_links"
    }
    fn description(&self) -> &'static str {
        "One row per entry, with rows that carry a URL wrapped in OSC-8 hyperlinks so modern terminals make them clickable. Use this for feed-style payloads (HN headlines, recent PRs, releases) where each row has a canonical destination."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::LinkedTextBlock]
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
        if let Body::LinkedTextBlock(d) = body {
            render_links(frame, area, d, opts, theme);
        }
    }
}

fn render_links(
    frame: &mut Frame,
    area: Rect,
    data: &LinkedTextBlockData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let cap = opts.max_items.unwrap_or(usize::MAX);
    let style = Style::default().fg(theme.text);
    let buf = frame.buffer_mut();
    data.items
        .iter()
        .take(cap)
        .take(area.height as usize)
        .enumerate()
        .for_each(|(i, line)| {
            let y = area.y + i as u16;
            write_row(buf, area.x, y, area.width, line, style);
        });
}

fn write_row(buf: &mut Buffer, x: u16, y: u16, max_width: u16, line: &LinkedLine, style: Style) {
    let (end_x, _) = buf.set_stringn(x, y, &line.text, max_width as usize, style);
    let drawn = end_x.saturating_sub(x);
    if drawn == 0 {
        return;
    }
    if osc8_suppressed() {
        return;
    }
    if let Some(url) = line.url.as_deref() {
        wrap_link(buf, x, y, end_x, url, style);
    }
}

thread_local! {
    /// Suppresses the OSC 8 wrap inside the closure passed to [`without_osc8`]. The runtime's
    /// off-screen capture path (`render_full_into_buffer`) targets `TestBackend`, which holds
    /// raw cell symbols verbatim and doesn't interpret escape sequences — putting the OSC 8
    /// wrap there would only pollute the buffer that later gets sliced into the inline
    /// scrollback. The on-screen `CrosstermBackend` pass runs without this flag so user-facing
    /// frames keep the clickable hyperlinks.
    static OSC8_SUPPRESSED: StdCell<bool> = const { StdCell::new(false) };
}

/// Run `f` with OSC 8 hyperlink wrapping disabled. Idempotent and re-entrant: nested calls
/// stack the suppression and restore the prior value on exit.
pub fn without_osc8<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let prior = OSC8_SUPPRESSED.with(|cell| cell.replace(true));
    let result = f();
    OSC8_SUPPRESSED.with(|cell| cell.set(prior));
    result
}

fn osc8_suppressed() -> bool {
    OSC8_SUPPRESSED.with(StdCell::get)
}

fn wrap_link(buf: &mut Buffer, x: u16, y: u16, end_x: u16, url: &str, style: Style) {
    let visible: String = (x..end_x)
        .filter_map(|col| buf.cell((col, y)).map(|c| c.symbol().to_string()))
        .collect();
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_symbol(&format!("\x1b]8;;{url}\x1b\\{visible}\x1b]8;;\x1b\\"));
        cell.set_style(style);
    }
    for col in (x + 1)..end_x {
        if let Some(cell) = buf.cell_mut((col, y)) {
            cell.set_skip(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{LinkedLine, LinkedTextBlockData, Payload};
    use crate::render::test_utils::render_to_buffer_with_spec;
    use crate::render::{Registry, RenderSpec};

    fn payload(items: Vec<LinkedLine>) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::LinkedTextBlock(LinkedTextBlockData { items }),
        }
    }

    fn line(text: &str, url: Option<&str>) -> LinkedLine {
        LinkedLine {
            text: text.into(),
            url: url.map(String::from),
        }
    }

    fn cells_concat(buf: &ratatui::buffer::Buffer, y: u16) -> String {
        (buf.area.x..buf.area.right())
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect()
    }

    #[test]
    fn renders_unlinked_rows_as_plain_text() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_links".into());
        let buf = render_to_buffer_with_spec(
            &payload(vec![line("alpha 1pt", None)]),
            Some(&spec),
            &registry,
            30,
            2,
        );
        let row = cells_concat(&buf, 0);
        assert!(row.contains("alpha 1pt"), "row: {row:?}");
        assert!(!row.contains("\x1b]8;;"), "should not embed OSC 8: {row:?}");
    }

    #[test]
    fn linked_row_wraps_first_cell_with_full_osc_8() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_links".into());
        let buf = render_to_buffer_with_spec(
            &payload(vec![line("hello", Some("https://example.com"))]),
            Some(&spec),
            &registry,
            10,
            1,
        );
        let row = cells_concat(&buf, 0);
        assert!(
            row.starts_with("\x1b]8;;https://example.com\x1b\\hello\x1b]8;;\x1b\\"),
            "expected full OSC 8 wrap in cell 0: {row:?}",
        );
    }

    #[test]
    fn empty_body_does_not_panic() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_links".into());
        let buf = render_to_buffer_with_spec(&payload(vec![]), Some(&spec), &registry, 10, 2);
        // Empty body short-circuits to the shared placeholder, so we just assert no panic.
        let _ = cells_concat(&buf, 0);
    }

    #[test]
    fn narrow_area_truncates_without_breaking_link_close() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_links".into());
        let buf = render_to_buffer_with_spec(
            &payload(vec![line(
                "very long title here",
                Some("https://example.com"),
            )]),
            Some(&spec),
            &registry,
            8,
            1,
        );
        let row = cells_concat(&buf, 0);
        assert!(row.starts_with("\x1b]8;;"), "open missing: {row:?}");
        assert!(row.contains("\x1b]8;;\x1b\\"), "close missing: {row:?}");
    }

    #[test]
    fn max_items_caps_render() {
        let registry = Registry::with_builtins();
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W = toml::from_str(r#"render = { type = "list_links", max_items = 1 }"#).unwrap();
        let buf = render_to_buffer_with_spec(
            &payload(vec![line("first", None), line("second", None)]),
            Some(&w.render),
            &registry,
            20,
            3,
        );
        assert!(cells_concat(&buf, 0).contains("first"));
        assert!(!cells_concat(&buf, 1).contains("second"));
    }

    #[test]
    fn multiple_linked_rows_each_get_independent_osc_8() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_links".into());
        let buf = render_to_buffer_with_spec(
            &payload(vec![
                line("first", Some("https://example.com/a")),
                line("second", Some("https://example.com/b")),
            ]),
            Some(&spec),
            &registry,
            20,
            2,
        );
        let row0 = cells_concat(&buf, 0);
        let row1 = cells_concat(&buf, 1);
        assert!(row0.contains("https://example.com/a"), "row0: {row0:?}");
        assert!(row1.contains("https://example.com/b"), "row1: {row1:?}");
    }

    #[test]
    fn mixed_linked_and_unlinked_rows_only_wrap_linked() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_links".into());
        let buf = render_to_buffer_with_spec(
            &payload(vec![
                line("linked", Some("https://example.com")),
                line("plain", None),
            ]),
            Some(&spec),
            &registry,
            20,
            2,
        );
        let row0 = cells_concat(&buf, 0);
        let row1 = cells_concat(&buf, 1);
        assert!(row0.contains("\x1b]8;;"), "row0 should be linked: {row0:?}");
        assert!(!row1.contains("\x1b]8;;"), "row1 should be plain: {row1:?}");
        assert!(row1.contains("plain"));
    }

    #[test]
    fn area_height_caps_rendered_rows() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_links".into());
        let buf = render_to_buffer_with_spec(
            &payload(vec![
                line("a", None),
                line("b", None),
                line("c", None),
                line("d", None),
            ]),
            Some(&spec),
            &registry,
            20,
            2,
        );
        assert!(cells_concat(&buf, 0).contains("a"));
        assert!(cells_concat(&buf, 1).contains("b"));
        // Buffer is only 2 rows tall — c and d simply have no place to render.
    }

    #[test]
    fn unicode_text_in_linked_row_keeps_osc_8_wrapper() {
        // Wide-grapheme inputs (CJK) get a per-grapheme trailing-cell reset from `set_stringn`,
        // so a literal `"こんにちは"` substring search on the cell concat won't work. We just
        // assert the OSC 8 wrapper survives intact and each visible char shows up somewhere.
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_links".into());
        let buf = render_to_buffer_with_spec(
            &payload(vec![line("こんにちは world", Some("https://example.com"))]),
            Some(&spec),
            &registry,
            30,
            1,
        );
        let row = cells_concat(&buf, 0);
        assert!(
            row.starts_with("\x1b]8;;https://example.com\x1b\\"),
            "missing OSC open: {row:?}",
        );
        assert!(row.contains("\x1b]8;;\x1b\\"), "missing OSC close: {row:?}");
        for ch in ['こ', 'ん', 'に', 'ち', 'は'] {
            assert!(row.contains(ch), "missing {ch}: {row:?}");
        }
        assert!(row.contains("world"));
    }
}
