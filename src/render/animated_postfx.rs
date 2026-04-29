use std::sync::OnceLock;
use std::time::Instant;

use ratatui::{Frame, layout::Rect, style::Color, widgets::Paragraph};
use tachyonfx::{
    Duration as FxDuration, Effect, EffectRenderer, EffectTimer, Interpolation, IntoEffect, fx,
    fx::{EvolveSymbolSet, Glitch},
    pattern::{CheckerboardPattern, DiagonalPattern, DissolvePattern, RadialPattern, SweepPattern},
};

use crate::options::OptionSchema;
use crate::payload::Body;
use crate::theme::Theme;

use super::{Registry, RenderOptions, Renderer, Shape, default_renderer_for, shape_of};

/// Wrapper Options intentionally lenient on unknown fields: the same `raw` blob carries
/// inner-renderer keys (e.g. `font` / `pixel_size` for text_ascii), which a strict parse
/// would reject and silently fall back to defaults, dropping `inner` and turning a figlet
/// hero into plain text.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(super) struct Options {
    #[serde(default)]
    pub inner: Option<String>,
    #[serde(default)]
    pub effect: Option<String>,
}

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
        type_hint: "\"fade_in\" | \"fade_out\" | \"dissolve\" | \"coalesce\" | \"sweep_in\" | \"sweep_in_right\" | \"sweep_in_down\" | \"sweep_in_up\" | \"slide_in\" | \"slide_in_right\" | \"slide_in_down\" | \"slide_in_up\" | \"hsl_shift\" | \"stagger_reveal\" | \"stagger_reveal_radial\" | \"matrix_rain\" | \"particle_burst\" | \"bounce_in\" | \"elastic_in\" | \"checkerboard_in\" | \"neon_flash\" | \"glitch_in\"",
        required: false,
        default: Some("\"fade_in\""),
        description: "tachyonfx effect applied to the inner render. `sweep_in*` / `slide_in*` use a directional reveal (default = left→right; `_right`/`_down`/`_up` suffixes invert the direction). `stagger_reveal` reveals cells along a diagonal (top-left → bottom-right); `stagger_reveal_radial` reveals outward from the centre. `matrix_rain` rains random glyphs that dissolve into the underlying render; `particle_burst` scatters radial particles that resolve into the figlet. `bounce_in` / `elastic_in` use bounce/spring timing curves for a playful arrival; `checkerboard_in` reveals tiles in a checker pattern; `neon_flash` pulses through a bright hue before settling. `glitch_in` scrambles a fraction of cells then settles into the clean render — a broken-signal / decode vibe. Unknown names fall back to `fade_in` rather than failing the widget.",
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
    fn description(&self) -> &'static str {
        "Wrapper that draws the inner renderer and overlays a tachyonfx pixel-level effect (fade, dissolve, sweep, glitch, matrix rain, neon flash, and more — pick via `effect`) for the duration of the animation window. Accepts every shape; the resting frame is the unmodified inner render."
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
    fn natural_height(
        &self,
        body: &Body,
        opts: &RenderOptions,
        max_width: u16,
        registry: &Registry,
    ) -> u16 {
        // The visible output is whatever `inner` produces; the postfx layer
        // doesn't add rows. Delegate so `height = "auto"` on a hero wrapped in
        // animated_postfx still picks up the inner's wrap height.
        let shape = shape_of(body);
        let specific: Options = opts.parse_specific();
        let inner_name = specific
            .inner
            .as_deref()
            .unwrap_or_else(|| default_renderer_for(shape));
        let Some(inner) = registry.get(inner_name) else {
            return 1;
        };
        let forwarded = inner_options(opts);
        inner.natural_height(body, &forwarded, max_width, registry)
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
    let specific: Options = opts.parse_specific();
    let inner_name = specific
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
    let effect_name = specific.effect.as_deref().unwrap_or(DEFAULT_EFFECT);
    let mut effect = build_effect(effect_name, duration_ms);
    frame.render_effect(&mut effect, area, FxDuration::from_millis(elapsed_ms));
}

/// Strip the postfx-specific options before handing off to the inner renderer so it only
/// sees the fields the inner renderer understands.
pub(super) fn inner_options(opts: &RenderOptions) -> RenderOptions {
    RenderOptions {
        duration_ms: None,
        ..opts.clone()
    }
    .without_extra("inner")
    .without_extra("effect")
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
        "stagger_reveal" => fx::fade_from_fg(Color::Black, timer)
            .with_pattern(DiagonalPattern::top_left_to_bottom_right()),
        "stagger_reveal_radial" => fx::fade_from_fg(Color::Black, timer)
            .with_pattern(RadialPattern::with_transition((0.5, 0.5), 6.0)),
        "matrix_rain" => fx::evolve_into(EvolveSymbolSet::BlocksVertical, timer)
            .with_pattern(DissolvePattern::new()),
        "particle_burst" => fx::evolve_into(EvolveSymbolSet::Shaded, timer)
            .with_pattern(RadialPattern::with_transition((0.5, 0.5), 8.0)),
        "bounce_in" => fx::fade_from_fg(
            Color::Black,
            EffectTimer::from_ms(duration_ms, Interpolation::BounceOut),
        )
        .with_pattern(SweepPattern::up_to_down(6)),
        "elastic_in" => fx::fade_from_fg(
            Color::Black,
            EffectTimer::from_ms(duration_ms, Interpolation::ElasticOut),
        )
        .with_pattern(RadialPattern::with_transition((0.5, 0.5), 4.0)),
        "checkerboard_in" => fx::fade_from_fg(Color::Black, timer)
            .with_pattern(CheckerboardPattern::with_cell_size(2)),
        // Bright hue rotation + lightness lift during the window — reads as the neon
        // warming up then settling back into the theme colour. The timer is
        // SineInOut so the pulse is symmetric.
        "neon_flash" => fx::hsl_shift(
            Some([120.0, 20.0, 35.0]),
            None,
            EffectTimer::from_ms(duration_ms, Interpolation::SineInOut),
        ),
        "glitch_in" => {
            // Glitch cells mutate a fraction of the area at random for short bursts. We cap
            // the whole run with `with_duration` so the effect stops exactly at the boundary;
            // afterwards `render_postfx` skips re-applying it and the clean inner render is
            // what the user sees.
            let glitch = Glitch::builder()
                .cell_glitch_ratio(0.30)
                .action_start_delay_ms(0..(duration_ms / 2).max(1))
                .action_ms((duration_ms / 10).max(40)..(duration_ms / 3).max(120))
                .build()
                .into_effect();
            fx::with_duration(FxDuration::from_millis(duration_ms), glitch)
        }
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
            "stagger_reveal",
            "stagger_reveal_radial",
            "matrix_rain",
            "particle_burst",
            "bounce_in",
            "elastic_in",
            "checkerboard_in",
            "neon_flash",
            "glitch_in",
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
                duration_ms: Some(400),
                ..Default::default()
            }
            .with_extra("inner", "text_ascii")
            .with_extra("effect", "dissolve"),
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
            options: RenderOptions::default().with_extra("inner", "does_not_exist"),
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
    fn postfx_options_survive_inner_only_extras() {
        // Regression: project_splash spec inlines `font = "ansi_shadow"` (a text_ascii field)
        // alongside `inner` and `effect`. With deny_unknown_fields and no catch-all, the
        // postfx Options parse failed silently, falling back to inner=None — which then
        // resolved to text_plain instead of the intended text_ascii figlet, producing a
        // single-line "splashboard" instead of the figlet hero.
        let opts = RenderOptions::default()
            .with_extra("inner", "text_ascii")
            .with_extra("effect", "particle_burst")
            .with_extra("font", "ansi_shadow");
        let parsed: Options = opts.parse_specific();
        assert_eq!(parsed.inner.as_deref(), Some("text_ascii"));
        assert_eq!(parsed.effect.as_deref(), Some("particle_burst"));
    }

    #[test]
    fn inner_options_drops_postfx_fields() {
        let opts = RenderOptions {
            style: Some("figlet".into()),
            align: Some("center".into()),
            duration_ms: Some(1200),
            ..RenderOptions::default()
        }
        .with_extra("pixel_size", "quadrant")
        .with_extra("inner", "text_ascii")
        .with_extra("effect", "dissolve");
        let inner = inner_options(&opts);
        assert_eq!(inner.style.as_deref(), Some("figlet"));
        assert_eq!(inner.align.as_deref(), Some("center"));
        // postfx-specific keys are stripped from the forwarded raw blob…
        let parsed_inner: Options = inner.parse_specific();
        assert!(parsed_inner.inner.is_none());
        assert!(parsed_inner.effect.is_none());
        // …but the inner renderer still sees its own keys (e.g. text_ascii's pixel_size).
        let parsed_text: crate::render::text_ascii::Options = inner.parse_specific();
        assert_eq!(parsed_text.pixel_size.as_deref(), Some("quadrant"));
        assert!(inner.duration_ms.is_none());
    }
}
