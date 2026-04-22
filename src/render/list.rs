use ratatui::{
    Frame,
    layout::Rect,
    widgets::{List, ListItem},
};

use crate::payload::{Body, TextBlockData};

use super::{RenderOptions, Renderer, Shape};

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

/// Renders a multi-line block through ratatui's `List` widget. Behaviourally close to `text`
/// today; the distinction matters once we add list-specific options (bullet marker, highlight
/// selected item, scrollbar). Alternate renderer for the `TextBlock` shape so tests of the 1→N
/// dispatch stay honest.
pub struct ListRenderer;

impl Renderer for ListRenderer {
    fn name(&self) -> &str {
        "list"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::TextBlock]
    }
    fn render(&self, frame: &mut Frame, area: Rect, body: &Body, opts: &RenderOptions) {
        if let Body::TextBlock(d) = body {
            render_list(frame, area, d, opts);
        }
    }
}

fn render_list(frame: &mut Frame, area: Rect, data: &TextBlockData, opts: &RenderOptions) {
    let max = data
        .lines
        .iter()
        .map(|l| l.chars().count() as u16)
        .max()
        .unwrap_or(0);
    let target = align_rect(area, max, opts.align.as_deref());
    let items: Vec<ListItem> = data
        .lines
        .iter()
        .map(|l| ListItem::new(l.clone()))
        .collect();
    frame.render_widget(List::new(items), target);
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
        let spec = RenderSpec::Short("list".into());
        let buf =
            render_to_buffer_with_spec(&payload(&["alpha", "beta"]), Some(&spec), &registry, 20, 4);
        assert!(line_text(&buf, 0).contains("alpha"));
        assert!(line_text(&buf, 1).contains("beta"));
    }
}
