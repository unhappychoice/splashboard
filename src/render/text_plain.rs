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

/// Plain-text renderer for the single-string `Text` shape — draws the value as a ratatui
/// `Paragraph`. Honours the `align` option (left / center / right). For multi-line `TextBlock`
/// payloads use `list_plain` instead; pairing `text_plain` with a multi-line fetcher yields a
/// shape-mismatch placeholder by design.
pub struct TextPlainRenderer;

impl Renderer for TextPlainRenderer {
    fn name(&self) -> &str {
        "text_plain"
    }
    fn description(&self) -> &'static str {
        "Single-line body text in the theme text colour, no decoration. The quiet default for any `Text` payload — pick `text_ascii` for a hero block, or `list_plain` when the fetcher emits a `TextBlock`."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Text]
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
        let Body::Text(d) = body else { return };
        let content = d.value.clone();
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
    use crate::payload::{Payload, TextData};
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

    #[test]
    fn renders_text_at_top() {
        let buf = render_to_buffer(&text_payload("hello world"), 30, 5);
        assert!(line_text(&buf, 0).contains("hello world"));
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
    fn natural_height_floors_at_one_for_empty_bodies() {
        let r = TextPlainRenderer;
        let registry = Registry::with_builtins();
        let opts = RenderOptions::default();
        assert_eq!(
            r.natural_height(&text_payload("").body, &opts, 30, &registry),
            1
        );
    }
}
