use std::sync::OnceLock;
use std::time::Instant;

use ratatui::{Frame, layout::Rect, style::Color, widgets::Paragraph};
use tachyonfx::{
    Duration as FxDuration, Effect, EffectRenderer, EffectTimer, Interpolation, fx,
    pattern::SweepPattern,
};

use crate::options::OptionSchema;
use crate::payload::Body;
use crate::theme::Theme;

use super::{Registry, RenderOptions, Renderer, Shape, default_renderer_for, shape_of};

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "inner",
        type_hint: "renderer name (e.g. \"text_ascii\")",
        required: false,
        default: Some("shape default renderer"),
        description: "Renderer whose output the effect is applied over. Falls back to the shape's natural default when omitted.",
    },
    OptionSchema {
        name: "effect",
        type_hint: "\"fade_in\" | \"fade_out\" | \"dissolve\" | \"coalesce\" | \"sweep_in\" | \"slide_in\" | \"hsl_shift\" | \"glitch\"",
        required: false,
        default: Some("\"fade_in\""),
        description: "tachyonfx effect applied to the inner render. Unknown names fall back to `fade_in` rather than failing the widget.",
    },
    OptionSchema {
        name: "duration_ms",
        type_hint: "milliseconds (u64)",
        required: false,
        default: Some("800"),
        description: "Effect duration. Kept short so the motion completes well inside the 2s ANIMATION_WINDOW and the final frame sits static.",
    },
];

/// Shader-like post-process layered on top of another renderer. Draws the inner renderer
/// normally, then applies a tachyonfx `Effect` over the occupied area for the duration of the
/// runtime's ANIMATION_WINDOW. After the effect completes, the inner render is left untouched
/// so the final frozen frame matches what a static widget would produce.
///
/// Accepts every shape — the actual compatibility check is delegated to the resolved inner
/// renderer, which is picked via `inner = "..."` (falls back to the shape's default).
pub struct AnimatedPostfxRenderer;

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

const DEFAULT_DURATION_MS: u64 = 800;
const DEFAULT_EFFECT: &str = "fade_in";

impl Renderer for AnimatedPostfxRenderer {
    fn name(&self) -> &str {
        "animated_postfx"
    }
    fn accepts(&self) -> &[Shape] {
        ALL_SHAPES
    }
    fn animates(&self) -> bool {
        true
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
        render_postfx(frame, area, body, opts, theme, registry);
    }
}

fn render_postfx(
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
        render_inline_error(
            frame,
            area,
            &format!("unknown inner renderer: {inner_name}"),
        );
        return;
    };
    if !inner.accepts().contains(&shape) {
        render_inline_error(
            frame,
            area,
            &format!("inner {inner_name} cannot display {shape:?}"),
        );
        return;
    }
    inner.render(frame, area, body, &inner_options(opts), theme, registry);

    let duration_ms = opts.duration_ms.unwrap_or(DEFAULT_DURATION_MS) as u32;
    let elapsed_ms = elapsed_since_start_ms();
    // Once the effect window closes, leave the inner render as-is so the frozen splash looks
    // the same as it would without the effect.
    if elapsed_ms >= duration_ms {
        return;
    }
    let effect_name = opts.effect.as_deref().unwrap_or(DEFAULT_EFFECT);
    let mut effect = build_effect(effect_name, duration_ms);
    frame.render_effect(&mut effect, area, FxDuration::from_millis(elapsed_ms));
}

/// Strip the postfx-specific options before handing off to the inner renderer so it only
/// sees the fields it understands (style / pixel_size / align).
fn inner_options(opts: &RenderOptions) -> RenderOptions {
    RenderOptions {
        inner: None,
        effect: None,
        duration_ms: None,
        ..opts.clone()
    }
}

/// Map an effect name to a tachyonfx `Effect`. Unknown names fall back to `fade_in` so a typo
/// degrades gracefully instead of crashing the widget — the fetcher's data still reaches the
/// inner renderer either way.
///
/// All variants only modify foreground colours (or the cell character itself for
/// dissolve/coalesce). We intentionally avoid tachyonfx's `fade_from` / `sweep_in` / `slide_in`
/// because those paint the `faded_color` onto the cell background, which overlays the
/// terminal's default background with a solid black rectangle on any non-default-black
/// theme. `fade_from_fg` + `SweepPattern` produces the same directional reveal without that
/// artefact.
fn build_effect(name: &str, duration_ms: u32) -> Effect {
    let timer: EffectTimer = EffectTimer::from_ms(duration_ms, Interpolation::QuadOut);
    match name {
        "fade_in" => fx::fade_from_fg(Color::Black, timer),
        "fade_out" => fx::fade_to_fg(Color::Black, timer),
        "dissolve" => fx::dissolve(timer),
        "coalesce" => fx::coalesce(timer),
        "sweep_in" => {
            fx::fade_from_fg(Color::Black, timer).with_pattern(SweepPattern::left_to_right(10))
        }
        "sweep_in_right" => {
            fx::fade_from_fg(Color::Black, timer).with_pattern(SweepPattern::right_to_left(10))
        }
        "sweep_in_down" => {
            fx::fade_from_fg(Color::Black, timer).with_pattern(SweepPattern::up_to_down(10))
        }
        "sweep_in_up" => {
            fx::fade_from_fg(Color::Black, timer).with_pattern(SweepPattern::down_to_up(10))
        }
        "slide_in" => {
            fx::fade_from_fg(Color::Black, timer).with_pattern(SweepPattern::left_to_right(3))
        }
        "slide_in_right" => {
            fx::fade_from_fg(Color::Black, timer).with_pattern(SweepPattern::right_to_left(3))
        }
        "slide_in_down" => {
            fx::fade_from_fg(Color::Black, timer).with_pattern(SweepPattern::up_to_down(3))
        }
        "slide_in_up" => {
            fx::fade_from_fg(Color::Black, timer).with_pattern(SweepPattern::down_to_up(3))
        }
        "hsl_shift" => fx::hsl_shift(Some([180.0, 0.0, 0.0]), None, timer),
        _ => fx::fade_from_fg(Color::Black, timer),
    }
}

fn render_inline_error(frame: &mut Frame, area: Rect, msg: &str) {
    frame.render_widget(Paragraph::new(msg), area);
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
    fn known_effects_build_without_panic() {
        for name in [
            "fade_in",
            "fade_out",
            "dissolve",
            "coalesce",
            "sweep_in",
            "sweep_in_right",
            "sweep_in_down",
            "sweep_in_up",
            "slide_in",
            "slide_in_right",
            "slide_in_down",
            "slide_in_up",
            "hsl_shift",
        ] {
            let _ = build_effect(name, 800);
        }
    }

    #[test]
    fn unknown_effect_falls_back_silently() {
        // Typos shouldn't crash — we return a fade_in effect instead so the widget stays visible.
        let _ = build_effect("totally-not-a-real-effect", 800);
    }

    #[test]
    fn renders_text_without_panic() {
        let payload = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData {
                value: "hello".into(),
            }),
        };
        let spec = RenderSpec::Full {
            type_name: "animated_postfx".into(),
            options: RenderOptions {
                inner: Some("text_ascii".into()),
                effect: Some("dissolve".into()),
                duration_ms: Some(400),
                ..Default::default()
            },
        };
        let registry = super::super::Registry::with_builtins();
        // Draws through the inner renderer + applies the effect. The assertion is just "doesn't
        // panic" — the visual output depends on wall-clock elapsed time so a cell-level compare
        // would be flaky.
        let _buf = render_to_buffer_with_spec(&payload, Some(&spec), &registry, 20, 4);
    }

    #[test]
    fn unknown_inner_renders_inline_error() {
        let payload = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData { value: "hi".into() }),
        };
        let spec = RenderSpec::Full {
            type_name: "animated_postfx".into(),
            options: RenderOptions {
                inner: Some("does_not_exist".into()),
                ..Default::default()
            },
        };
        let registry = super::super::Registry::with_builtins();
        let buf = render_to_buffer_with_spec(&payload, Some(&spec), &registry, 40, 2);
        let joined: String = (0..2)
            .map(|y| crate::render::test_utils::line_text(&buf, y))
            .collect();
        assert!(
            joined.contains("unknown inner"),
            "expected inline error, got {joined:?}"
        );
    }

    #[test]
    fn inner_options_drops_postfx_fields() {
        let opts = RenderOptions {
            style: Some("figlet".into()),
            pixel_size: Some("quadrant".into()),
            align: Some("center".into()),
            inner: Some("text_ascii".into()),
            effect: Some("dissolve".into()),
            duration_ms: Some(1200),
            ..RenderOptions::default()
        };
        let inner = inner_options(&opts);
        assert_eq!(inner.style.as_deref(), Some("figlet"));
        assert_eq!(inner.pixel_size.as_deref(), Some("quadrant"));
        assert_eq!(inner.align.as_deref(), Some("center"));
        assert!(inner.inner.is_none());
        assert!(inner.effect.is_none());
        assert!(inner.duration_ms.is_none());
    }
}
