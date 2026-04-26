use std::sync::OnceLock;
use std::time::Instant;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
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
    #[serde(default)]
    pub boot_lines: Option<Vec<String>>,
}

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT, theme::TEXT_SECONDARY, theme::STATUS_OK];

const DEFAULT_DURATION_MS: u64 = 1800;

const BOOT_SHARE: f32 = 0.70;

const DEFAULT_LINES: &[&str] = &[
    "mounting /splash",
    "loading theme",
    "connecting terminal",
    "starting splashboard",
    "ready",
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "inner",
        type_hint: "renderer name (e.g. \"text_ascii\")",
        required: false,
        default: Some("shape default renderer"),
        description: "Renderer drawn once the boot sequence completes. Falls back to the shape's natural default.",
    },
    OptionSchema {
        name: "boot_lines",
        type_hint: "array of short strings",
        required: false,
        default: Some("a built-in `[ OK ] ...` list"),
        description: "Boot-log lines scrolled in one by one. Each line is prefixed with a green `[ OK ]` automatically; keep each string short enough to fit the widget width.",
    },
    OptionSchema {
        name: "duration_ms",
        type_hint: "milliseconds (u64)",
        required: false,
        default: Some("1800"),
        description: "Total duration. The boot log scrolls during the first ~70% and the inner render takes over for the remainder — stays inside the 2s ANIMATION_WINDOW.",
    },
];

/// Retro boot-log reveal. Scrolls a series of `[ OK ] ...` lines into the widget, then clears
/// and hands off to the inner renderer so the final frame looks like a static hero. Best used
/// on a tall hero cell (8+ rows) — shorter cells truncate the log.
///
/// Accepts every shape: the actual compatibility check is delegated to the resolved inner
/// renderer. The inner renderer is what shows once the boot log finishes.
pub struct AnimatedBootRenderer;

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

impl Renderer for AnimatedBootRenderer {
    fn name(&self) -> &str {
        "animated_boot"
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
            draw_inline_error(
                frame,
                area,
                &format!("unknown inner renderer: {inner_name}"),
            );
            return;
        };
        if !inner.accepts().contains(&shape) {
            draw_inline_error(
                frame,
                area,
                &format!("inner {inner_name} cannot display {shape:?}"),
            );
            return;
        }

        let total = opts.duration_ms.unwrap_or(DEFAULT_DURATION_MS).max(1) as u32;
        let elapsed = elapsed_since_start_ms();
        let boot_ms = (total as f32 * BOOT_SHARE) as u32;
        let lines = boot_lines(&specific);
        if elapsed >= boot_ms || lines.is_empty() {
            inner.render(frame, area, body, &inner_options(opts), theme, registry);
            return;
        }
        draw_boot_log(frame, area, &lines, elapsed, boot_ms, theme);
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

fn draw_boot_log(
    frame: &mut Frame,
    area: Rect,
    lines: &[String],
    elapsed_ms: u32,
    boot_ms: u32,
    theme: &Theme,
) {
    let progress = (elapsed_ms as f32 / boot_ms.max(1) as f32).clamp(0.0, 1.0);
    let visible_count = ((progress * lines.len() as f32).ceil() as usize).min(lines.len());
    if visible_count == 0 {
        return;
    }
    let ok_style = Style::default()
        .fg(theme.status_ok)
        .add_modifier(Modifier::BOLD);
    let body_style = Style::default().fg(theme.text_secondary);
    let rendered: Vec<Line> = lines
        .iter()
        .take(visible_count)
        .map(|l| {
            Line::from(vec![
                Span::styled("[ OK ] ", ok_style),
                Span::styled(l.clone(), body_style),
            ])
        })
        .collect();

    let content_height = rendered.len() as u16;
    let chunks = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(content_height.min(area.height)),
        Constraint::Fill(1),
    ])
    .split(area);
    let p = Paragraph::new(rendered).alignment(Alignment::Left);
    frame.render_widget(p, chunks[1]);
}

fn boot_lines(specific: &Options) -> Vec<String> {
    match specific.boot_lines.as_ref() {
        Some(list) => list.clone(),
        None => DEFAULT_LINES.iter().map(|s| s.to_string()).collect(),
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

fn draw_inline_error(frame: &mut Frame, area: Rect, msg: &str) {
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
    fn boot_lines_falls_back_to_default_when_none() {
        let specific = Options::default();
        assert_eq!(boot_lines(&specific).len(), DEFAULT_LINES.len());
    }

    #[test]
    fn boot_lines_respects_empty_override() {
        let specific = Options {
            boot_lines: Some(vec![]),
            ..Options::default()
        };
        assert!(boot_lines(&specific).is_empty());
    }

    #[test]
    fn inner_options_strips_boot_fields() {
        let opts = RenderOptions {
            duration_ms: Some(1500),
            style: Some("figlet".into()),
            ..RenderOptions::default()
        }
        .with_extra("inner", "text_ascii")
        .with_extra("effect", "fade_in")
        .with_extra("boot_lines", vec!["a".to_string()])
        .with_extra("font_sequence", vec!["small".to_string()]);
        let inner = inner_options(&opts);
        let parsed: Options = inner.parse_specific();
        assert!(parsed.inner.is_none());
        assert!(parsed.boot_lines.is_none());
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
                value: "hello".into(),
            }),
        };
        let spec = RenderSpec::Full {
            type_name: "animated_boot".into(),
            options: RenderOptions {
                style: Some("figlet".into()),
                duration_ms: Some(400),
                ..RenderOptions::default()
            }
            .with_extra("inner", "text_ascii")
            .with_extra("font", "standard"),
        };
        let registry = super::super::Registry::with_builtins();
        let _ = render_to_buffer_with_spec(&payload, Some(&spec), &registry, 60, 12);
    }
}
