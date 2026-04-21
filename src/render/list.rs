use ratatui::{
    Frame,
    layout::Rect,
    widgets::{List, ListItem},
};

use crate::payload::{Body, LinesData};

use super::{RenderOptions, Renderer, Shape};

/// Renders lines through ratatui's `List` widget. Behaviourally close to `simple` today; the
/// distinction matters once we add list-specific options (bullet marker, highlight selected
/// item, scrollbar). Alternate renderer for the `Lines` shape so tests of the 1→N dispatch
/// stay honest.
pub struct ListRenderer;

impl Renderer for ListRenderer {
    fn name(&self) -> &str {
        "list"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Lines]
    }
    fn render(&self, frame: &mut Frame, area: Rect, body: &Body, _opts: &RenderOptions) {
        if let Body::Lines(d) = body {
            render_list(frame, area, d);
        }
    }
}

fn render_list(frame: &mut Frame, area: Rect, data: &LinesData) {
    let items: Vec<ListItem> = data
        .lines
        .iter()
        .map(|l| ListItem::new(l.clone()))
        .collect();
    frame.render_widget(List::new(items), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{LinesData, Payload};
    use crate::render::test_utils::{line_text, render_to_buffer_with_spec};
    use crate::render::{Registry, RenderSpec};

    fn payload(lines: &[&str]) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Lines(LinesData {
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
