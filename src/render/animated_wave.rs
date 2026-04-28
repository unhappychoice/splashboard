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

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    pub inner: Option<String>,
}

const COLOR_KEYS: &[ColorKey] = &[theme::PANEL_TITLE, theme::TEXT];

const DEFAULT_DURATION_MS: u64 = 1600;

const WAVE_CREST_WIDTH: u16 = 6;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "inner",
        type_hint: "renderer name (e.g. \"text_ascii\")",
        required: false,
        default: Some("shape default renderer"),
        description: "Renderer drawn as the resting frame once the wave has passed through every column. Falls back to the shape's natural default.",
    },
    OptionSchema {
        name: "duration_ms",
        type_hint: "milliseconds (u64)",
        required: false,
        default: Some("1600"),
        description: "Total sweep duration. The wave enters from the left at t=0 and exits past the right edge at t=duration.",
    },
];

/// Wave reveal — a bright vertical crest travels left-to-right across the widget. Columns the
/// crest has passed are revealed; columns ahead of it stay blank; the column at the crest is
/// highlighted with the accent colour. Reads as "a wave of signal washes over the hero".
///
/// Accepts every shape: the compatibility check is delegated to the resolved inner renderer.
/// Static post-window frame = the inner render, unchanged.
pub struct AnimatedWaveRenderer;

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

impl Renderer for AnimatedWaveRenderer {
    fn name(&self) -> &str {
        "animated_wave"
    }
    fn description(&self) -> &'static str {
        "Wrapper that sweeps a bright vertical crest left-to-right across the inner renderer; columns ahead of the crest stay blank, columns behind are revealed, the crest itself is highlighted in the accent colour. Accepts every shape; reads as a wave of signal washing over the hero."
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
        if elapsed >= total {
            return;
        }
        apply_wave(frame, area, elapsed, total, theme);
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

fn apply_wave(frame: &mut Frame, area: Rect, elapsed_ms: u32, total_ms: u32, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let progress = (elapsed_ms as f32 / total_ms as f32).clamp(0.0, 1.0);
    let travel = area.width as i32 + WAVE_CREST_WIDTH as i32 * 2;
    let crest_x = (progress * travel as f32) as i32 - WAVE_CREST_WIDTH as i32;
    let buf = frame.buffer_mut();
    let crest_style = Style::default()
        .fg(theme.panel_title)
        .add_modifier(Modifier::BOLD);

    for y in 0..area.height {
        for x in 0..area.width {
            let x_i = x as i32;
            if x_i > crest_x + WAVE_CREST_WIDTH as i32 {
                blank_cell(buf, area, x, y);
            } else if (x_i - crest_x).abs() <= WAVE_CREST_WIDTH as i32 / 2 {
                highlight_cell(buf, area, x, y, crest_style);
            }
        }
    }
}

fn blank_cell(buf: &mut ratatui::buffer::Buffer, area: Rect, x: u16, y: u16) {
    if let Some(cell) = buf.cell_mut((area.x + x, area.y + y)) {
        cell.set_symbol(" ");
        cell.set_style(Style::default());
    }
}

fn highlight_cell(buf: &mut ratatui::buffer::Buffer, area: Rect, x: u16, y: u16, style: Style) {
    if let Some(cell) = buf.cell_mut((area.x + x, area.y + y))
        && !cell.symbol().trim().is_empty()
    {
        cell.set_style(style);
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
        RenderSpec,
        test_utils::{line_text, render_to_buffer_with_spec},
    };
    use ratatui::{
        Terminal, backend::TestBackend, buffer::Buffer, style::Color, widgets::Paragraph,
    };

    fn text_payload(value: &str) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData {
                value: value.into(),
            }),
        }
    }

    fn fg_at(buf: &Buffer, row: u16, needle: char) -> Color {
        let area = buf.area;
        for x in 0..area.width {
            let cell = buf.cell((x, row)).unwrap();
            if cell.symbol().contains(needle) {
                return cell.style().fg.unwrap_or(Color::Reset);
            }
        }
        panic!("char {needle:?} not found on row {row}");
    }

    fn render_wave_frame(elapsed_ms: u32, total_ms: u32, width: u16) -> Buffer {
        let backend = TestBackend::new(width, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::default();
        terminal
            .draw(|frame| {
                let area = frame.area();
                frame.render_widget(Paragraph::new("ABCDEFGH"), area);
                apply_wave(frame, area, elapsed_ms, total_ms, &theme);
            })
            .unwrap();
        terminal.backend().buffer().clone()
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
    fn renderer_exposes_docs_metadata() {
        let renderer = AnimatedWaveRenderer;
        let schemas = renderer.option_schemas();
        let colors = renderer.color_keys();

        assert_eq!(renderer.name(), "animated_wave");
        assert!(renderer.animates());
        assert!(renderer.description().contains("wave of signal"));
        assert_eq!(renderer.accepts(), ALL_SHAPES);
        assert_eq!(schemas.len(), 2);
        assert_eq!(schemas[0].name, "inner");
        assert_eq!(schemas[1].name, "duration_ms");
        assert_eq!(colors.len(), 2);
        assert_eq!(colors[0].name, "panel_title");
        assert_eq!(colors[1].name, "text");
    }

    #[test]
    fn unknown_inner_renderer_renders_inline_error() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Full {
            type_name: "animated_wave".into(),
            options: RenderOptions::default().with_extra("inner", "missing"),
        };
        let buf = render_to_buffer_with_spec(&text_payload("wave"), Some(&spec), &registry, 50, 1);
        assert!(line_text(&buf, 0).contains("unknown inner renderer: missing"));
    }

    #[test]
    fn shape_mismatch_inner_renderer_renders_inline_error() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Full {
            type_name: "animated_wave".into(),
            options: RenderOptions::default().with_extra("inner", "grid_table"),
        };
        let buf = render_to_buffer_with_spec(&text_payload("wave"), Some(&spec), &registry, 50, 1);
        assert!(line_text(&buf, 0).contains("cannot display Text"));
    }

    #[test]
    fn natural_height_uses_shape_default_and_unknown_falls_back_to_one() {
        let registry = Registry::with_builtins();
        let renderer = AnimatedWaveRenderer;
        let body = text_payload("wave\ncrest").body;
        let opts = RenderOptions::default();
        let expected = registry
            .get(default_renderer_for(Shape::Text))
            .unwrap()
            .natural_height(&body, &inner_options(&opts), 20, &registry);

        assert_eq!(
            renderer.natural_height(&body, &opts, 20, &registry),
            expected
        );
        assert_eq!(
            renderer.natural_height(
                &body,
                &RenderOptions::default().with_extra("inner", "missing"),
                20,
                &registry,
            ),
            1
        );
    }

    #[test]
    fn wave_masks_future_columns_and_highlights_the_crest() {
        let theme = Theme::default();
        let buf = render_wave_frame(250, 1000, 8);

        assert_eq!(line_text(&buf, 0), "ABCDEF  ");
        assert_eq!(fg_at(&buf, 0, 'A'), theme.panel_title);
        assert_eq!(fg_at(&buf, 0, 'F'), Color::Reset);
    }

    #[test]
    fn renders_without_panic() {
        let payload = text_payload("wave");
        let spec = RenderSpec::Full {
            type_name: "animated_wave".into(),
            options: RenderOptions {
                style: Some("figlet".into()),
                duration_ms: Some(400),
                ..RenderOptions::default()
            }
            .with_extra("inner", "text_ascii"),
        };
        let registry = super::super::Registry::with_builtins();
        let _ = render_to_buffer_with_spec(&payload, Some(&spec), &registry, 40, 10);
    }
}
