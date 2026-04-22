use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::payload::{BadgeData, Body, Status};
use crate::theme::{self, ColorKey, Theme};

use super::{RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[
    theme::STATUS_OK,
    theme::STATUS_WARN,
    theme::STATUS_ERROR,
    theme::TEXT,
];

/// Traffic-light / badge renderer: a single coloured dot (● green / yellow / red) driven by
/// `BadgeData.status`, followed by `BadgeData.label`. One indicator per widget — CI, deploy,
/// SLO, oncall. Rows of badges are a composition concern, handled by the nested layout
/// (`status_row`), not by stuffing multiple statuses into one payload.
pub struct StatusBadgeRenderer;

const DOT: &str = "●";

impl Renderer for StatusBadgeRenderer {
    fn name(&self) -> &str {
        "status_badge"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Badge]
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
    ) {
        if let Body::Badge(d) = body {
            render_badge(frame, area, d, opts, theme);
        }
    }
}

fn render_badge(
    frame: &mut Frame,
    area: Rect,
    data: &BadgeData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    let label = Style::default().fg(theme.text);
    let line = Line::from(vec![
        Span::styled(DOT, Style::default().fg(status_color(data.status, theme))),
        Span::styled(" ", label),
        Span::styled(data.label.clone(), label),
    ]);
    let p = Paragraph::new(line).alignment(parse_align(opts.align.as_deref()));
    frame.render_widget(p, area);
}

pub(super) fn status_color(status: Status, theme: &Theme) -> Color {
    match status {
        Status::Ok => theme.status_ok,
        Status::Warn => theme.status_warn,
        Status::Error => theme.status_error,
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
        let spec = RenderSpec::Short("status_badge".into());
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
    fn ok_maps_to_theme_status_ok() {
        let buf = render(Status::Ok, "x", 10, 1);
        let theme = crate::theme::Theme::default();
        assert_eq!(buf.cell((0, 0)).unwrap().fg, theme.status_ok);
    }

    #[test]
    fn warn_maps_to_theme_status_warn() {
        let buf = render(Status::Warn, "x", 10, 1);
        let theme = crate::theme::Theme::default();
        assert_eq!(buf.cell((0, 0)).unwrap().fg, theme.status_warn);
    }

    #[test]
    fn error_maps_to_theme_status_error() {
        let buf = render(Status::Error, "x", 10, 1);
        let theme = crate::theme::Theme::default();
        assert_eq!(buf.cell((0, 0)).unwrap().fg, theme.status_error);
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
