use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::payload::{BadgeData, Body, Status};

use super::{RenderOptions, Renderer, Shape};

/// Traffic-light / badge renderer: a single coloured dot (● green / yellow / red) driven by
/// `BadgeData.status`, followed by `BadgeData.label`. One indicator per widget — CI, deploy,
/// SLO, oncall. Rows of badges are a composition concern, handled by the nested layout
/// (`combined_status_row`), not by stuffing multiple statuses into one payload.
pub struct BadgeRenderer;

const DOT: &str = "●";

impl Renderer for BadgeRenderer {
    fn name(&self) -> &str {
        "badge"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Badge]
    }
    fn render(&self, frame: &mut Frame, area: Rect, body: &Body, opts: &RenderOptions) {
        if let Body::Badge(d) = body {
            render_badge(frame, area, d, opts);
        }
    }
}

fn render_badge(frame: &mut Frame, area: Rect, data: &BadgeData, opts: &RenderOptions) {
    let line = Line::from(vec![
        Span::styled(DOT, Style::default().fg(status_color(data.status))),
        Span::raw(" "),
        Span::raw(data.label.clone()),
    ]);
    let p = Paragraph::new(line).alignment(parse_align(opts.align.as_deref()));
    frame.render_widget(p, area);
}

fn status_color(status: Status) -> Color {
    match status {
        Status::Ok => Color::Green,
        Status::Warn => Color::Yellow,
        Status::Error => Color::Red,
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
    use crate::payload::{BadgeData, Payload, Status};
    use crate::render::test_utils::{line_text, render_to_buffer_with_spec};
    use crate::render::{Registry, RenderSpec};

    fn payload(status: Status, label: &str) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Badge(BadgeData {
                status,
                label: label.into(),
            }),
        }
    }

    fn render(status: Status, label: &str, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("badge".into());
        render_to_buffer_with_spec(&payload(status, label), Some(&spec), &registry, w, h)
    }

    #[test]
    fn renders_dot_and_label() {
        let buf = render(Status::Ok, "build passing", 30, 1);
        let row = line_text(&buf, 0);
        assert!(row.starts_with(DOT), "expected dot prefix: {row:?}");
        assert!(row.contains("build passing"));
    }

    #[test]
    fn ok_maps_to_green() {
        let buf = render(Status::Ok, "x", 10, 1);
        assert_eq!(buf.cell((0, 0)).unwrap().fg, Color::Green);
    }

    #[test]
    fn warn_maps_to_yellow() {
        let buf = render(Status::Warn, "x", 10, 1);
        assert_eq!(buf.cell((0, 0)).unwrap().fg, Color::Yellow);
    }

    #[test]
    fn error_maps_to_red() {
        let buf = render(Status::Error, "x", 10, 1);
        assert_eq!(buf.cell((0, 0)).unwrap().fg, Color::Red);
    }

    #[test]
    fn badge_is_the_default_renderer_for_badge_shape() {
        // A widget with no explicit `render =` still picks `badge`, since the shape has exactly
        // one natural renderer.
        let registry = Registry::with_builtins();
        let buf = render_to_buffer_with_spec(&payload(Status::Ok, "ci"), None, &registry, 10, 1);
        assert!(line_text(&buf, 0).contains("ci"));
    }
}
