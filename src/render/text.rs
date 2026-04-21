use ratatui::{Frame, layout::Rect, widgets::Paragraph};

use crate::payload::TextData;

pub fn render(frame: &mut Frame, area: Rect, data: &TextData) {
    frame.render_widget(Paragraph::new(data.lines.join("\n")), area);
}

#[cfg(test)]
mod tests {
    use crate::payload::{Body, Payload, TextData};
    use crate::render::test_utils::{line_text, render_to_buffer};

    fn payload(lines: &[&str]) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData {
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
