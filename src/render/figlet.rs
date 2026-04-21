use ratatui::{Frame, layout::Rect, widgets::Paragraph};

use crate::payload::{Body, LinesData};

use super::{RenderOptions, Renderer, Shape};

/// FIGlet-style ASCII art text over the `Lines` shape. Paired with `ascii_art` (block glyphs via
/// `tui-big-text`) as an alternate visual for the same data — classic "letters made of letters"
/// vs. half-block pixels. Falls back to the raw text on conversion failure so a malformed input
/// never goes blank.
pub struct FigletRenderer;

impl Renderer for FigletRenderer {
    fn name(&self) -> &str {
        "figlet"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Lines]
    }
    fn render(&self, frame: &mut Frame, area: Rect, body: &Body, _opts: &RenderOptions) {
        if let Body::Lines(d) = body {
            render_figlet(frame, area, d);
        }
    }
}

fn render_figlet(frame: &mut Frame, area: Rect, data: &LinesData) {
    let rendered = data
        .lines
        .iter()
        .map(|l| figletify(l))
        .collect::<Vec<_>>()
        .join("\n");
    frame.render_widget(Paragraph::new(rendered), area);
}

fn figletify(text: &str) -> String {
    let Ok(font) = figlet_rs::FIGfont::standard() else {
        return text.to_string();
    };
    font.convert(text)
        .map(|f| f.to_string())
        .unwrap_or_else(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{LinesData, Payload};
    use crate::render::test_utils::render_to_buffer_with_spec;
    use crate::render::{Registry, RenderSpec};

    fn payload(text: &str) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Lines(LinesData {
                lines: vec![text.into()],
            }),
        }
    }

    #[test]
    fn renders_without_panicking() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("figlet".into());
        let _ = render_to_buffer_with_spec(&payload("hi"), Some(&spec), &registry, 60, 10);
    }

    #[test]
    fn empty_input_falls_back_gracefully() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("figlet".into());
        let _ = render_to_buffer_with_spec(&payload(""), Some(&spec), &registry, 60, 10);
    }
}
