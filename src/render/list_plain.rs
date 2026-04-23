use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    widgets::{List, ListItem},
};

use crate::options::OptionSchema;
use crate::payload::{Body, TextBlockData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "bullet",
        type_hint: "\"•\" | \"●\" | \"▪\" | \"→\" | \"none\"",
        required: false,
        default: Some("none"),
        description: "Glyph prefixed to each entry. `none` (default) leaves the text unbulleted; any other string is rendered verbatim followed by a single space.",
    },
    OptionSchema {
        name: "max_items",
        type_hint: "positive integer",
        required: false,
        default: Some("all lines"),
        description: "Cap on rendered entries. Truncates from the end when the block has more lines than the cap.",
    },
    OptionSchema {
        name: "align",
        type_hint: "\"left\" | \"center\" | \"right\"",
        required: false,
        default: Some("\"left\""),
        description: "Horizontal alignment of the whole list within its cell.",
    },
];

fn align_rect(area: Rect, content_width: u16, align: Option<&str>) -> Rect {
    if content_width == 0 || content_width >= area.width {
        return area;
    }
    let offset = match align {
        Some("center") => (area.width - content_width) / 2,
        Some("right") => area.width - content_width,
        _ => return area,
    };
    Rect {
        x: area.x + offset,
        y: area.y,
        width: content_width,
        height: area.height,
    }
}

/// Renders a multi-line block through ratatui's `List` widget. Behaviourally close to `text_plain`
/// today; the distinction matters once we add list-specific options (bullet marker, highlight
/// selected item, scrollbar). Alternate renderer for the `TextBlock` shape so tests of the 1→N
/// dispatch stay honest.
pub struct ListPlainRenderer;

impl Renderer for ListPlainRenderer {
    fn name(&self) -> &str {
        "list_plain"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::TextBlock]
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
        if let Body::TextBlock(d) = body {
            render_list(frame, area, d, opts, theme);
        }
    }
}

fn render_list(
    frame: &mut Frame,
    area: Rect,
    data: &TextBlockData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    let prefix = bullet_prefix(opts.bullet.as_deref());
    let lines: Vec<String> = data
        .lines
        .iter()
        .take(opts.max_items.unwrap_or(usize::MAX))
        .map(|l| format!("{prefix}{l}"))
        .collect();
    let max = lines
        .iter()
        .map(|l| l.chars().count() as u16)
        .max()
        .unwrap_or(0);
    let target = align_rect(area, max, opts.align.as_deref());
    let items: Vec<ListItem> = lines.into_iter().map(ListItem::new).collect();
    frame.render_widget(
        List::new(items).style(Style::default().fg(theme.text)),
        target,
    );
}

fn bullet_prefix(bullet: Option<&str>) -> String {
    match bullet {
        None | Some("none") | Some("") => String::new(),
        Some(g) => format!("{g} "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Payload, TextBlockData};
    use crate::render::test_utils::{line_text, render_to_buffer_with_spec};
    use crate::render::{Registry, RenderSpec};

    fn payload(lines: &[&str]) -> Payload {
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
    fn renders_each_line() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_plain".into());
        let buf =
            render_to_buffer_with_spec(&payload(&["alpha", "beta"]), Some(&spec), &registry, 20, 4);
        assert!(line_text(&buf, 0).contains("alpha"));
        assert!(line_text(&buf, 1).contains("beta"));
    }

    #[test]
    fn bullet_option_prefixes_each_entry() {
        let registry = Registry::with_builtins();
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W = toml::from_str(r#"render = { type = "list_plain", bullet = "•" }"#).unwrap();
        let buf =
            render_to_buffer_with_spec(&payload(&["a", "b"]), Some(&w.render), &registry, 20, 3);
        assert!(
            line_text(&buf, 0).contains("• a"),
            "row 0: {:?}",
            line_text(&buf, 0)
        );
        assert!(
            line_text(&buf, 1).contains("• b"),
            "row 1: {:?}",
            line_text(&buf, 1)
        );
    }

    #[test]
    fn max_items_truncates_output() {
        let registry = Registry::with_builtins();
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W = toml::from_str(r#"render = { type = "list_plain", max_items = 2 }"#).unwrap();
        let buf = render_to_buffer_with_spec(
            &payload(&["a", "b", "c", "d"]),
            Some(&w.render),
            &registry,
            20,
            5,
        );
        assert!(line_text(&buf, 0).contains("a"));
        assert!(line_text(&buf, 1).contains("b"));
        // Third line was capped out.
        assert!(
            !line_text(&buf, 2).contains("c"),
            "row 2: {:?}",
            line_text(&buf, 2)
        );
    }
}
