use std::sync::OnceLock;
use std::time::Instant;

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    widgets::Paragraph,
};

use crate::options::OptionSchema;
use crate::payload::Body;
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape, default_renderer_for, shape_of};

/// Lenient on unknown fields: see `animated_postfx::Options` for the rationale.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct Options {
    #[serde(default)]
    pub inner: Option<String>,
}

const COLOR_KEYS: &[ColorKey] = &[theme::PANEL_TITLE, theme::TEXT];

const DEFAULT_DURATION_MS: u64 = 1600;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "inner",
        type_hint: "renderer name (e.g. \"text_ascii\")",
        required: false,
        default: Some("shape default renderer"),
        description: "Renderer drawn underneath the scanline sweep. Falls back to the shape's natural default.",
    },
    OptionSchema {
        name: "duration_ms",
        type_hint: "milliseconds (u64)",
        required: false,
        default: Some("1600"),
        description: "How long the scanline takes to traverse from top to bottom of the widget cell. Stays inside the 2s ANIMATION_WINDOW by default.",
    },
];

/// CRT-style reveal. Renders the inner renderer once, then masks everything below a moving
/// horizontal scanline. The scanline itself renders as a bright highlight row; rows below
/// stay blank until the line passes them. Once the window closes, the inner render is left
/// unmasked so the static frame is indistinguishable from a plain widget.
///
/// Accepts every shape: the compatibility check is delegated to the resolved inner renderer.
pub struct AnimatedScanlinesRenderer;

const ALL_SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::Entries,
    Shape::Ratio,
    Shape::NumberSeries,
    Shape::PointSeries,
    Shape::Bars,
    Shape::Image,
    Shape::Calendar,
    Shape::Heatmap,
    Shape::Badge,
    Shape::Timeline,
];

impl Renderer for AnimatedScanlinesRenderer {
    fn name(&self) -> &str {
        "animated_scanlines"
    }
    fn description(&self) -> &'static str {
        "Wrapper that sweeps a bright horizontal CRT scanline from top to bottom over the inner renderer, hiding rows below the line until it passes them. Accepts every shape; reads as a vintage tube monitor warming up."
    }
    fn accepts(&self) -> &[Shape] {
        ALL_SHAPES
    }
    fn animates(&self) -> bool {
        true
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
        registry: &Registry,
    ) {
        let shape = shape_of(body);
        let specific: Options = opts.parse_specific();
        let inner_name = specific
            .inner
            .as_deref()
            .unwrap_or_else(|| default_renderer_for(shape));
        let Some(inner) = registry.get(inner_name) else {
            frame.render_widget(
                Paragraph::new(format!("unknown inner renderer: {inner_name}")),
                area,
            );
            return;
        };
        if !inner.accepts().contains(&shape) {
            frame.render_widget(
                Paragraph::new(format!("inner {inner_name} cannot display {shape:?}")),
                area,
            );
            return;
        }
        inner.render(frame, area, body, &inner_options(opts), theme, registry);

        let total = opts.duration_ms.unwrap_or(DEFAULT_DURATION_MS).max(1) as u32;
        let elapsed = elapsed_since_start_ms();
        if elapsed >= total || area.height == 0 {
            return;
        }
        apply_scanline(frame, area, elapsed, total, theme);
    }
    fn natural_height(
        &self,
        body: &Body,
        opts: &RenderOptions,
        max_width: u16,
        registry: &Registry,
    ) -> u16 {
        let shape = shape_of(body);
        let specific: Options = opts.parse_specific();
        let inner_name = specific
            .inner
            .as_deref()
            .unwrap_or_else(|| default_renderer_for(shape));
        let Some(inner) = registry.get(inner_name) else {
            return 1;
        };
        inner.natural_height(body, &inner_options(opts), max_width, registry)
    }
}

fn apply_scanline(frame: &mut Frame, area: Rect, elapsed_ms: u32, total_ms: u32, theme: &Theme) {
    let progress = (elapsed_ms as f32 / total_ms as f32).clamp(0.0, 1.0);
    let travel = area.height as f32 + 2.0;
    let scan_y = progress * travel;
    let scan_row = scan_y.floor() as i32;
    let buf = frame.buffer_mut();

    for y in 0..area.height {
        let row = (area.y + y) as i32;
        let abs_y = y as i32;
        if abs_y > scan_row {
            blank_row(buf, area, y);
        } else if abs_y == scan_row && row >= area.y as i32 {
            highlight_row(buf, area, y, theme);
        }
    }
}

fn blank_row(buf: &mut ratatui::buffer::Buffer, area: Rect, y: u16) {
    for x in 0..area.width {
        if let Some(cell) = buf.cell_mut((area.x + x, area.y + y)) {
            cell.set_symbol(" ");
            cell.set_style(Style::default());
        }
    }
}

fn highlight_row(buf: &mut ratatui::buffer::Buffer, area: Rect, y: u16, theme: &Theme) {
    let style = Style::default()
        .fg(theme.panel_title)
        .add_modifier(Modifier::BOLD);
    for x in 0..area.width {
        if let Some(cell) = buf.cell_mut((area.x + x, area.y + y))
            && cell.symbol().trim().is_empty()
        {
            cell.set_symbol("─");
            cell.set_style(style);
        }
    }
}

fn inner_options(opts: &RenderOptions) -> RenderOptions {
    RenderOptions {
        duration_ms: None,
        ..opts.clone()
    }
    .without_extra("inner")
    .without_extra("effect")
    .without_extra("boot_lines")
    .without_extra("font_sequence")
}

fn elapsed_since_start_ms() -> u32 {
    let elapsed = process_start().elapsed().as_millis();
    elapsed.min(u32::MAX as u128) as u32
}

fn process_start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Payload, TextData};
    use crate::render::{
        Registry, RenderSpec,
        test_utils::{line_text, render_to_buffer, render_to_buffer_with_spec},
    };
    use crate::theme::{self, Theme};
    use ratatui::{Terminal, backend::TestBackend, widgets::Paragraph};

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
    fn exposes_wrapper_contract() {
        let renderer = AnimatedScanlinesRenderer;
        assert_eq!(renderer.name(), "animated_scanlines");
        assert!(renderer.description().contains("CRT scanline"));
        assert!(renderer.animates());
        assert_eq!(renderer.color_keys().len(), 2);
        assert_eq!(renderer.color_keys()[0].name, theme::PANEL_TITLE.name);
        assert_eq!(renderer.color_keys()[1].name, theme::TEXT.name);
        assert_eq!(renderer.option_schemas().len(), OPTION_SCHEMAS.len());
        assert_eq!(renderer.option_schemas()[0].name, "inner");
        assert_eq!(renderer.option_schemas()[1].name, "duration_ms");
        assert!(renderer.accepts().contains(&Shape::Text));
        assert!(renderer.accepts().contains(&Shape::Timeline));
    }

    #[test]
    fn natural_height_delegates_and_unknown_inner_falls_back_to_one() {
        let registry = Registry::with_builtins();
        let renderer = AnimatedScanlinesRenderer;
        let opts = RenderOptions {
            style: Some("figlet".into()),
            ..RenderOptions::default()
        }
        .with_extra("inner", "text_ascii");
        let body = text_payload("scanlines").body;
        let expected = registry.get("text_ascii").unwrap().natural_height(
            &body,
            &inner_options(&opts),
            40,
            &registry,
        );
        assert_eq!(
            renderer.natural_height(&body, &opts, 40, &registry),
            expected
        );

        let missing = RenderOptions::default().with_extra("inner", "missing_renderer");
        assert_eq!(renderer.natural_height(&body, &missing, 40, &registry), 1);
    }

    #[test]
    fn inner_options_strips_wrapper_fields() {
        let opts = RenderOptions {
            duration_ms: Some(1500),
            style: Some("figlet".into()),
            ..RenderOptions::default()
        }
        .with_extra("inner", "text_ascii");
        let inner = inner_options(&opts);
        let parsed: Options = inner.parse_specific();
        assert!(parsed.inner.is_none());
        assert!(inner.duration_ms.is_none());
        assert_eq!(inner.style.as_deref(), Some("figlet"));
    }

    #[test]
    fn render_reports_unknown_and_incompatible_inner() {
        let payload = text_payload("hi");
        let registry = Registry::with_builtins();
        let unknown = RenderSpec::Full {
            type_name: "animated_scanlines".into(),
            options: RenderOptions::default().with_extra("inner", "missing_renderer"),
        };
        let buf = render_to_buffer_with_spec(&payload, Some(&unknown), &registry, 40, 3);
        assert!(line_text(&buf, 0).contains("unknown inner renderer: missing_renderer"));

        let incompatible = RenderSpec::Full {
            type_name: "animated_scanlines".into(),
            options: RenderOptions::default().with_extra("inner", "gauge_circle"),
        };
        let buf = render_to_buffer_with_spec(&payload, Some(&incompatible), &registry, 40, 3);
        assert!(line_text(&buf, 0).contains("cannot display Text"));
    }

    #[test]
    fn render_matches_static_inner_once_window_expires() {
        let payload = text_payload("scanline text");
        let spec = RenderSpec::Full {
            type_name: "animated_scanlines".into(),
            options: RenderOptions {
                duration_ms: Some(1),
                ..RenderOptions::default()
            },
        };
        let expected = render_to_buffer(&payload, 30, 3);
        let _ = elapsed_since_start_ms();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let actual =
            render_to_buffer_with_spec(&payload, Some(&spec), &Registry::with_builtins(), 30, 3);
        assert_eq!(actual, expected);
    }

    #[test]
    fn apply_scanline_highlights_current_row_and_blanks_lower_rows() {
        let backend = TestBackend::new(5, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                frame.render_widget(Paragraph::new("A\nB\nCCC"), area);
                apply_scanline(frame, area, 300, 1000, &Theme::default());
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        assert!(line_text(&buf, 0).starts_with('A'));
        assert!(line_text(&buf, 1).starts_with('B'));
        assert!(line_text(&buf, 1).contains('─'));
        assert_eq!(line_text(&buf, 2), "     ");
        let scanline = buf.cell((1, 1)).unwrap().style();
        assert_eq!(scanline.fg, Some(Theme::default().panel_title));
    }

    #[test]
    fn renders_without_panic() {
        let payload = text_payload("hi");
        let spec = RenderSpec::Full {
            type_name: "animated_scanlines".into(),
            options: RenderOptions {
                style: Some("figlet".into()),
                duration_ms: Some(400),
                ..RenderOptions::default()
            }
            .with_extra("inner", "text_ascii"),
        };
        let registry = Registry::with_builtins();
        let _ = render_to_buffer_with_spec(&payload, Some(&spec), &registry, 40, 10);
    }
}
