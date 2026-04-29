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
    use crate::render::{RenderSpec, test_utils::render_to_buffer_with_spec};

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
    fn renders_without_panic() {
        let payload = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData { value: "hi".into() }),
        };
        let spec = RenderSpec::Full {
            type_name: "animated_scanlines".into(),
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
