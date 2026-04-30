use std::sync::OnceLock;
use std::time::Instant;

use ratatui::{Frame, layout::Rect, style::Color};
use tachyonfx::{Duration as FxDuration, Effect, EffectRenderer, EffectTimer, Interpolation, fx};

use crate::options::OptionSchema;
use crate::payload::Body;
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    pub font_sequence: Option<Vec<String>>,
}

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT, theme::PANEL_TITLE];

const DEFAULT_DURATION_MS: u64 = 1800;

const DEFAULT_SEQUENCE: &[&str] = &["small", "banner", "ansi_shadow"];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "font_sequence",
        type_hint: "array of font names (e.g. [\"small\", \"banner\", \"ansi_shadow\"])",
        required: false,
        default: Some("[\"small\", \"banner\", \"ansi_shadow\"]"),
        description: "Figlet fonts to step through. Each phase renders the text in the next font and quickly crossfades to the one after. The final entry is what the static resting frame shows — pick the font you'd normally use for the hero.",
    },
    OptionSchema {
        name: "duration_ms",
        type_hint: "milliseconds (u64)",
        required: false,
        default: Some("1800"),
        description: "Total morph duration across every phase. Split evenly between phases; kept short so the motion lands inside the 2s ANIMATION_WINDOW.",
    },
    OptionSchema {
        name: "color",
        type_hint: "theme token name (e.g. \"panel_title\", \"text\")",
        required: false,
        default: Some("\"text\""),
        description: "Foreground colour token forwarded to text_ascii. Any `[theme]` key name is accepted.",
    },
    OptionSchema {
        name: "align",
        type_hint: "\"left\" | \"center\" | \"right\"",
        required: false,
        default: Some("\"left\""),
        description: "Horizontal alignment forwarded to text_ascii.",
    },
];

/// Animated hero that steps the text through a sequence of figlet fonts, letting each phase
/// crossfade into the next. The last font is the resting frame — once the window closes the
/// output matches what a plain `text_ascii { style = "figlet", font = <last> }` would produce.
///
/// Delegates rendering to `text_ascii` with a per-phase `font` override, so the same figlet
/// word-wrap + alignment logic is reused. Only `Text` is supported.
pub struct AnimatedFigletMorphRenderer;

impl Renderer for AnimatedFigletMorphRenderer {
    fn name(&self) -> &str {
        "animated_figlet_morph"
    }
    fn description(&self) -> &'static str {
        "Hero text rendered through a sequence of figlet fonts, crossfading from one to the next so the typography visibly reshapes itself before settling on the final font. Accepts `Text` only; the resting frame matches a static `text_ascii` render in the last font of the sequence."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Text]
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
        render_morph(frame, area, body, opts, theme, registry);
    }
    fn natural_height(
        &self,
        body: &Body,
        opts: &RenderOptions,
        max_width: u16,
        registry: &Registry,
    ) -> u16 {
        let Some(text_ascii) = registry.get("text_ascii") else {
            return 1;
        };
        // Height is whatever the tallest font in the sequence would need — otherwise the
        // widget's row would be sized for the resting font and the earlier phases would clip.
        sequence(opts)
            .iter()
            .map(|font| {
                let forwarded = text_ascii_opts(opts, font.as_str());
                text_ascii.natural_height(body, &forwarded, max_width, registry)
            })
            .max()
            .unwrap_or(1)
    }
}

fn render_morph(
    frame: &mut Frame,
    area: Rect,
    body: &Body,
    opts: &RenderOptions,
    theme: &Theme,
    registry: &Registry,
) {
    let Some(text_ascii) = registry.get("text_ascii") else {
        return;
    };
    let seq = sequence(opts);
    let total = opts.duration_ms.unwrap_or(DEFAULT_DURATION_MS).max(1) as u32;
    let elapsed = elapsed_since_start_ms();
    let (phase, phase_elapsed, phase_len) = phase_for(elapsed, total, seq.len());
    let font = seq[phase].as_str();
    let forwarded = text_ascii_opts(opts, font);
    text_ascii.render(frame, area, body, &forwarded, theme, registry);

    if elapsed >= total {
        return;
    }
    // Crossfade window inside each phase: fade_in at the start, fade_out at the end. The
    // resting phase (last) skips the fade_out so the final frame is a clean static render.
    let fade_ms = phase_len.min(260);
    if phase_elapsed < fade_ms {
        let mut effect = fx::fade_from_fg(
            Color::Black,
            EffectTimer::from_ms(fade_ms, Interpolation::QuadOut),
        );
        apply(frame, area, &mut effect, phase_elapsed);
    } else if phase + 1 < seq.len() && phase_elapsed + fade_ms >= phase_len {
        let mut effect = fx::fade_to_fg(
            Color::Black,
            EffectTimer::from_ms(fade_ms, Interpolation::QuadIn),
        );
        let into_fade = phase_elapsed + fade_ms - phase_len;
        apply(frame, area, &mut effect, into_fade);
    }
}

fn apply(frame: &mut Frame, area: Rect, effect: &mut Effect, elapsed_ms: u32) {
    frame.render_effect(effect, area, FxDuration::from_millis(elapsed_ms));
}

fn sequence(opts: &RenderOptions) -> Vec<String> {
    let specific: Options = opts.parse_specific();
    match specific.font_sequence {
        Some(list) if !list.is_empty() => list,
        _ => DEFAULT_SEQUENCE.iter().map(|s| (*s).to_string()).collect(),
    }
}

fn phase_for(elapsed: u32, total: u32, phase_count: usize) -> (usize, u32, u32) {
    let count = phase_count.max(1) as u32;
    let phase_len = total / count;
    let last_idx = (count - 1) as usize;
    if elapsed >= total || phase_len == 0 {
        return (last_idx, 0, phase_len.max(1));
    }
    let idx = (elapsed / phase_len).min(count - 1) as usize;
    let phase_elapsed = elapsed - (idx as u32) * phase_len;
    (idx, phase_elapsed, phase_len)
}

fn text_ascii_opts(opts: &RenderOptions, font: &str) -> RenderOptions {
    RenderOptions {
        style: Some("figlet".into()),
        color: opts.color.clone(),
        align: opts.align.clone(),
        ..RenderOptions::default()
    }
    .with_extra("font", font)
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
    use std::thread;
    use std::time::Duration;

    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::payload::{Payload, TextData};
    use crate::render::{
        RenderSpec,
        test_utils::{line_text, render_to_buffer_with_spec},
    };
    use crate::theme::Theme;

    fn text_body() -> Body {
        Body::Text(TextData { value: "hi".into() })
    }

    #[test]
    fn renderer_contract_exposes_description_theme_keys_and_options() {
        let renderer = AnimatedFigletMorphRenderer;
        assert!(renderer.description().contains("crossfading"));
        assert_eq!(
            renderer
                .color_keys()
                .iter()
                .map(|key| key.name)
                .collect::<Vec<_>>(),
            vec!["text", "panel_title"]
        );
        assert_eq!(
            renderer
                .option_schemas()
                .iter()
                .map(|schema| schema.name)
                .collect::<Vec<_>>(),
            vec!["font_sequence", "duration_ms", "color", "align"]
        );
    }

    #[test]
    fn phase_for_picks_last_on_overflow() {
        let (idx, _, _) = phase_for(5_000, 1_500, 3);
        assert_eq!(idx, 2);
    }

    #[test]
    fn phase_for_returns_start_of_first_phase() {
        let (idx, elapsed, len) = phase_for(0, 900, 3);
        assert_eq!(idx, 0);
        assert_eq!(elapsed, 0);
        assert_eq!(len, 300);
    }

    #[test]
    fn phase_for_advances_on_boundary() {
        let (idx, _, _) = phase_for(310, 900, 3);
        assert_eq!(idx, 1);
    }

    #[test]
    fn phase_for_guards_zero_length_phases() {
        assert_eq!(phase_for(0, 2, 3), (2, 0, 1));
    }

    #[test]
    fn sequence_falls_back_to_default_when_empty() {
        let opts = RenderOptions::default().with_extra("font_sequence", Vec::<String>::new());
        let want: Vec<String> = DEFAULT_SEQUENCE.iter().map(|s| (*s).to_string()).collect();
        assert_eq!(sequence(&opts), want);
    }

    #[test]
    fn sequence_uses_custom_font_sequence_when_present() {
        let opts = RenderOptions::default().with_extra("font_sequence", vec!["small", "doom"]);
        assert_eq!(
            sequence(&opts),
            vec!["small".to_string(), "doom".to_string()]
        );
    }

    #[test]
    fn natural_height_returns_one_without_text_ascii() {
        let renderer = AnimatedFigletMorphRenderer;
        assert_eq!(
            renderer.natural_height(
                &text_body(),
                &RenderOptions::default(),
                60,
                &Registry::default()
            ),
            1
        );
    }

    #[test]
    fn natural_height_uses_tallest_font_in_sequence() {
        let renderer = AnimatedFigletMorphRenderer;
        let registry = Registry::with_builtins();
        let opts = RenderOptions::default().with_extra("font_sequence", vec!["small", "banner"]);
        let text_ascii = registry.get("text_ascii").unwrap();
        let expected = ["small", "banner"]
            .into_iter()
            .map(|font| {
                text_ascii.natural_height(
                    &text_body(),
                    &text_ascii_opts(&opts, font),
                    60,
                    &registry,
                )
            })
            .max()
            .unwrap();
        assert_eq!(
            renderer.natural_height(&text_body(), &opts, 60, &registry),
            expected
        );
    }

    #[test]
    fn text_ascii_opts_forward_common_fields_and_font() {
        let opts = RenderOptions {
            align: Some("center".into()),
            color: Some("panel_title".into()),
            ..RenderOptions::default()
        };
        let forwarded = text_ascii_opts(&opts, "doom");
        assert_eq!(forwarded.style.as_deref(), Some("figlet"));
        assert_eq!(forwarded.align.as_deref(), Some("center"));
        assert_eq!(forwarded.color.as_deref(), Some("panel_title"));
        assert_eq!(forwarded.extra_str("font"), Some("doom"));
    }

    #[test]
    fn render_returns_early_when_registry_lacks_text_ascii() {
        let body = text_body();
        let mut terminal = Terminal::new(TestBackend::new(20, 4)).unwrap();
        terminal
            .draw(|frame| {
                render_morph(
                    frame,
                    frame.area(),
                    &body,
                    &RenderOptions::default(),
                    &Theme::default(),
                    &Registry::default(),
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        assert!(line_text(&buffer, 0).trim().is_empty());
    }

    #[test]
    fn render_skips_crossfade_after_duration_has_elapsed() {
        process_start();
        thread::sleep(Duration::from_millis(2));
        let payload = Payload {
            icon: None,
            status: None,
            format: None,
            body: text_body(),
        };
        let spec = RenderSpec::Full {
            type_name: "animated_figlet_morph".into(),
            options: RenderOptions {
                duration_ms: Some(1),
                ..RenderOptions::default()
            },
        };
        let registry = super::super::Registry::with_builtins();
        let buffer = render_to_buffer_with_spec(&payload, Some(&spec), &registry, 60, 14);
        let has_visible_text =
            (0..buffer.area.height).any(|y| !line_text(&buffer, y).trim().is_empty());
        assert!(has_visible_text);
    }
}
