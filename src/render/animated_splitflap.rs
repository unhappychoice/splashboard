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

const FLAP_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!?*#@%&/=+-<>:;^~.,'\"";

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "inner",
        type_hint: "renderer name (e.g. \"text_ascii\")",
        required: false,
        default: Some("shape default renderer"),
        description: "Renderer drawn as the resting frame once every cell has settled. Falls back to the shape's natural default.",
    },
    OptionSchema {
        name: "duration_ms",
        type_hint: "milliseconds (u64)",
        required: false,
        default: Some("1600"),
        description: "Total cycling duration. Earlier cells settle first; the last cell lands right as the window closes. Kept inside the 2s ANIMATION_WINDOW by default.",
    },
];

/// Split-flap reveal — like the departures board at a train station. Each cell in the widget's
/// rectangle cycles through an A-Z / 0-9 / punctuation set, landing on its final glyph at a
/// position-dependent settle time. Columns to the left land first so the text reads
/// left-to-right as it resolves.
///
/// Accepts every shape: the compatibility check is delegated to the resolved inner renderer.
/// Static post-window frame = the inner render, unchanged.
pub struct AnimatedSplitflapRenderer;

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

impl Renderer for AnimatedSplitflapRenderer {
    fn name(&self) -> &str {
        "animated_splitflap"
    }
    fn description(&self) -> &'static str {
        "Wrapper that flips each cell through random A-Z / 0-9 / punctuation glyphs before settling on the inner renderer's final character, like a train station departures board. Columns settle left-to-right so the text resolves in reading order. Accepts every shape."
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
        apply_flap(frame, area, elapsed, total, theme);
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

fn apply_flap(frame: &mut Frame, area: Rect, elapsed_ms: u32, total_ms: u32, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let buf = frame.buffer_mut();
    let settle_style = Style::default()
        .fg(theme.panel_title)
        .add_modifier(Modifier::BOLD);
    for y in 0..area.height {
        for x in 0..area.width {
            let settle_at = cell_settle_ms(x, y, area.width, area.height, total_ms);
            if elapsed_ms >= settle_at {
                continue;
            }
            let Some(cell) = buf.cell_mut((area.x + x, area.y + y)) else {
                continue;
            };
            // Blank cells stay blank during cycling; a row of flapping background
            // characters would dwarf the actual figlet shape and read as noise.
            if cell.symbol().trim().is_empty() {
                continue;
            }
            let glyph = flap_glyph(x, y, elapsed_ms);
            cell.set_symbol(&glyph.to_string());
            cell.set_style(settle_style);
        }
    }
}

/// Per-cell settle time. Columns to the left settle first (text reads left-to-right), with a
/// small per-row offset so adjacent rows don't flip in lockstep.
fn cell_settle_ms(x: u16, y: u16, width: u16, height: u16, total_ms: u32) -> u32 {
    let width = width.max(1) as u32;
    let height = height.max(1) as u32;
    let col_share = 0.80;
    let row_share = 0.15;
    let col = (x as u32 * total_ms) / width;
    let row = (y as u32 * total_ms) / height;
    let settle = (col as f32 * col_share + row as f32 * row_share) as u32 + total_ms / 12;
    settle.min(total_ms)
}

fn flap_glyph(x: u16, y: u16, elapsed_ms: u32) -> char {
    // Rotate the character set on a short clock so each cell appears to flip several times
    // per second. The (x, y) offset keeps neighbours on different glyphs on the same tick.
    let tick = (elapsed_ms / 45) as usize + (x as usize * 7 + y as usize * 13);
    FLAP_CHARS[tick % FLAP_CHARS.len()] as char
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
    fn left_cells_settle_before_right_cells() {
        let left = cell_settle_ms(0, 0, 20, 5, 1600);
        let right = cell_settle_ms(19, 0, 20, 5, 1600);
        assert!(left < right, "left={left} right={right}");
    }

    #[test]
    fn flap_glyph_is_printable_ascii() {
        let g = flap_glyph(3, 2, 900);
        assert!(g.is_ascii_graphic(), "got {g:?}");
    }

    #[test]
    fn renders_without_panic() {
        let payload = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData {
                value: "ready".into(),
            }),
        };
        let spec = RenderSpec::Full {
            type_name: "animated_splitflap".into(),
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
