use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::options::OptionSchema;
use crate::payload::{BadgeData, Body, Status};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    pub shape: Option<String>,
}

const COLOR_KEYS: &[ColorKey] = &[
    theme::STATUS_OK,
    theme::STATUS_WARN,
    theme::STATUS_ERROR,
    theme::TEXT,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "shape",
        type_hint: "\"rounded\" | \"square\" | \"none\"",
        required: false,
        default: Some("\"none\""),
        description: "Pill framing. `rounded` wraps the label in `( … )`, `square` in `[ … ]`, `none` (default) leaves the bare dot + label.",
    },
    OptionSchema {
        name: "padding",
        type_hint: "cells (u16)",
        required: false,
        default: Some("0"),
        description: "Horizontal padding inside the pill, before the dot and after the label.",
    },
    OptionSchema {
        name: "align",
        type_hint: "\"left\" | \"center\" | \"right\"",
        required: false,
        default: Some("\"left\""),
        description: "Horizontal placement of the pill within its cell.",
    },
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
    let label_style = Style::default().fg(theme.text);
    let dot_style = Style::default().fg(status_color(data.status, theme));
    let specific: Options = opts.parse_specific();
    let (open, close) = frame_glyphs(specific.shape.as_deref());
    let pad = opts.padding.unwrap_or(0) as usize;
    let spacer: String = " ".repeat(pad);
    let mut spans: Vec<Span> = Vec::with_capacity(7);
    if !open.is_empty() {
        spans.push(Span::styled(format!("{open}{spacer}"), label_style));
    } else if pad > 0 {
        spans.push(Span::styled(spacer.clone(), label_style));
    }
    spans.push(Span::styled(DOT, dot_style));
    spans.push(Span::styled(" ", label_style));
    spans.push(Span::styled(data.label.clone(), label_style));
    if !close.is_empty() {
        spans.push(Span::styled(format!("{spacer}{close}"), label_style));
    } else if pad > 0 {
        spans.push(Span::styled(spacer, label_style));
    }
    let p = Paragraph::new(Line::from(spans)).alignment(parse_align(opts.align.as_deref()));
    frame.render_widget(p, area);
}

fn frame_glyphs(shape: Option<&str>) -> (&'static str, &'static str) {
    match shape {
        Some("rounded") => ("( ", " )"),
        Some("square") => ("[ ", " ]"),
        _ => ("", ""),
    }
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
    fn rounded_shape_wraps_label_in_parens() {
        let registry = Registry::with_builtins();
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W =
            toml::from_str(r#"render = { type = "status_badge", shape = "rounded" }"#).unwrap();
        let buf = render_to_buffer_with_spec(
            &payload(Status::Ok, "ci"),
            Some(&w.render),
            &registry,
            30,
            1,
        );
        let row = line_text(&buf, 0);
        assert!(row.starts_with("( "), "row: {row:?}");
        assert!(row.contains(" )"), "row: {row:?}");
    }

    #[test]
    fn square_shape_wraps_label_in_brackets() {
        let registry = Registry::with_builtins();
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W =
            toml::from_str(r#"render = { type = "status_badge", shape = "square", padding = 1 }"#)
                .unwrap();
        let buf = render_to_buffer_with_spec(
            &payload(Status::Ok, "ci"),
            Some(&w.render),
            &registry,
            30,
            1,
        );
        let row = line_text(&buf, 0);
        assert!(row.starts_with("[ "), "row: {row:?}");
        assert!(row.contains(" ]"), "row: {row:?}");
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
