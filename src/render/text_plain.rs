use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
    widgets::Paragraph,
};

use crate::options::OptionSchema;
use crate::payload::Body;
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "align",
    type_hint: "\"left\" | \"center\" | \"right\"",
    required: false,
    default: Some("\"left\""),
    description: "Horizontal alignment of the rendered text within its cell.",
}];

/// Plain-text renderer. Accepts both `Text` (single string) and `TextBlock` (multi-line) and
/// draws them as a ratatui `Paragraph`. The default renderer for both shapes. Honours the
/// `align` option (left / center / right).
pub struct TextPlainRenderer;

impl Renderer for TextPlainRenderer {
    fn name(&self) -> &str {
        "text_plain"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Text, Shape::TextBlock]
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn color_keys(&self) -> &[ColorKey] {
        COLOR_KEYS
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
        let content = match body {
            Body::Text(d) => d.value.clone(),
            Body::TextBlock(d) => d.lines.join("\n"),
            _ => return,
        };
        let p = Paragraph::new(content)
            .style(Style::default().fg(theme.text))
            .alignment(parse_align(opts.align.as_deref()));
        frame.render_widget(p, area);
    }
    fn natural_height(
        &self,
        body: &Body,
        _opts: &RenderOptions,
        _max_width: u16,
        _registry: &Registry,
    ) -> u16 {
        // text_plain doesn't wrap, so each `\n`-separated line takes exactly one row. Empty
        // bodies still deserve a row of height so `length = "auto"` callers don't collapse.
        let lines = match body {
            Body::Text(d) => d.value.lines().count().max(1),
            Body::TextBlock(d) => d.lines.len().max(1),
            _ => 1,
        };
        u16::try_from(lines).unwrap_or(u16::MAX)
    }
}

fn parse_align(s: Option<&str>) -> Alignment {
    match s {
        Some("center") => Alignment::Center,
        Some("right") => Alignment::Right,
        _ => Alignment::Left,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Payload, TextBlockData, TextData};
    use crate::render::test_utils::{line_text, render_to_buffer};

    fn text_payload(value: &str) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData {
                value: value.to_string(),
            }),
        }
    }

    fn block_payload(lines: &[&str]) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::TextBlock(TextBlockData {
                lines: lines.iter().map(|s| s.to_string()).collect(),
            }),
        }
    }

    #[test]
    fn renders_text_at_top() {
        let buf = render_to_buffer(&text_payload("hello world"), 30, 5);
        assert!(line_text(&buf, 0).contains("hello world"));
    }

    #[test]
    fn renders_text_block_stacked() {
        let buf = render_to_buffer(&block_payload(&["first", "second"]), 30, 5);
        assert!(line_text(&buf, 0).contains("first"));
        assert!(line_text(&buf, 1).contains("second"));
    }

    #[test]
    fn natural_height_text_counts_embedded_newlines() {
        let r = TextPlainRenderer;
        let registry = Registry::with_builtins();
        let opts = RenderOptions::default();
        assert_eq!(
            r.natural_height(&text_payload("hello world").body, &opts, 30, &registry),
            1
        );
        assert_eq!(
            r.natural_height(&text_payload("a\nb\nc").body, &opts, 30, &registry),
            3
        );
    }

    #[test]
    fn natural_height_textblock_uses_line_count() {
        let r = TextPlainRenderer;
        let registry = Registry::with_builtins();
        let opts = RenderOptions::default();
        assert_eq!(
            r.natural_height(&block_payload(&["one"]).body, &opts, 30, &registry),
            1
        );
        assert_eq!(
            r.natural_height(
                &block_payload(&["one", "two", "three"]).body,
                &opts,
                30,
                &registry
            ),
            3
        );
    }

    #[test]
    fn natural_height_floors_at_one_for_empty_bodies() {
        let r = TextPlainRenderer;
        let registry = Registry::with_builtins();
        let opts = RenderOptions::default();
        assert_eq!(
            r.natural_height(&text_payload("").body, &opts, 30, &registry),
            1
        );
        assert_eq!(
            r.natural_height(&block_payload(&[]).body, &opts, 30, &registry),
            1
        );
    }
}
