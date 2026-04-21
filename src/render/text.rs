use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    widgets::Paragraph,
};

use crate::payload::{Body, LinesData};

use super::{RenderOptions, Renderer, Shape};

/// Plain-text renderer: stacks `LinesData.lines` into a ratatui `Paragraph`. The default
/// renderer for the `Lines` shape, used for greetings, project notes, static blocks. Honours
/// the `align` option (left / center / right).
pub struct SimpleRenderer;

impl Renderer for SimpleRenderer {
    fn name(&self) -> &str {
        "simple"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Lines]
    }
    fn render(&self, frame: &mut Frame, area: Rect, body: &Body, opts: &RenderOptions) {
        if let Body::Lines(d) = body {
            render_lines(frame, area, d, opts);
        }
    }
}

fn render_lines(frame: &mut Frame, area: Rect, data: &LinesData, opts: &RenderOptions) {
    let p = Paragraph::new(data.lines.join("\n")).alignment(parse_align(opts.align.as_deref()));
    frame.render_widget(p, area);
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
    use crate::payload::{LinesData, Payload};
    use crate::render::test_utils::{line_text, render_to_buffer};

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
    fn renders_lines_at_top() {
        let buf = render_to_buffer(&payload(&["hello world"]), 30, 5);
        assert!(line_text(&buf, 0).contains("hello world"));
    }
}
