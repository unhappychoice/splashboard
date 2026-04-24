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
        let inner_name = opts
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
        let inner_name = opts
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
        inner: None,
        effect: None,
        duration_ms: None,
        boot_lines: None,
        font_sequence: None,
        ..opts.clone()
    }
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
    use crate::render::{RenderSpec, test_utils::render_to_buffer_with_spec};

    #[test]
    fn inner_options_strips_wrapper_fields() {
        let opts = RenderOptions {
            inner: Some("text_ascii".into()),
            duration_ms: Some(1500),
            style: Some("figlet".into()),
            ..RenderOptions::default()
        };
        let inner = inner_options(&opts);
        assert!(inner.inner.is_none());
        assert!(inner.duration_ms.is_none());
        assert_eq!(inner.style.as_deref(), Some("figlet"));
    }

    #[test]
    fn renders_without_panic() {
        let payload = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData {
                value: "wave".into(),
            }),
        };
        let spec = RenderSpec::Full {
            type_name: "animated_wave".into(),
            options: RenderOptions {
                inner: Some("text_ascii".into()),
                style: Some("figlet".into()),
                duration_ms: Some(400),
                ..RenderOptions::default()
            },
        };
        let registry = super::super::Registry::with_builtins();
        let _ = render_to_buffer_with_spec(&payload, Some(&spec), &registry, 40, 10);
    }
}
